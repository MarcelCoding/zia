use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use async_http_proxy::{http_connect_tokio, http_connect_tokio_with_basic_auth};
use hyper::upgrade::Upgraded;
use hyper::Uri;
use hyper_util::rt::TokioIo;
use tokio::io::{split, BufStream, ReadHalf, WriteHalf};
use tokio::net::{TcpStream, UdpSocket};
use tokio::select;
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info};
use url::Url;
use wsocket::handshake;

use zia_common::{
  PoolEntry, ReadConnection, ReadPool, ReadTcpConnection, ReadWsConnection, WriteConnection,
  WritePool, WriteTcpConnection, WriteWsConnection, MAX_DATAGRAM_SIZE,
};

use crate::tls::{tls, MaybeTlsStream};

pub struct Upstream {
  uri: Uri,
  host: String,
  port: u16,
  tls: bool,
  proxy: Option<Url>,
  ws_masking: bool,
}

pub(crate) struct Handler<W: ConnectionManager> {
  upstream: Arc<Upstream>,
  write_pool: Arc<WritePool<W::WriteConnection>>,
  read_pool: Arc<ReadPool>,
}

impl<C: ConnectionManager> Handler<C> {
  pub(crate) fn new(
    socket: UdpSocket,
    upstream: Url,
    proxy: Option<Url>,
    ws_masking: bool,
  ) -> anyhow::Result<Self> {
    let socket = Arc::new(socket);
    let addr = Arc::new(RwLock::new(None));

    let write_pool = Arc::new(WritePool::new(socket.clone(), addr.clone()));
    let read_pool = Arc::new(ReadPool::new(socket, addr));

    let upstream_host = upstream
      .host_str()
      .ok_or_else(|| anyhow!("Upstream url is missing host"))?
      .to_string();
    let upstream_port = upstream
      .port_or_known_default()
      .ok_or_else(|| anyhow!("Upstream url is missing port"))?;

    Ok(Self {
      upstream: Arc::new(Upstream {
        uri: upstream.to_string().parse().unwrap(),
        host: upstream_host,
        port: upstream_port,
        tls: upstream.scheme() == "wss",
        proxy,
        ws_masking,
      }),
      write_pool,
      read_pool,
    })
  }

  pub(crate) async fn run(&self, conn_count: usize) -> anyhow::Result<()> {
    let (dead_conn_tx, mut dead_conn_rx) = mpsc::channel(128);

    let conn_backpressure = async {
      let mut missing = conn_count;
      while missing > 0 || dead_conn_rx.recv().await.is_some() {
        missing = missing.saturating_sub(1);

        match C::open_connection(&self.upstream).await {
          Ok((read, write)) => {
            self.read_pool.push(read).await;
            self.write_pool.push(write).await;
          }
          Err(err) => {
            error!("Unable to open connection, trying again in 1s: {err:?}");
            missing += 1;
            tokio::time::sleep(Duration::from_secs(1)).await;
          }
        };
      }
    };

    let write_pool = self.write_pool.clone();
    let read_pool = self.read_pool.clone();

    let write = tokio::spawn(async move { write_pool.join().await });

    select! {
      _ = conn_backpressure => info!("Conn Backpressure finished?"),
      result = write => {
        info!("Write pool finished");
        result?;
      },
      result = read_pool.join(Some(&dead_conn_tx)) => {
        info!("Read pool finished");
        result?;
      },
    }
    Ok(())
  }
}

// Tie hyper's executor to tokio runtime
struct SpawnExecutor;

impl<Fut> hyper::rt::Executor<Fut> for SpawnExecutor
where
  Fut: Future + Send + 'static,
  Fut::Output: Send + 'static,
{
  fn execute(&self, fut: Fut) {
    tokio::task::spawn(fut);
  }
}

#[async_trait::async_trait]
pub trait ConnectionManager {
  type ReadConnection: ReadConnection + Send + 'static;
  type WriteConnection: WriteConnection + PoolEntry + Send + 'static;

  async fn open_connection(
    upstream: &Upstream,
  ) -> anyhow::Result<(Self::ReadConnection, Self::WriteConnection)>;
}

pub struct WsConnectionManager;

#[async_trait::async_trait]
impl ConnectionManager for WsConnectionManager {
  type ReadConnection = ReadWsConnection<ReadHalf<TokioIo<Upgraded>>>;
  type WriteConnection = WriteWsConnection<WriteHalf<TokioIo<Upgraded>>>;

