extern crate core;

use std::net::SocketAddr;

use clap::Parser;
use tokio::{select, signal};
use tokio::net::UdpSocket;
use tracing::info;
use url::Url;

use crate::upstream::{Connection, Upstream, WsUpstream};

mod upstream;

#[derive(Parser)]
struct Args {
  listen_addr: SocketAddr,
  upstream_url: Url,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  tracing_subscriber::fmt::init();

  let args = Args::parse();

  info!("Using websocket upstream at {}", args.upstream_url);

  let upstream = WsUpstream {
    url: args.upstream_url,
  };

  select! {
    result = listen(args.listen_addr, upstream) => {
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
