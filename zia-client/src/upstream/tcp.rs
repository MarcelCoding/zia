use std::mem;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::io::{split, AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::{TcpStream, UdpSocket};
use url::Url;

use zia_common::Stream;

use crate::handler::UdpHandler;
use crate::upstream::{Upstream, UpstreamSink, UpstreamStream};

pub(crate) async fn transmit(
  socket: UdpSocket,
  upstream: &Url,
  proxy: &Option<Url>,
) -> anyhow::Result<()> {
  let upstream = TcpUpstream {
    url: upstream.clone(),
    proxy: proxy.clone(),
  };

  let handler = UdpHandler::new(upstream);
  handler.listen(Arc::new(socket)).await
}

struct TcpUpstream {
  url: Url,
  proxy: Option<Url>,
}

struct TcpUpstreamSink {
  inner: WriteHalf<Stream<TcpStream>>,
}

struct TcpUpstreamStream {
  inner: ReadHalf<Stream<TcpStream>>,
}

#[async_trait]
impl Upstream for TcpUpstream {
  type Sink = TcpUpstreamSink;
  type Stream = TcpUpstreamStream;

  /// A UDP datagram header has a 16 bit field containing an unsigned integer
  /// describing the length of the datagram (including the header itself).
  /// The max value is 2^16 = 65536 bytes. But since that includes the
  /// UDP header, this constant is 8 bytes more than any UDP socket
  /// read operation would ever return. We are going to use that extra space.
  const BUF_SIZE: usize = u16::MAX as usize;
  const BUF_SKIP: usize = mem::size_of::<u16>();

  fn buffer() -> Box<[u8]> {
    Box::new([0_u8; Self::BUF_SIZE])
  }

  async fn open(&self) -> anyhow::Result<(Self::Sink, Self::Stream)> {
    let stream = Stream::connect(&self.url, &self.proxy).await?;
    let (stream, sink) = split(stream);

    let sink = Self::Sink { inner: sink };
    let stream = Self::Stream { inner: stream };

    Ok((sink, stream))
  }
}

#[async_trait]
impl UpstreamSink for TcpUpstreamSink {
  async fn write(&mut self, buf: &mut [u8]) -> anyhow::Result<()> {
    let len = (buf.len() - TcpUpstream::BUF_SKIP) as u16;

    buf[0..TcpUpstream::BUF_SKIP].copy_from_slice(&len.to_le_bytes());

    self.inner.write_all(buf).await?;

    Ok(())
  }

  async fn close(&mut self) -> anyhow::Result<()> {
    Ok(self.inner.shutdown().await?)
  }
}

#[async_trait]
impl UpstreamStream for TcpUpstreamStream {
  async fn connect(&mut self, socket: &UdpSocket, addr: SocketAddr) -> anyhow::Result<()> {
    let mut buf = TcpUpstream::buffer();
    let mut unprocessed = 0;

    loop {
      let read = self.inner.read(&mut buf[unprocessed..]).await?;

      if read == 0 {
        return Err(anyhow!("End of tcp upstream stream."));
      }

      unprocessed += read;

      let processed = forward(socket, addr, &buf[..unprocessed]).await?;

      // discard processed bytes
      if unprocessed > processed {
        buf.copy_within(processed..unprocessed, 0);
      }

      unprocessed -= processed;
    }
  }
}

async fn forward(socket: &UdpSocket, addr: SocketAddr, buf: &[u8]) -> anyhow::Result<usize> {
  let mut start = 0;

  loop {
    let body = start + TcpUpstream::BUF_SKIP;

    let len: [u8; TcpUpstream::BUF_SKIP] = match buf.get(start..body) {
      Some(header) => header.try_into().unwrap(),
      // not enough bytes for a complete header
      None => return Ok(start),
    };

    let len = u16::from_le_bytes(len) as usize;
    let end = body + len;

    let data = match buf.get(body..end) {
      Some(data) => data,
      // not enough bytes for a complete dataframe
      None => return Ok(start),
    };

    let written = socket.send_to(data, addr).await?;
    assert_eq!(len, written, "Did not send entire UDP datagram");

    start = end;
  }
}

// Creates and returns a buffer on the heap with enough space to contain any possible
// UDP datagram.
//
// This is put on the heap and in a separate function to avoid the 64k buffer from ending
// up on the stack and blowing up the size of the futures using it.
// #[inline(never)]
// fn datagram_buffer<U: Upstream>() -> Box<[u8; U::BUF_SIZE]> {
//   Box::new([0u8; U::BUF_SIZE])
// }
