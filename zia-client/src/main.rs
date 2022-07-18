extern crate core;

use std::net::SocketAddr;

use tokio::{select, signal};
use tokio::net::UdpSocket;
use tracing::info;

use crate::cfg::ClientCfg;
use crate::upstream::{Connection, Upstream, WsUpstream};

mod cfg;
mod upstream;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  tracing_subscriber::fmt::init();

  let config = ClientCfg::load()?;

  info!("Using websocket upstream at {}", config.upstream);

  let upstream = WsUpstream {
    url: config.upstream,
    proxy: config.proxy,
  };

  select! {
    result = listen(config.listen_addr, upstream) => {
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

    select! {
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

async fn listen<U: Upstream>(addr: SocketAddr, upstream: U) -> anyhow::Result<()> {
  let connection = upstream.connect().await?;

  let inbound = UdpSocket::bind(addr).await?;
  info!("Listening on {}/udp", inbound.local_addr()?);

  connection.mount(inbound).await?;

  Ok(())
}
