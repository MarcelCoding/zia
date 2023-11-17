use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use clap::Parser;
use hyper::service::{make_service_fn, Service};
use hyper::upgrade::Upgraded;
use hyper::{Body, Request, Response, Server, StatusCode};
use tokio::net::UdpSocket;
use tokio::select;
use tokio::signal::ctrl_c;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::info;
use wsocket::WebSocket;

use zia_common::{ReadConnection, ReadPool, WriteConnection, WritePool, MAX_DATAGRAM_SIZE};

use crate::cfg::ServerCfg;

mod cfg;

#[pin_project::pin_project]
struct HandleRequestFuture {
  req: Request<Body>,
  read: Arc<ReadPool>,
  write: Arc<WritePool<Upgraded>>,
}

impl Future for HandleRequestFuture {
  type Output = Result<Response<Body>, Infallible>;

  fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
    let this = self.project();

    if !fastwebsockets::upgrade::is_upgrade_request(this.req) {
      return Poll::Ready(Ok(
        Response::builder()
          .status(StatusCode::BAD_REQUEST)
          .body(Body::from("bad request: expected websocket upgrade"))
          .unwrap(),
      ));
    }

    let (resp, upgrade) = fastwebsockets::upgrade::upgrade(this.req).unwrap();

    let wread = this.read.clone();
    let wwrite = this.write.clone();

    tokio::spawn(async move {
      let ws = upgrade.await.unwrap().into_inner();

      let ws = WebSocket::server(ws, MAX_DATAGRAM_SIZE);
      let (read, write) = ws.split();

      wread.push(ReadConnection::new(read)).await;
      wwrite.push(WriteConnection::new(write)).await;
    });

    Poll::Ready(Ok(resp))
  }
}

// mod app;
struct ConnectionHandler {
  read: Arc<ReadPool>,
  write: Arc<WritePool<Upgraded>>,
}

impl Service<Request<Body>> for ConnectionHandler {
  type Response = Response<Body>;
  type Error = Infallible;
  type Future = HandleRequestFuture;

  fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
    Poll::Ready(Ok(()))
  }

  fn call(&mut self, req: Request<Body>) -> Self::Future {
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

  let make_service = make_service_fn(|_conn| {
    let read = rp.clone();
    let write = wp.clone();

    async move { Ok::<_, Infallible>(ConnectionHandler { read, write }) }
  });

  let server = Server::bind(&config.listen_addr).serve(make_service);

  info!("Listening on {}://{}...", config.mode, config.listen_addr);

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
