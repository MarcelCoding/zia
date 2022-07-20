#[cfg(not(target_env = "msvc"))]
use tikv_jemallocator::Jemalloc;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

use tokio::{select, signal};
use tracing::info;

use crate::cfg::{ClientCfg, Mode};
use crate::listener::{Listener, TcpListener, WsListener};

mod cfg;
mod listener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  tracing_subscriber::fmt::init();

  let config = ClientCfg::load()?;

  let listener: Box<dyn Listener> = match config.mode {
    Mode::Ws => Box::new(WsListener {
      addr: config.listen_addr,
    }),
    Mode::Tcp => Box::new(TcpListener {
      addr: config.listen_addr,
    }),
  };

  info!("Listening in {}://{}...", config.mode, config.listen_addr);

  select! {
    result = listener.listen(config.upstream) => {
      result?;
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
