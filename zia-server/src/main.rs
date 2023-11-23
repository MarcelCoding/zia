use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use clap::Parser;
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::service::Service;
use hyper::upgrade::Upgraded;
use hyper::{Request, Response, StatusCode};
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use pin_project_lite::pin_project;
use tokio::io::WriteHalf;
use tokio::net::{TcpListener, UdpSocket};
use tokio::select;
use tokio::signal::ctrl_c;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{error, info};
use wsocket::{is_upgrade_request, upgrade};

use zia_common::{ReadConnection, ReadPool, WriteConnection, WritePool, MAX_DATAGRAM_SIZE};

use crate::cfg::ServerCfg;

mod cfg;

pin_project! {
  struct HandleRequestFuture {
    req: Request<Incoming>,
    read: Arc<ReadPool>,
    write: Arc<WritePool<WriteHalf<TokioIo<Upgraded>>>>,
  }
}

impl Future for HandleRequestFuture {
  type Output = Result<Response<Full<Bytes>>, Infallible>;

  fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
    let this = self.project();

    if !is_upgrade_request(this.req) {
      return Poll::Ready(Ok(
        Response::builder()
          .status(StatusCode::BAD_REQUEST)
          .body(Full::new(Bytes::from(
            "bad request: expected websocket upgrade",
          )))
          .unwrap(),
      ));
    }

    let (resp, upgrade) = match upgrade(this.req, MAX_DATAGRAM_SIZE) {
      Ok(res) => res,
      Err(err) => {
        error!("Error: {:?}", err);
        let mut resp = Response::new(Full::new(Bytes::from("bad request")));
        *resp.status_mut() = StatusCode::BAD_REQUEST;
        return Poll::Ready(Ok(resp));
      }
    };

    let cloned_read = this.read.clone();
    let cloned_write = this.write.clone();

    tokio::spawn(async move {
      match upgrade.await {
        Ok(ws) => {
          let (read, write) = ws.split();

          cloned_read.push(ReadConnection::new(read)).await;
          cloned_write.push(WriteConnection::new(write)).await;
        }
        Err(err) => error!("Error while upgrading connection: {:?}", err),
      }
    });

    Poll::Ready(Ok(resp))
  }
}

// mod app;
struct ConnectionHandler {
  read: Arc<ReadPool>,
  write: Arc<WritePool<WriteHalf<TokioIo<Upgraded>>>>,
}

impl Service<Request<Incoming>> for ConnectionHandler {
  type Response = Response<Full<Bytes>>;
  type Error = Infallible;
  type Future = HandleRequestFuture;

  fn call(&self, req: Request<Incoming>) -> Self::Future {
    HandleRequestFuture {
      req,
      read: self.read.clone(),
      write: self.write.clone(),
    }
  }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  let config = ServerCfg::parse();

  tracing_subscriber::fmt::init();

  let socket = Arc::new(UdpSocket::bind(Into::<SocketAddr>::into(([0, 0, 0, 0], 0))).await?);
  socket.connect(&config.upstream).await?;
  info!("Connected to upstream udp://{}...", config.upstream);

  let addr = Arc::new(RwLock::new(Some(socket.peer_addr()?)));
  let write_pool = Arc::new(WritePool::new(socket.clone(), addr.clone()));
  let read_pool = Arc::new(ReadPool::new(socket, addr));

  let wp = write_pool.clone();
  let rp = read_pool.clone();

  let server = TcpListener::bind(&config.listen_addr).await?;

  info!("Listening on {}://{}...", config.mode, config.listen_addr);

  let server = tokio::spawn(async move {
    loop {
      let (stream, _) = server.accept().await.unwrap();

      let io = TokioIo::new(stream);

      let read = rp.clone();
      let write = wp.clone();

      let service = ConnectionHandler { read, write };

      let conn = http1::Builder::new().serve_connection(io, service);

      tokio::spawn(async move {
        if let Err(err) = conn.with_upgrades().await {
          error!("Error: {:?}", err);
        }
      });
    }
  });

  let write_handle: JoinHandle<anyhow::Result<()>> = tokio::spawn(async move {
    loop {
      write_pool.execute().await?;
    }
  });

  select! {
    result = server => {
      info!("Socket closed, quitting...");
      result?;
    },
      result = write_handle => {
      info!("Write pool finished");
      result??;
    },
    result = read_pool.join() => {
      info!("Read pool finished");
      result?;
    },
    result = shutdown_signal() => {
      info!("Termination signal received, quitting...");
      result?;
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
