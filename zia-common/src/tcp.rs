use std::net::SocketAddr;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UdpSocket;
use tokio::sync::RwLock;

use crate::pool::PoolEntry;
use crate::{datagram_buffer, ReadConnection, WriteConnection, MAX_DATAGRAM_SIZE};

pub struct ReadTcpConnection<R> {
  read: R,
}

impl<R> ReadTcpConnection<R> {
  pub fn new(read: R) -> Self {
    Self { read }
  }
}

#[async_trait::async_trait]
impl<R: AsyncRead + Unpin + Send> ReadConnection for ReadTcpConnection<R> {
  fn is_closed(&self) -> bool {
    false
  }

  async fn handle_frame(
    &mut self,
    socket: &UdpSocket,
    addr: &RwLock<Option<SocketAddr>>,
    buf: &mut [u8],
  ) -> anyhow::Result<()> {
    let frame_size = self.read.read_u16().await? as usize;
    self.read.read_exact(&mut buf[..frame_size]).await?;

    // drop packets until addr is known
    if let Some(addr) = addr.read().await.as_ref() {
      socket.send_to(&buf[..frame_size], addr).await?;
    }

    Ok(())
  }
}

pub struct WriteTcpConnection<W> {
  write: W,
  buf: Box<[u8; MAX_DATAGRAM_SIZE]>,
}

impl<W: AsyncWrite + Unpin> WriteTcpConnection<W> {
  pub fn new(write: W) -> Self {
    Self {
      buf: datagram_buffer(),
      write,
    }
  }
}

#[async_trait::async_trait]
impl<W: AsyncWrite + Unpin + Send> WriteConnection for WriteTcpConnection<W> {
  fn buf_as_mut(&mut self) -> &mut [u8] {
    self.buf.as_mut()
  }

  async fn flush(&mut self, size: usize) -> anyhow::Result<()> {
    assert!(size <= MAX_DATAGRAM_SIZE);

    self.write.write_u16(size as u16).await?;
    self.write.write_all(&self.buf[..size]).await?;
    self.write.flush().await?;

    Ok(())
  }
}

impl<W> PoolEntry for WriteTcpConnection<W> {
  fn is_closed(&self) -> bool {
    // self.write.is_closed()
    // TODO: open new connection on client - maybe fancy login in "abstract" pool
    // TODO: implement
    false
  }
}
