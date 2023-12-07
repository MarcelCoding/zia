use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::RwLock;
use tracing::{error, warn};

use crate::pool::{Pool, PoolEntry};

#[async_trait::async_trait]
pub trait WriteConnection {
  fn buf_as_mut(&mut self) -> &mut [u8];
  async fn flush(&mut self, size: usize) -> anyhow::Result<()>;
}

pub struct WritePool<C: PoolEntry> {
  socket: Arc<UdpSocket>,
  pool: Pool<C>,
  addr: Arc<RwLock<Option<SocketAddr>>>,
}

impl<C: WriteConnection + PoolEntry + Send + 'static> WritePool<C> {
  pub fn new(socket: Arc<UdpSocket>, addr: Arc<RwLock<Option<SocketAddr>>>) -> Self {
    Self {
      socket,
      pool: Pool::default(),
      addr,
    }
  }

  async fn update_addr(&self, addr: SocketAddr) {
    let is_outdated = self
      .addr
      .read()
      .await
      .map(|last_addr| last_addr != addr)
      .unwrap_or(true);

    if is_outdated {
      *(self.addr.write().await) = Some(addr);
    }
  }

  pub async fn push(&self, conn: C) {
    self.pool.push(conn);
  }

  pub async fn join(&self) {
    loop {
      let conn = self.pool.acquire().await;

      // TODO:
      // maybe just block until it is not empty anymore
      // .revc() in self.pool.acquire() would be "blocking" (asynchronous)
      // until a connection becomes available, therefore
      // this would be appropriate
      // - better would be an option to enable the blocking
      // only on the server and on the client a `None`
      // returned would open a new connection
      let mut conn = match conn {
        Some(conn) => conn,
        None => {
          warn!("Write pool is empty, waiting 1s");
          tokio::time::sleep(Duration::from_secs(1)).await;
          continue;
        }
      };

      if conn.is_closed() {
        continue;
      }

      // read from udp socket and save to buf of selected conn
      let (read, addr) = self.socket.recv_from(conn.buf_as_mut()).await.unwrap();

      self.update_addr(addr).await;

      // flush buf of conn asynchronously to read again from udp socket in parallel
      tokio::spawn(async move {
        if let Err(err) = conn.flush(read).await {
          error!("Unable to flush websocket buf: {:?}", err);
        }
      });
    }
  }
}
