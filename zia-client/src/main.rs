extern crate core;

use std::net::SocketAddr;
use std::str;
use std::sync::Arc;

use anyhow::anyhow;
use httparse::Request;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::ReadHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::{select, signal};
use tracing::{error, info};

use url::Url;

use crate::upstream::{Connection, Upstream, WsUpstream};

mod upstream;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  tracing_subscriber::fmt::init();

  let addr: SocketAddr = "127.0.0.1:8080".parse()?;

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
  let upstream = Arc::new(WsUpstream {
    url: Url::parse("ws://127.0.0.1:1234")?,
  });
  let listener = TcpListener::bind(addr).await?;
  info!("Listening on {}...", addr);

  loop {
    let (sock, _) = listener.accept().await?;
    let up = upstream.clone();

    tokio::spawn(async move {
      if let Err(err) = handle(up, sock).await {
        error!("Error while handling connection: {:?}", err);
      }
    });
  }
}

// \r\n\r\n
const HTTP_HEADER_END: [u8; 4] = [0xd, 0xa, 0xd, 0xa];

// 200 Ok\r\n\r\n
const HTTP_OK: [u8; 10] = [0x32, 0x30, 0x30, 0x20, 0x4f, 0x6b, 0xd, 0xa, 0xd, 0xa];

async fn handle<U: Upstream>(upstream: Arc<U>, mut inbound: TcpStream) -> anyhow::Result<()> {
  // inbound
  let (mut ri, mut wi) = inbound.split();

  let dest = read_host(&mut ri).await?;

  info!("> Connecting {}...", dest);
  let conn = upstream.connect(&dest).await?;

  // connection successful
  wi.write_all(&HTTP_OK).await?;

  info!("+ Connection to {} opened...", dest);

  conn.mount(inbound).await?;

  info!("- Connection to {} closed...", dest);

  Ok(())
}

async fn read_host(rx: &mut ReadHalf<'_>) -> anyhow::Result<String> {
  let mut buf = Vec::with_capacity(1024);

  // read till end of http header "\r\n\r\n"
  rx.read_buf(&mut buf).await?;
  while !buf.ends_with(&HTTP_HEADER_END) {
    if rx.read_buf(&mut buf).await? == 0 {
      return Err(anyhow!("Socket closed"));
    }
  }

  let mut headers = [httparse::EMPTY_HEADER; 64];
  let mut req = Request::new(&mut headers);

  req.parse(&buf)?;

  let path = req.path.unwrap().to_lowercase();

  for header in req.headers {
    if header.name.to_lowercase() == "host" {
      let host = str::from_utf8(header.value)?;

      if !host.contains(':') {
        if path.starts_with("http://") {
          return Ok(host.to_owned() + ":80");
        } else if path.starts_with("https://") {
          return Ok(host.to_owned() + ":443");
        } else {
          panic!("Missing port");
        }
      }

      return Ok(host.to_owned());
    }
  }

  Err(anyhow!("Unable to extract destination host"))
}
