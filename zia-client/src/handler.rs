use std::future::Future;
use std::io::{Error, IoSlice};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::anyhow;
use async_http_proxy::{http_connect_tokio, http_connect_tokio_with_basic_auth};
use hyper::upgrade::Upgraded;
use hyper::Uri;
use hyper_util::rt::TokioIo;
use once_cell::sync::Lazy;
use pin_project_lite::pin_project;
use rustls_pki_types::ServerName;
use tokio::io::{split, AsyncRead, AsyncWrite, BufStream, ReadBuf, ReadHalf, WriteHalf};
use tokio::net::{TcpStream, UdpSocket};
use tokio::select;
use tokio::sync::{mpsc, RwLock};
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;
use tracing::{error, info};
use url::Url;
use wsocket::handshake;

use zia_common::{
  PoolEntry, ReadConnection, ReadPool, ReadTcpConnection, ReadWsConnection, WriteConnection,
  WritePool, WriteTcpConnection, WriteWsConnection, MAX_DATAGRAM_SIZE,
};

static TLS_CONFIG: Lazy<Arc<ClientConfig>> = Lazy::new(|| {
  let mut store = RootCertStore::empty();
  store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

  let config = ClientConfig::builder()
    .with_root_certificates(store)
    .with_no_client_auth();

  Arc::new(config)
});

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
      let domain = ServerName::try_from(upstream.host.as_str())?.to_owned();
      let stream = TlsConnector::from(TLS_CONFIG.clone())
        .connect(domain, stream)
        .await?;
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

pub struct TcpConnectionManager;

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
      let domain = ServerName::try_from(upstream.host.as_str())?.to_owned();
      let stream = TlsConnector::from(TLS_CONFIG.clone())
        .connect(domain, stream)
        .await?;
      info!("Upgraded to tls");

      MaybeTlsStream::Tls { io: stream }
    } else {
      MaybeTlsStream::Raw { io: stream }
    };

    info!("Finished websocket handshake");

    let (read, write) = split(stream);

    Ok((ReadTcpConnection::new(read), WriteTcpConnection::new(write)))
  }
}
pin_project! {
  #[project = MaybeTlsStreamProj]
  pub enum MaybeTlsStream<IO> {
    Tls { #[pin] io: TlsStream<IO> },
    Raw { #[pin] io: IO },
  }
}

impl<IO: AsyncRead + AsyncWrite + Unpin> AsyncWrite for MaybeTlsStream<IO> {
  fn poll_write(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &[u8],
  ) -> Poll<Result<usize, Error>> {
    match self.project() {
      MaybeTlsStreamProj::Tls { io } => io.poll_write(cx, buf),
      MaybeTlsStreamProj::Raw { io } => io.poll_write(cx, buf),
    }
  }

  fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
    match self.project() {
      MaybeTlsStreamProj::Tls { io } => io.poll_flush(cx),
      MaybeTlsStreamProj::Raw { io } => io.poll_flush(cx),
    }
  }

  fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
    match self.project() {
      MaybeTlsStreamProj::Tls { io } => io.poll_shutdown(cx),
      MaybeTlsStreamProj::Raw { io } => io.poll_shutdown(cx),
    }
  }

  fn poll_write_vectored(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    bufs: &[IoSlice<'_>],
  ) -> Poll<Result<usize, Error>> {
    match self.project() {
      MaybeTlsStreamProj::Tls { io } => io.poll_write_vectored(cx, bufs),
      MaybeTlsStreamProj::Raw { io } => io.poll_write_vectored(cx, bufs),
    }
  }

  fn is_write_vectored(&self) -> bool {
    match self {
      MaybeTlsStream::Tls { io } => io.is_write_vectored(),
      MaybeTlsStream::Raw { io } => io.is_write_vectored(),
    }
  }
}

impl<IO: AsyncRead + AsyncWrite + Unpin> AsyncRead for MaybeTlsStream<IO> {
  fn poll_read(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &mut ReadBuf<'_>,
  ) -> Poll<std::io::Result<()>> {
    match self.project() {
      MaybeTlsStreamProj::Tls { io } => io.poll_read(cx, buf),
      MaybeTlsStreamProj::Raw { io } => io.poll_read(cx, buf),
    }
  }
}
