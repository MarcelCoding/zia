use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use tokio::net::{TcpListener, TcpStream};
use tokio::{select, signal, try_join};
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  tracing_subscriber::fmt::init();

  let addr: SocketAddr = "127.0.0.1:1234".parse()?;

  select! {
    result = accept_connections(addr) => {
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

async fn accept_connections(addr: SocketAddr) -> anyhow::Result<()> {
  let listener = TcpListener::bind(addr).await?;
  info!("Listening on {}...", addr);

  loop {
    let (sock, _) = listener.accept().await?;

    tokio::spawn(async move {
      if let Err(err) = handle(sock).await {
        error!("Error while handling connection: {:?}", err);
      }
    });
  }
}

async fn handle(stream: TcpStream) -> anyhow::Result<()> {
  info!("New connection: {}", stream.peer_addr()?);
  let ws_stream = tokio_tungstenite::accept_async(stream).await?;

  let (mut wi, mut ri) = ws_stream.split();

  let next = ri.next().await.unwrap()?.into_text()?;
  info!("{}", next);

  let mut stream1 = TcpStream::connect(next).await?;
  let (mut ro, mut wo) = stream1.split();

  let server_to_client = async {
    let mut buf = [0; 16384];

    let mut read = ro.read(&mut buf[..]).await?;
    while read > 0 {
      wi.send(Message::binary(&buf[0..read])).await?;
      read = ro.read(&mut buf[..]).await?;
    }

    wi.close().await?;
    Ok::<_, anyhow::Error>(())
  };

  let client_to_server = async {
    while let Some(next) = ri.next().await {
      wo.write_all(&next?.into_data()).await?;
    }

    wo.shutdown().await?;
    Ok::<_, anyhow::Error>(())
  };

  try_join!(client_to_server, server_to_client)?;

  // We should not forward messages other than text or binary.
  // read.try_filter(|msg| future::ready(msg.is_text() || msg.is_binary()))
  //   .forward(write)
  //   .await
  //   .expect("Failed to forward messages")

  Ok(())
}
