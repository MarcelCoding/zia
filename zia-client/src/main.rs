use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::app::{Connection, WritePool};
use clap::Parser;
use fastwebsockets::{Frame, OpCode, Payload};
use tokio::net::UdpSocket;
use tokio::select;
use tokio::signal::ctrl_c;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tracing::{error, info};
use url::Url;

use crate::cfg::ClientCfg;

mod app;
mod cfg;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  let config = ClientCfg::parse();

  tracing_subscriber::fmt::init();

  select! {
    result = tokio::spawn(listen(config.listen_addr, config.upstream, config.proxy, config.count)) => {
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
) -> anyhow::Result<()> {
  let socket = Arc::new(UdpSocket::bind(addr).await?);

  let upstream = Arc::new(upstream);
  let proxy = Arc::new(proxy);

  let mut conns = JoinSet::new();
  for _ in 0..connection_count {
    let upstream = upstream.clone();
    let proxy = proxy.clone();
    conns.spawn(async move { Connection::new(&upstream, &proxy).await });
  }

  let mut connections = Vec::new();
  let mut read_pool = Vec::new();

  while let Some(connection) = conns.join_next().await.transpose()? {
    let (conn, read) = connection?;
    connections.push(conn);
    read_pool.push(read);
  }

  info!("Connected to upstream");

  let addr = Arc::new(Mutex::new(Option::None));

  async fn test(_: Frame<'_>) -> Result<(), Infallible> {
    todo!()
  }

  for mut ws_read in read_pool {
    let addr = addr.clone();
    let socket = socket.clone();

    tokio::spawn(async move {
      loop {
        let frame = ws_read.read_frame(&mut test).await.unwrap();

        if !frame.fin {
          error!(
            "unexpected buffer received, expect full udp frame to be in one websocket message"
          );
          continue;
        }

        match frame.opcode {
          OpCode::Binary => match frame.payload {
            Payload::BorrowedMut(payload) => {
              socket
                .send_to(payload, addr.lock().await.unwrap())
                .await
                .unwrap();
            }
            Payload::Borrowed(payload) => {
              socket
                .send_to(payload, addr.lock().await.unwrap())
                .await
                .unwrap();
            }
            Payload::Owned(payload) => {
              socket
                .send_to(&payload, addr.lock().await.unwrap())
                .await
                .unwrap();
            }
          },
          opcode => error!("Unexpected opcode: {:?}", opcode),
        }
      }
    });
  }

  let socket = socket.clone();
  let old_addr = addr.clone();

  let mut pool = WritePool {
    connections,
    next: 0,
  };

  loop {
    pool.write(&socket, old_addr.clone()).await?;
  }
}
