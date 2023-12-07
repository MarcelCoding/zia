use std::net::SocketAddr;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UdpSocket;
use tokio::sync::RwLock;
use wsocket::{Message, WebSocket};

use crate::pool::PoolEntry;
use crate::{datagram_buffer, ReadConnection, WriteConnection, MAX_DATAGRAM_SIZE};

pub struct ReadWsConnection<R> {
  read: WebSocket<R>,
}

impl<R> ReadWsConnection<R> {
  pub fn new(read: WebSocket<R>) -> Self {
    Self { read }
  }
}

#[async_trait::async_trait]
impl<R: AsyncRead + Unpin + Send> ReadConnection for ReadWsConnection<R> {
  fn is_closed(&self) -> bool {
    self.read.is_closed()
  }

  async fn handle_frame(
    &mut self,
    socket: &UdpSocket,
    addr: &RwLock<Option<SocketAddr>>,
    buf: &mut [u8],
  ) -> anyhow::Result<()> {
    let message = self.read.recv(buf).await?;

    match message {
      Message::Binary(data) => {
        // drop packets until addr is known
        if let Some(addr) = addr.read().await.as_ref() {
          socket.send_to(data, addr).await?;
        }
      }
      _ => unimplemented!(),
    }

    Ok(())
  }
}

pub struct WriteWsConnection<W> {
  write: WebSocket<W>,
  buf: Box<[u8; MAX_DATAGRAM_SIZE]>,
}

impl<W: AsyncWrite + Unpin> WriteWsConnection<W> {
  pub fn new(write: WebSocket<W>) -> Self {
    Self {
      buf: datagram_buffer(),
      write,
    }
  }
}

#[async_trait::async_trait]
impl<W: AsyncWrite + Unpin + Send> WriteConnection for WriteWsConnection<W> {
  fn buf_as_mut(&mut self) -> &mut [u8] {
    self.buf.as_mut()
  }

  async fn flush(&mut self, size: usize) -> anyhow::Result<()> {
    assert!(size <= MAX_DATAGRAM_SIZE);

    let message = Message::Binary(&self.buf[..size]);
    self.write.send(message).await?;

    Ok(())
  }
}

impl<W> PoolEntry for WriteWsConnection<W> {
  fn is_closed(&self) -> bool {
    self.write.is_closed()
    // TODO: open new connection on client - maybe fancy login in "abstract" pool
  }
}
