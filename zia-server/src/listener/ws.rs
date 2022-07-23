use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tracing::{error, info};
use zia_common::process_udp_over_ws;
use zia_common::Stream::Plain;

use crate::listener::Listener;

pub(crate) struct WsListener {
  pub(crate) addr: SocketAddr,
}

#[async_trait::async_trait]
impl Listener for WsListener {
  async fn listen(&self, upstream: SocketAddr) -> anyhow::Result<()> {
    let listener = TcpListener::bind(self.addr).await?;
    info!("Listening on ws://{}...", listener.local_addr()?);

    loop {
      let (sock, _) = listener.accept().await?;

      tokio::spawn(async move {
        if let Err(err) = Self::handle(sock, upstream).await {
          error!("Error while handling connection: {:?}", err);
        }
      });
    }
  }
}

impl WsListener {
  async fn handle(downstream: TcpStream, upstream_addr: SocketAddr) -> anyhow::Result<()> {
    downstream.set_nodelay(true)?;
    let downstream_addr = downstream.peer_addr()?;
    info!("New downstream connection: {}", downstream_addr);
    let downstream = tokio_tungstenite::accept_async(Plain(downstream)).await?;

    let upstream = UdpSocket::bind("0.0.0.0:0").await?; // TODO: maybe make this configurable

    upstream.connect(upstream_addr).await?;

    info!(
      "Connected to udp upstream (local: {}/udp, peer: {}/udp) for downstream {}",
      upstream.local_addr()?,
      upstream.peer_addr()?,
      downstream_addr
    );

    process_udp_over_ws(upstream, downstream, Some(Duration::from_secs(60))).await;

    info!("Connection with downstream {} closed...", downstream_addr);

    Ok(())
  }
}
