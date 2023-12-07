use anyhow::anyhow;
use clap::Parser;
use tokio::net::UdpSocket;
use tokio::select;
use tokio::signal::ctrl_c;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

use crate::cfg::ClientCfg;
use crate::handler::{Handler, TcpConnectionManager, WsConnectionManager};

mod cfg;
mod handler;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  let config = ClientCfg::parse();

  let subscriber = FmtSubscriber::builder()
    .with_max_level(Level::INFO)
    .compact()
    .finish();

  tracing::subscriber::set_global_default(subscriber)?;

  info!(concat!(
    "Booting ",
    env!("CARGO_PKG_NAME"),
    "/",
    env!("CARGO_PKG_VERSION"),
    "..."
  ));

  let socket = UdpSocket::bind(config.listen_addr).await?;
  info!("Listening on {}/udp...", config.listen_addr);

  let handler = match config.upstream.scheme() {
    "ws" | "wss" => tokio::spawn(async move {
      let handler = Handler::<WsConnectionManager>::new(
        socket,
        config.upstream,
        config.proxy,
        config.ws_masking,
      )?;
      handler.run(config.count).await
    }),
    "tcp" | "tcps" => tokio::spawn(async move {
      let handler = Handler::<TcpConnectionManager>::new(
        socket,
        config.upstream,
        config.proxy,
        config.ws_masking,
      )?;
      handler.run(config.count).await
    }),
    _ => return Err(anyhow!("Umsupported upstream protocol")),
  };

  select! {
    result = handler => {
      result??;
      info!("Socket closed, quitting...");
    },
    result = shutdown_signal() => {
      result?;
      info!("Termination signal received, quitting...");
    }
  }

  Ok(())
}

async fn shutdown_signal() -> anyhow::Result<()> {
  let ctrl_c = async { ctrl_c().await.expect("failed to install Ctrl+C handler") };

  #[cfg(unix)]
  {
    let terminate = async {
      tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
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
