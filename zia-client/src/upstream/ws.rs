use std::mem;
use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{client_async, WebSocketStream};
use url::Url;

use zia_common::Stream;

use crate::handler::UdpHandler;
use crate::upstream::{Upstream, UpstreamSink, UpstreamStream};

pub(crate) async fn transmit(
  socket: UdpSocket,
  upstream: &Url,
  proxy: &Option<Url>,
) -> anyhow::Result<()> {
  let upstream = WsUpstream {
    url: upstream.clone(),
    proxy: proxy.clone(),
  };

  let handler = UdpHandler::new(upstream);
  handler.listen(Arc::new(socket)).await
}

struct WsUpstream {
  url: Url,
  proxy: Option<Url>,
}

struct WsUpstreamSink {
  inner: SplitSink<WebSocketStream<Stream<TcpStream>>, Message>,
}

struct WsUpstreamStream {
  inner: SplitStream<WebSocketStream<Stream<TcpStream>>>,
}

#[async_trait]
impl Upstream for WsUpstream {
  type Sink = WsUpstreamSink;
  type Stream = WsUpstreamStream;

  /// A UDP datagram header has a 16 bit field containing an unsigned integer
  /// describing the length of the datagram (including the header itself).
  /// The max value is 2^16 = 65536 bytes. But since that includes the
  /// UDP header, this constant is 8 bytes more than any UDP socket
  /// read operation would ever return. We are going to save that extra space.
  const BUF_SIZE: usize = u16::MAX as usize - mem::size_of::<u16>();
  const BUF_SKIP: usize = 0;

  fn buffer() -> Box<[u8]> {
    Box::new([0_u8; Self::BUF_SIZE])
  }

  async fn open(&self) -> anyhow::Result<(Self::Sink, Self::Stream)> {
    let stream = Stream::connect(&self.url, &self.proxy).await?;
    let (stream, _) = client_async(&self.url, stream).await?;
    let (sink, stream) = stream.split();

    let sink = Self::Sink { inner: sink };
    let stream = Self::Stream { inner: stream };

    Ok((sink, stream))
  }
}

#[async_trait]
impl UpstreamSink for WsUpstreamSink {
  async fn write(&mut self, buf: &mut [u8]) -> anyhow::Result<()> {
    Ok(self.inner.send(Message::Binary(buf.to_vec())).await?)
  }

  async fn close(&mut self) -> anyhow::Result<()> {
    Ok(self.inner.close().await?)
  }
}

#[async_trait]
impl UpstreamStream for WsUpstreamStream {
  async fn connect(&mut self, socket: &UdpSocket, addr: SocketAddr) -> anyhow::Result<()> {
    loop {
      match self.inner.next().await {
        Some(Ok(message)) => {
          socket.send_to(&message.into_data(), addr).await?;
        }
        Some(err @ Err(_)) => {
          err?;
        }
        None => {}
      }
    }
  }
}
