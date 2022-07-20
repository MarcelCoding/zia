use std::io::{Error, IoSlice};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use anyhow::anyhow;
use async_http_proxy::{http_connect_tokio, http_connect_tokio_with_basic_auth};
use once_cell::sync::Lazy;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::{ClientConfig, OwnedTrustAnchor, RootCertStore, ServerName};
use tokio_rustls::TlsConnector;
use url::Url;

static TLS_CONNECTOR: Lazy<TlsConnector> = Lazy::new(|| {
  let mut store = RootCertStore::empty();
  store.add_server_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.0.iter().map(|ta| {
    OwnedTrustAnchor::from_subject_spki_name_constraints(ta.subject, ta.spki, ta.name_constraints)
  }));

  let config = ClientConfig::builder()
    .with_safe_defaults()
    .with_root_certificates(store)
    .with_no_client_auth();

  TlsConnector::from(Arc::new(config))
});

pub enum Stream<IO> {
  Plain(IO),
  Tls(TlsStream<IO>),
}

impl Stream<TcpStream> {
  pub async fn connect(upstream: &Url, proxy: &Option<Url>) -> anyhow::Result<Self> {
    let upstream_host = upstream
      .host_str()
      .ok_or_else(|| anyhow!("Upstream url is missing host"))?;
    let upstream_port = upstream
      .port_or_known_default()
      .ok_or_else(|| anyhow!("Upstream url is missing port"))?;

    let mut stream = match proxy {
      None => {
        let stream = TcpStream::connect((upstream_host, upstream_port)).await?;
        stream.set_nodelay(true)?;

        Self::Plain(stream)
      }
      Some(proxy) => {
        let proxy_host = proxy
          .host_str()
          .ok_or_else(|| anyhow!("Proxy url is missing host"))?;
        let proxy_port = proxy
          .port_or_known_default()
          .ok_or_else(|| anyhow!("Proxy url is missing port"))?;

        let stream = TcpStream::connect((proxy_host, proxy_port)).await?;
        stream.set_nodelay(true)?;

        let mut stream = Self::Plain(stream);

        if proxy.scheme() == "https" {
          stream = stream.upgrade_to_tls(proxy_host).await?;
        };

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

        stream
      }
    };

    if upstream.scheme() == "wss" || upstream.scheme() == "tcps" {
      stream = stream.upgrade_to_tls(upstream_host).await?;
    }

    Ok(stream)
  }

  pub async fn upgrade_to_tls(self, host: &str) -> anyhow::Result<Self> {
    let domain = ServerName::try_from(host)?;

    let stream = match self {
      Self::Plain(stream) => Self::Tls(TLS_CONNECTOR.connect(domain, stream).await?),
      Self::Tls(_) => unimplemented!(), //Self::Tls(TLS_CONNECTOR.connect(domain, stream).await?),
    };

    Ok(stream)
  }
}

impl<IO: AsyncRead + AsyncWrite + Unpin> AsyncRead for Stream<IO> {
  fn poll_read(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &mut ReadBuf<'_>,
  ) -> Poll<std::io::Result<()>> {
    match &mut *self {
      Self::Plain(stream) => Pin::new(stream).poll_read(cx, buf),
      Self::Tls(stream) => Pin::new(stream).poll_read(cx, buf),
    }
  }
}

impl<IO: AsyncRead + AsyncWrite + Unpin> AsyncWrite for Stream<IO> {
  fn poll_write(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &[u8],
  ) -> Poll<Result<usize, Error>> {
    match &mut *self {
      Self::Plain(stream) => Pin::new(stream).poll_write(cx, buf),
      Self::Tls(stream) => Pin::new(stream).poll_write(cx, buf),
    }
  }

  fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
    match &mut *self {
      Self::Plain(stream) => Pin::new(stream).poll_flush(cx),
      Self::Tls(stream) => Pin::new(stream).poll_flush(cx),
    }
  }

  fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
    match &mut *self {
      Self::Plain(stream) => Pin::new(stream).poll_shutdown(cx),
      Self::Tls(stream) => Pin::new(stream).poll_shutdown(cx),
    }
  }

  fn poll_write_vectored(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    bufs: &[IoSlice<'_>],
  ) -> Poll<Result<usize, Error>> {
    match &mut *self {
      Self::Plain(stream) => Pin::new(stream).poll_write_vectored(cx, bufs),
      Self::Tls(stream) => Pin::new(stream).poll_write_vectored(cx, bufs),
    }
  }

  fn is_write_vectored(&self) -> bool {
    match self {
      Self::Plain(stream) => stream.is_write_vectored(),
      Self::Tls(stream) => stream.is_write_vectored(),
    }
  }
}
