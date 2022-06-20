use anyhow::anyhow;
use futures_util::{SinkExt, Stream, StreamExt};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio::try_join;
use tokio_rustls::rustls::ClientConfig;
use tokio_rustls::webpki::DnsNameRef;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use url::Url;

use crate::upstream::{Connection, Upstream};

pub(crate) struct WsUpstream {
  pub(crate) url: Url,
}

#[async_trait::async_trait]
impl Upstream for WsUpstream {
  type Conn = WsConnection;

  async fn connect(&self, addr: &str) -> anyhow::Result<Self::Conn> {
    let (mut outbound, _) = connect_async(&self.url).await?;

    // let mut config = ClientConfig::new();
    // config.root_store.add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);
    // let config = TlsConnector::from(Arc::new(config));
    // let dnsname = DnsNameRef::try_from_ascii_str("www.rust-lang.org").unwrap();

    // config.connect(dnsname, outbound);

    outbound.send(Message::text(addr)).await?;

    Ok(WsConnection { outbound })

    // 3, 8, 13
  }
}

pub(crate) struct WsConnection {
  outbound: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

#[async_trait::async_trait]
impl Connection for WsConnection {
  async fn mount(mut self, mut inbound: TcpStream) -> anyhow::Result<()> {
    let (mut ri, mut wi) = inbound.split();
    let (mut wo, mut ro) = self.outbound.split();

    let client_to_server = async {
      let mut buf = [0; 16384];

      let mut read = ri.read(&mut buf[..]).await?;
      while read > 0 {
        wo.send(Message::binary(&buf[0..read])).await?;
        read = ri.read(&mut buf[..]).await?;
      }

      wo.close().await?;
      Ok::<_, anyhow::Error>(())
    };

    let server_to_client = async {
      while let Some(next) = ro.next().await {
        wi.write_all(&next?.into_data()).await?;
      }

      wi.shutdown().await?;
      Ok::<_, anyhow::Error>(())
    };

    try_join!(client_to_server, server_to_client)?;

    // TODO: close

    Ok(())
  }
}

// struct WsWrapper {
//   stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
//   write: Option<futures_util::sink::Feed<>>
// }

// impl AsyncRead for WsWrapper {
//   fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
//     match self.stream.poll_next(ctx) {
//       Poll::Ready(Some(Ok(message))) => {
//         buf.put_slice(&message.into_data());
//         return Poll::Ready(Ok(()));
//       }
//       Poll::Ready(Some(Err(err))) => Poll::Ready(Err()),
//       Poll::Ready(None) => Poll::Pending,
//       Poll::Pending => Poll::Pending,
//     }
//   }
// }
//
// impl AsyncWrite for WsWrapper {
//   fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize, std::io::Error>> {
//     self.stream.send()
//   }
//
//   fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
//     todo!()
//   }
//
//   fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
//     todo!()
//   }
// }
