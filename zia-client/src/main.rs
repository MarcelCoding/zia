use std::net::SocketAddr;
use std::sync::Arc;

use clap::Parser;
use tokio::net::UdpSocket;
use tokio::select;
use tokio::signal::ctrl_c;
use tokio::sync::RwLock;
use tokio::task::JoinSet;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;
use url::Url;

use zia_common::{ReadPool, WritePool};

use crate::app::open_connection;
use crate::cfg::ClientCfg;

mod app;
mod cfg;

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

  select! {
    result = tokio::spawn(listen(config.listen_addr, config.upstream, config.proxy, config.count, config.websocket_masking)) => {
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

async fn listen(
  addr: SocketAddr,
  upstream: Url,
  proxy: Option<Url>,
  connection_count: usize,
  websocket_masking: bool,
) -> anyhow::Result<()> {
  let socket = Arc::new(UdpSocket::bind(addr).await?);

  let upstream = Arc::new(upstream);
  let proxy = Arc::new(proxy);

  let mut conns = JoinSet::new();
  for _ in 0..connection_count {
    let upstream = upstream.clone();
    let proxy = proxy.clone();
    conns.spawn(async move { open_connection(&upstream, &proxy, websocket_masking).await });
  }

  let addr = Arc::new(RwLock::new(Option::None));

  let write_pool = WritePool::new(socket.clone(), addr.clone());
  let read_pool = ReadPool::new(socket, addr);

  while let Some(connection) = conns.join_next().await.transpose()? {
    let (read, write) = connection?;
    read_pool.push(read).await;
    write_pool.push(write).await;
  }

  info!("Connected to upstream");

  let write_handle = tokio::spawn(async move {
    loop {
      write_pool.execute().await?;
    }
  });

  select! {
    result = write_handle => {
      info!("Write pool finished");
      result?
    },
    result = read_pool.join() => {
      info!("Read pool finished");
      result
    },
  }
}
