use std::io::{Error, IoSlice};
use std::pin::Pin;
use std::sync::{Arc, LazyLock};
use std::task::{Context, Poll};

use pin_project_lite::pin_project;
use rustls_pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

static TLS_CONFIG: LazyLock<Arc<ClientConfig>> = LazyLock::new(|| {
  let mut store = RootCertStore::empty();
  store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

  let config = ClientConfig::builder()
    .with_root_certificates(store)
    .with_no_client_auth();

  Arc::new(config)
});

pub(crate) async fn tls<IO: AsyncRead + AsyncWrite + Unpin>(
  io: IO,
  domain: &str,
) -> anyhow::Result<TlsStream<IO>> {
  let domain = ServerName::try_from(domain)?.to_owned();

  Ok(
    TlsConnector::from(TLS_CONFIG.clone())
      .connect(domain, io)
      .await?,
  )
}

pin_project! {
  #[project = MaybeTlsStreamProj]
  pub(crate) enum MaybeTlsStream<IO> {
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
