use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, ReadHalf};
use tokio::net::UdpSocket;
use tokio::select;
use tokio::sync::{Mutex, RwLock};
use tokio::task::{JoinError, JoinSet};
use tracing::error;
use crate::datagram_buffer;

use crate::ws::{Event, WebSocket};

pub struct ReadConnection<R> {
  read: WebSocket<ReadHalf<R>>,
}

impl<R: AsyncRead> ReadConnection<R> {
  pub fn new(read: WebSocket<ReadHalf<R>>) -> Self {
    Self { read }
  }

  async fn handle_frame(
    &mut self,
    socket: &UdpSocket,
    addr: &RwLock<Option<SocketAddr>>,
    buf: &mut [u8],
  ) -> anyhow::Result<()> {
    let event = self.read.recv(buf).await?;

    match event {
      Event::Data(data) => {
        let addr = addr.read().await.unwrap();
        socket.send_to(data, addr).await?;
      }
      Event::Close { .. } => {}
    }

    Ok(())
  }
}

pub struct ReadPool {
  socket: Arc<UdpSocket>,
  addr: Arc<RwLock<Option<SocketAddr>>>,
  tasks: Mutex<JoinSet<anyhow::Result<()>>>,
}

impl ReadPool {
  pub fn new(socket: Arc<UdpSocket>, addr: Arc<RwLock<Option<SocketAddr>>>) -> Self {
    Self {
      socket,
      addr,
      tasks: Mutex::new(JoinSet::new()),
    }
  }

  async fn wait(&self) -> Option<Result<anyhow::Result<()>, JoinError>> {
    let mut set = self.tasks.lock().await;
    select! {
      result = set.join_next() => result,
      _result = tokio::time::sleep(Duration::from_millis(200)) => Some(Ok(Ok(()))),
    }
  }

  pub async fn join(&self) -> anyhow::Result<()> {
    // hack
    loop {
      while let Some(result) = self.wait().await {
        if let Err(err) = result? {
          error!("Error while handling websocket frame: {}", err);
          // TODO: close and remove from write pool
        }
      }

      // hack
      tokio::time::sleep(Duration::from_secs(1)).await;
    }
  }

  pub async fn push<R: AsyncRead + Send + 'static>(&self, mut conn: ReadConnection<R>) {
    let socket = self.socket.clone();
    let addr = self.addr.clone();

    self.tasks.lock().await.spawn(async move {
      let mut buf = datagram_buffer();
      loop {
        conn.handle_frame(&socket, &addr,  buf.as_mut()).await?;
      }
    });
  }
}
