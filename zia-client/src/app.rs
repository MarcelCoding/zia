use std::future::Future;
use std::sync::Arc;

use anyhow::anyhow;
use async_http_proxy::{http_connect_tokio, http_connect_tokio_with_basic_auth};
use hyper::header::{
  CONNECTION, HOST, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_VERSION, UPGRADE, USER_AGENT,
};
use hyper::upgrade::Upgraded;
use hyper::{Body, Request};
use once_cell::sync::Lazy;
use tokio::io::split;
use tokio::net::TcpStream;
use tokio_rustls::rustls::{ClientConfig, OwnedTrustAnchor, RootCertStore, ServerName};
use tokio_rustls::TlsConnector;
use tracing::info;
use url::Url;

use zia_common::ws::{Role, WebSocket};
use zia_common::{ReadConnection, WriteConnection, MAX_DATAGRAM_SIZE};

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

pub(crate) async fn open_connection(
  upstream: &Url,
  proxy: &Option<Url>,
) -> anyhow::Result<(ReadConnection<Upgraded>, WriteConnection<Upgraded>)> {
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

  let (read, write) = split(ws.into_inner());

  let read = WebSocket::new(read, MAX_DATAGRAM_SIZE, Role::Client);
  let write = WebSocket::new(write, MAX_DATAGRAM_SIZE, Role::Client);

  Ok((ReadConnection::new(read), WriteConnection::new(write)))
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
