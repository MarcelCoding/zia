use std::net::SocketAddr;

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::try_join;
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info};

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
  async fn handle(raw_downstream: TcpStream, upstream_addr: SocketAddr) -> anyhow::Result<()> {
    let downstream_addr = raw_downstream.peer_addr()?;
    raw_downstream.set_nodelay(true)?;
    info!("New downstream connection: {}", downstream_addr);
    let downstream = tokio_tungstenite::accept_async(raw_downstream).await?;

    let (mut wi, mut ri) = downstream.split();

    let upstream = UdpSocket::bind("0.0.0.0:0").await?; // TODO: maybe make this configurable

    upstream.connect(upstream_addr).await?;

    info!(
      "Connected to udp upstream (local: {}/udp, peer: {}/udp) for downstream {}",
      upstream.local_addr()?,
      upstream.peer_addr()?,
      downstream_addr
    );

    let server_to_client = async {
      let mut buf = [0; 65507];

      let mut read = upstream.recv(&mut buf).await?;
      while read != 0 {
        wi.send(Message::binary(&buf[..read])).await?;
        read = upstream.recv(&mut buf).await?;
      }

      wi.close().await?;
      Ok::<_, anyhow::Error>(())
    };

    let client_to_server = async {
      while let Some(next) = ri.next().await {
        let x = &next?.into_data();
        upstream.send(x).await?;
      }

      Ok::<_, anyhow::Error>(())
    };

    try_join!(client_to_server, server_to_client)?;

    Ok(())
  }
}
