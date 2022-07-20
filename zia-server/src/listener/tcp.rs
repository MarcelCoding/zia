use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::{TcpStream, UdpSocket};
use tracing::{error, info};

use zia_common::process_udp_over_tcp;
use zia_common::Stream::Plain;

use crate::listener::Listener;

pub(crate) struct TcpListener {
  pub(crate) addr: SocketAddr,
}

#[async_trait::async_trait]
impl Listener for TcpListener {
  async fn listen(&self, upstream: SocketAddr) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(self.addr).await?;

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

impl TcpListener {
  async fn handle(downstream: TcpStream, upstream_addr: SocketAddr) -> anyhow::Result<()> {
    downstream.set_nodelay(true)?;
    let downstream_addr = downstream.peer_addr()?;
    info!("New downstream connection: {}", downstream_addr);

    let upstream = UdpSocket::bind("0.0.0.0:0").await?; // TODO: maybe make this configurable

    upstream.connect(upstream_addr).await?;

    info!(
      "Connected to udp upstream (local: {}/udp, peer: {}/udp) for downstream {}",
      upstream.local_addr()?,
      upstream.peer_addr()?,
      downstream_addr
    );

    process_udp_over_tcp(upstream, Plain(downstream), Some(Duration::from_secs(60))).await;

    info!("Connection with downstream {} closed...", downstream_addr);

    Ok(())
  }
}
