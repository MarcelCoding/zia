use std::net::SocketAddr;

use futures_util::{SinkExt, StreamExt};
use tokio::{select, signal, try_join};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info};

use crate::cfg::ClientCfg;

mod cfg;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  tracing_subscriber::fmt::init();

  let config = ClientCfg::load()?;

  select! {
    result = accept_connections(config.listen_addr, config.upstream) => {
      result?;
      info!("Socket closed, quitting...");
    },
    result = shutdown_signal() => {
      result?;
      info!("Termination signal received, quitting...")
    }
  }

  Ok(())
}

async fn shutdown_signal() -> anyhow::Result<()> {
  let ctrl_c = async {
    signal::ctrl_c()
      .await
      .expect("failed to install Ctrl+C handler")
  };

  #[cfg(unix)]
  {
    let terminate = async {
      signal::unix::signal(signal::unix::SignalKind::terminate())
        .expect("failed to install signal handler")
        .recv()
        .await;
    };

    tokio::select! {
      _ = ctrl_c => {},
      _ = terminate => {},
    }

    Ok(())
  }

  #[cfg(not(unix))]
  {
    ctrl_c.await;
    Ok(())
  }
}

async fn accept_connections(
  listen_addr: SocketAddr,
  upstream: SocketAddr,
) -> anyhow::Result<()> {
  let listener = TcpListener::bind(listen_addr).await?;
  info!("Listening on ws://{}...", listener.local_addr()?);

  loop {
    let (sock, _) = listener.accept().await?;

    tokio::spawn(async move {
      if let Err(err) = handle(sock, upstream).await {
        error!("Error while handling connection: {:?}", err);
      }
    });
  }
}

async fn handle(raw_downstream: TcpStream, upstream_addr: SocketAddr) -> anyhow::Result<()> {
  let downstream_addr = raw_downstream.peer_addr()?;
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

    let mut read = upstream.recv(&mut buf[..]).await?;
    while read > 0 {
      wi.send(Message::binary(&buf[0..read])).await?;
      read = upstream.recv(&mut buf[..]).await?;
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
