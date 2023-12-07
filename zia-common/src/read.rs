use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::select;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::task::{JoinError, JoinSet};
use tracing::{error, warn};

use crate::datagram_buffer;

#[async_trait::async_trait]
pub trait ReadConnection {
  fn is_closed(&self) -> bool;
  async fn handle_frame(
    &mut self,
    socket: &UdpSocket,
    addr: &RwLock<Option<SocketAddr>>,
    buf: &mut [u8],
  ) -> anyhow::Result<()>;
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

  async fn wait_for_connections_to_close(&self) -> Option<Result<anyhow::Result<()>, JoinError>> {
    let mut set = self.tasks.lock().await;
    select! {
      result = set.join_next() => result,
      _result = tokio::time::sleep(Duration::from_millis(200)) => Some(Ok(Ok(()))),
    }
  }

  pub async fn join(&self, dead_conn: Option<&mpsc::Sender<()>>) -> anyhow::Result<()> {
    // hack
    loop {
      while let Some(result) = self.wait_for_connections_to_close().await {
        if let Err(err) = result? {
          error!("Error while handling websocket frame: {}", err);
          // TODO: close and remove from write pool
          if let Some(dead_conn) = dead_conn {
            dead_conn.send(()).await?;
          }
        }
      }

      // hack
      tokio::time::sleep(Duration::from_secs(1)).await;
    }
  }

  pub async fn push<C: ReadConnection + Send + 'static>(&self, mut conn: C) {
    let socket = self.socket.clone();
    let addr = self.addr.clone();

    self.tasks.lock().await.spawn(async move {
      let mut buf = datagram_buffer();
      loop {
        if conn.is_closed() {
          warn!("Read connection closed");
          // TODO: open new connection on client
          return Ok(());
        }
        conn.handle_frame(&socket, &addr, buf.as_mut()).await?;
      }
    });
  }
}
