use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncWrite;
use tokio::net::UdpSocket;
use tokio::sync::RwLock;
use tracing::{error, warn};
use wsocket::{Message, WebSocket};

use crate::pool::{Pool, PoolEntry};
use crate::{datagram_buffer, MAX_DATAGRAM_SIZE};

pub struct WriteConnection<W> {
  write: WebSocket<W>,
  buf: Box<[u8; MAX_DATAGRAM_SIZE]>,
}

impl<W: AsyncWrite + Unpin> WriteConnection<W> {
  pub fn new(write: WebSocket<W>) -> Self {
    Self {
      buf: datagram_buffer(),
      write,
    }
  }

  async fn flush(&mut self, size: usize) -> anyhow::Result<()> {
    assert!(size <= MAX_DATAGRAM_SIZE);

    let message = Message::Binary(&self.buf[..size]);
    self.write.send(message).await?;

    Ok(())
  }
}

impl<W> PoolEntry for WriteConnection<W> {
  fn is_closed(&self) -> bool {
    self.write.is_closed()
    // TODO: open new connection on client - maybe fancy login in "abstract" pool
  }
}

pub struct WritePool<W> {
  socket: Arc<UdpSocket>,
  pool: Pool<WriteConnection<W>>,
  addr: Arc<RwLock<Option<SocketAddr>>>,
}

impl<W: AsyncWrite + Unpin + Send + 'static> WritePool<W> {
  pub fn new(socket: Arc<UdpSocket>, addr: Arc<RwLock<Option<SocketAddr>>>) -> Self {
    Self {
      socket,
      pool: Pool::new(),
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

  pub async fn push(&self, conn: WriteConnection<W>) {
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
      let (read, addr) = self.socket.recv_from(conn.buf.as_mut()).await.unwrap();

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
