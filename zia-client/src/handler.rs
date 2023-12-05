use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use async_http_proxy::{http_connect_tokio, http_connect_tokio_with_basic_auth};
use hyper::upgrade::Upgraded;
use hyper::Uri;
use hyper_util::rt::TokioIo;
use once_cell::sync::Lazy;
use rustls_pki_types::ServerName;
use tokio::io::{BufStream, WriteHalf};
use tokio::net::TcpStream;
use tokio::net::UdpSocket;
use tokio::select;
use tokio::sync::{mpsc, RwLock};
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;
use tracing::{error, info};
use url::Url;
use wsocket::handshake;

use zia_common::{ReadConnection, WriteConnection, MAX_DATAGRAM_SIZE};
use zia_common::{ReadPool, WritePool};

static TLS_CONFIG: Lazy<Arc<ClientConfig>> = Lazy::new(|| {
  let mut store = RootCertStore::empty();
  store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

  let config = ClientConfig::builder()
    .with_root_certificates(store)
    .with_no_client_auth();

  Arc::new(config)
});

struct Upstream {
  uri: Uri,
  host: String,
  port: u16,
  tls: bool,
  proxy: Option<Url>,
  ws_masking: bool,
}

pub(crate) struct Handler {
  upstream: Arc<Upstream>,
  write_pool: WritePool<WriteHalf<TokioIo<Upgraded>>>,
  read_pool: ReadPool,
}

impl Handler {
  pub(crate) fn new(
    socket: UdpSocket,
    upstream: Url,
    proxy: Option<Url>,
    ws_masking: bool,
  ) -> anyhow::Result<Self> {
    let socket = Arc::new(socket);
    let addr = Arc::new(RwLock::new(None));

    let write_pool = WritePool::new(socket.clone(), addr.clone());
    let read_pool = ReadPool::new(socket, addr);

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

        if let Err(err) = self.open_connection(dead_conn_tx.clone()).await {
          error!("Unable to open connection, trying again in 1s: {err:?}");
          missing += 1;
          tokio::time::sleep(Duration::from_secs(1)).await;
        }
      }
    };

    select! {
      _ = conn_backpressure => info!("Conn Backpressure finished?"),
      _ = self.write_pool.join() => info!("Write pool finished"),
      result = self.read_pool.join() => {
        info!("Read pool finished");
        result?;
      },
    }
    Ok(())
  }

  async fn open_connection(&self, dead_conn: mpsc::Sender<()>) -> anyhow::Result<()> {
    let stream = match &self.upstream.proxy {
      None => {
        let stream = TcpStream::connect((self.upstream.host.as_str(), self.upstream.port)).await?;
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
              &self.upstream.host,
              self.upstream.port,
              proxy.username(),
              password,
            )
            .await?
          }
          None => http_connect_tokio(&mut stream, &self.upstream.host, self.upstream.port).await?,
        };

        info!("Proxy handshake");

        stream
      }
    };

    let stream = BufStream::new(stream);

    let (ws, _) = if self.upstream.tls {
      let domain = ServerName::try_from(self.upstream.host.as_str())?.to_owned();
      let stream = TlsConnector::from(TLS_CONFIG.clone())
        .connect(domain, stream)
        .await?;
      info!("Upgraded to tls");

      handshake(
        stream,
        &self.upstream.uri,
        &self.upstream.host,
        self.upstream.port,
        "zia",
        MAX_DATAGRAM_SIZE,
        self.upstream.ws_masking,
      )
      .await?
    } else {
      handshake(
        stream,
        &self.upstream.uri,
        &self.upstream.host,
        self.upstream.port,
        "zia",
        MAX_DATAGRAM_SIZE,
        self.upstream.ws_masking,
      )
      .await?
    };

    info!("Finished websocket handshake");

    let (read, write) = ws.split();

    self
      .read_pool
      .push(ReadConnection::new(read), Some(dead_conn))
      .await;
    self.write_pool.push(WriteConnection::new(write)).await;

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
