use std::net::SocketAddr;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::net::UdpSocket;
use url::Url;

mod tcp;
mod ws;

pub(crate) async fn transmit(
  socket: UdpSocket,
  upstream: &Url,
  proxy: &Option<Url>,
) -> anyhow::Result<()> {
  match upstream.scheme().to_lowercase().as_str() {
    "tcp" | "tcps" => tcp::transmit(socket, upstream, proxy).await,
    "ws" | "wss" => ws::transmit(socket, upstream, proxy).await,
    _ => Err(anyhow!("Unsupported upstream scheme {}", upstream.scheme())),
  }
}

#[async_trait]
pub(crate) trait Upstream {
  type Sink: UpstreamSink;
  type Stream: UpstreamStream;

  const BUF_SIZE: usize;
  const BUF_SKIP: usize;

  fn buffer() -> Box<[u8]>;
  async fn open(&self) -> anyhow::Result<(Self::Sink, Self::Stream)>;
}

#[async_trait]
pub(crate) trait UpstreamSink {
  async fn write(&mut self, buf: &mut [u8]) -> anyhow::Result<()>;
  async fn close(&mut self) -> anyhow::Result<()>;
}

#[async_trait]
pub(crate) trait UpstreamStream {
  async fn connect(&mut self, socket: &UdpSocket, addr: SocketAddr) -> anyhow::Result<()>;
}
