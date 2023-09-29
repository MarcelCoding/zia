use std::future::Future;
use std::mem;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::anyhow;
use async_http_proxy::{http_connect_tokio, http_connect_tokio_with_basic_auth};
use fastwebsockets::{Frame, OpCode, Payload, WebSocketRead, WebSocketWrite};
use hyper::header::{
  CONNECTION, HOST, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_VERSION, UPGRADE, USER_AGENT,
};
use hyper::upgrade::Upgraded;
use hyper::{Body, Request};
use once_cell::sync::Lazy;
use tokio::io::{split, ReadHalf, WriteHalf};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::{Mutex as TokioMutex, Mutex};
use tokio_rustls::rustls::{ClientConfig, OwnedTrustAnchor, RootCertStore, ServerName};
use tokio_rustls::TlsConnector;
use tracing::info;
use url::Url;

static TLS_CONNECTOR: Lazy<TlsConnector> = Lazy::new(|| {
  let mut store = RootCertStore::empty();
  store.add_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.iter().map(|ta| {
    OwnedTrustAnchor::from_subject_spki_name_constraints(ta.subject, ta.spki, ta.name_constraints)
  }));

  let config = ClientConfig::builder()
    .with_safe_defaults()
    .with_root_certificates(store)
    .with_no_client_auth();

  TlsConnector::from(Arc::new(config))
});

pub(crate) struct Connection {
  finished: Arc<Mutex<bool>>,
  buf: Arc<Mutex<Box<[u8; MAX_DATAGRAM_SIZE]>>>,
  write: Arc<Mutex<WebSocketWrite<WriteHalf<Upgraded>>>>,
}

impl Connection {
  pub(crate) async fn new(
    upstream: &Url,
    proxy: &Option<Url>,
  ) -> anyhow::Result<(Self, WebSocketRead<ReadHalf<Upgraded>>)> {
    let upstream_host = upstream
      .host_str()
      .ok_or_else(|| anyhow!("Upstream url is missing host"))?;
    let upstream_port = upstream
      .port_or_known_default()
      .ok_or_else(|| anyhow!("Upstream url is missing port"))?;

    let stream = match proxy {
      None => {
        let stream = TcpStream::connect((upstream_host, upstream_port)).await?;
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
              upstream_host,
              upstream_port,
              proxy.username(),
              password,
            )
            .await?
          }
          None => http_connect_tokio(&mut stream, upstream_host, upstream_port).await?,
        };

        info!("Proxy handshake");

        stream
      }
    };

    let req = Request::get(upstream.to_string())
      .header(HOST, format!("{}:{}", upstream_host, upstream_port))
      .header(UPGRADE, "websocket")
      .header(CONNECTION, "upgrade")
      .header(SEC_WEBSOCKET_KEY, fastwebsockets::handshake::generate_key())
      .header(SEC_WEBSOCKET_VERSION, "13")
      .header(USER_AGENT, "zia")
      .body(Body::empty())?;

    let (ws, _) = if upstream.scheme() == "wss" {
      let domain = ServerName::try_from(upstream_host)?;
      let stream = TLS_CONNECTOR.connect(domain, stream).await?;
      info!("Upgraded to tls");

      fastwebsockets::handshake::client(&SpawnExecutor, req, stream).await?
    } else {
      fastwebsockets::handshake::client(&SpawnExecutor, req, stream).await?
    };

    info!("Finished websocket handshake");

    let (read, write) = ws.split(|upgraded| split(upgraded));

    Ok((
      Self {
        finished: Arc::new(Mutex::new(true)),
        buf: Arc::new(Mutex::new(datagram_buffer())),
        write: Arc::new(Mutex::new(write)),
      },
      read,
    ))
  }
}

pub(crate) struct WritePool {
  pub(crate) connections: Vec<Connection>,
  pub(crate) next: usize,
}

impl WritePool {
  pub(crate) async fn write(
    &mut self,
    socket: &UdpSocket,
    old_addr: Arc<TokioMutex<Option<SocketAddr>>>,
  ) -> anyhow::Result<()> {
    loop {
      let connection = self
        .connections
        .get(self.next % self.connections.len())
        .unwrap();

      self.next += 1;

      let mut finished = connection.finished.lock().await;
      if *finished {
        *finished = false;

        let mut buf = connection.buf.lock().await;
        let (read, addr) = socket.recv_from(&mut buf[..]).await.unwrap();
        tokio::spawn(async move {
          *(old_addr.lock().await) = Some(addr);
        });

        let finished = connection.finished.clone();
        let buf = connection.buf.clone();
        let write = connection.write.clone();

        tokio::spawn(async move {
          let mut buf = buf.lock().await;

          write
            .lock()
            .await
            .write_frame(Frame::new(
              true,
              OpCode::Binary,
              None,
              Payload::BorrowedMut(&mut buf[..read]),
            ))
            .await
            .unwrap();

          *(finished.lock().await) = true;
        });
        return Ok(());
      }
    }
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

const MAX_DATAGRAM_SIZE: usize = u16::MAX as usize - mem::size_of::<u16>();

/// Creates and returns a buffer on the heap with enough space to contain any possible
/// UDP datagram.
///
/// This is put on the heap and in a separate function to avoid the 64k buffer from ending
/// up on the stack and blowing up the size of the futures using it.
#[inline(never)]
fn datagram_buffer() -> Box<[u8; MAX_DATAGRAM_SIZE]> {
  Box::new([0u8; MAX_DATAGRAM_SIZE])
}