  async fn open_connection(
    upstream: &Upstream,
  ) -> anyhow::Result<(Self::ReadConnection, Self::WriteConnection)> {
    let stream = match &upstream.proxy {
      None => {
        let stream = TcpStream::connect((upstream.host.as_str(), upstream.port)).await?;
        stream.set_nodelay(true)?;

        info!("Connected to tcp");

        stream
      }
      Some(proxy) => {
        assert_eq!(proxy.scheme(), "http");

        let proxy_host = proxy
          .host_str()
          .ok_or_else(|| anyhow!("Proxy url is missing host"))?;
        let proxy_port = proxy
          .port_or_known_default()
          .ok_or_else(|| anyhow!("Proxy url is missing port"))?;

        let mut stream = TcpStream::connect((proxy_host, proxy_port)).await?;
        stream.set_nodelay(true)?;

        info!("Connected to tcp");

        match proxy.password() {
          Some(password) => {
            http_connect_tokio_with_basic_auth(
              &mut stream,
              &upstream.host,
              upstream.port,
              proxy.username(),
              password,
            )
            .await?
          }
          None => http_connect_tokio(&mut stream, &upstream.host, upstream.port).await?,
        };

        info!("Proxy handshake");

        stream
      }
    };

    let stream = BufStream::new(stream);

    let (ws, _) = if upstream.tls {
      let stream = tls(stream, &upstream.host).await?;

      info!("Upgraded to tls");

      handshake(
        stream,
        &upstream.uri,
        &upstream.host,
        upstream.port,
        "zia",
        MAX_DATAGRAM_SIZE,
        upstream.ws_masking,
      )
      .await?
    } else {
      handshake(
        stream,
        &upstream.uri,
        &upstream.host,
        upstream.port,
        "zia",
        MAX_DATAGRAM_SIZE,
        upstream.ws_masking,
      )
      .await?
    };

    info!("Finished websocket handshake");

    let (read, write) = ws.split();

    Ok((ReadWsConnection::new(read), WriteWsConnection::new(write)))
  }
}

pub(crate) struct TcpConnectionManager;

#[async_trait::async_trait]
impl ConnectionManager for TcpConnectionManager {
  type ReadConnection = ReadTcpConnection<ReadHalf<MaybeTlsStream<BufStream<TcpStream>>>>;
  type WriteConnection = WriteTcpConnection<WriteHalf<MaybeTlsStream<BufStream<TcpStream>>>>;

  async fn open_connection(
    upstream: &Upstream,
  ) -> anyhow::Result<(Self::ReadConnection, Self::WriteConnection)> {
    let stream = match &upstream.proxy {
      None => {
        let stream = TcpStream::connect((upstream.host.as_str(), upstream.port)).await?;
        stream.set_nodelay(true)?;

        info!("Connected to tcp");

        stream
      }
      Some(proxy) => {
        assert_eq!(proxy.scheme(), "http");

        let proxy_host = proxy
          .host_str()
          .ok_or_else(|| anyhow!("Proxy url is missing host"))?;
        let proxy_port = proxy
          .port_or_known_default()
          .ok_or_else(|| anyhow!("Proxy url is missing port"))?;

        let mut stream = TcpStream::connect((proxy_host, proxy_port)).await?;
        stream.set_nodelay(true)?;

        info!("Connected to tcp");

        match proxy.password() {
          Some(password) => {
            http_connect_tokio_with_basic_auth(
              &mut stream,
              &upstream.host,
              upstream.port,
              proxy.username(),
              password,
            )
            .await?
          }
          None => http_connect_tokio(&mut stream, &upstream.host, upstream.port).await?,
        };

        info!("Proxy handshake");

        stream
      }
    };

    let stream = BufStream::new(stream);

    let stream = if upstream.tls {
      let stream = MaybeTlsStream::Tls {
        io: tls(stream, &upstream.host).await?,
      };
      info!("Upgraded to tls");
      stream
    } else {
      MaybeTlsStream::Raw { io: stream }
    };

    info!("Finished websocket handshake");

    let (read, write) = split(stream);

    Ok((ReadTcpConnection::new(read), WriteTcpConnection::new(write)))
  }
}
