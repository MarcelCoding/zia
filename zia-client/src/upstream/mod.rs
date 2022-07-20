use anyhow::anyhow;
use tokio::net::UdpSocket;
use url::Url;

mod tcp;
mod ws;

pub(crate) async fn transmit(
  socket: UdpSocket,
  upstream: &Url,
  proxy: &Option<Url>,
) -> anyhow::Result<()> {
  match upstream.scheme() {
    "tcp" | "tcps" => tcp::transmit(socket, upstream, proxy).await,
    "ws" | "wss" => ws::transmit(socket, upstream, proxy).await,
    _ => Err(anyhow!("Unsupported upstream scheme {}", upstream.scheme())),
  }
}
