use futures_util::sink::SinkExt;
use futures_util::StreamExt;
use tokio::{io, try_join};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tokio_tungstenite::tungstenite::Message;
use url::Url;

use crate::upstream::{Connection, Upstream};

pub struct WsUpstream {
  upstream: Url,
}

#[async_trait::async_trait]
impl Upstream for WsUpstream {
  type Conn = WsConnection;

  async fn connect(&self, addr: &str) -> anyhow::Result<Self::Conn> {
    let (mut outbound, _) = connect_async(&self.upstream).await?;

    outbound.send(Message::Text(addr.to_string())).await?;

    Ok(WsConnection { outbound })
  }
}

pub struct WsConnection {
  outbound: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

#[async_trait::async_trait]
impl Connection for WsConnection {
  async fn mount(mut self, mut inbound: TcpStream) -> anyhow::Result<()> {
    let (mut ri, mut wi) = inbound.split();
    let (mut ro, mut wo) = self.outbound.split();

    let client_to_server = async {
      io::copy(&mut ri, &mut wo).await?;
      wo.shutdown().await
    };

    let server_to_client = async {
      io::copy(&mut ro, &mut wi).await?;
      wi.shutdown().await
    };

    try_join!(client_to_server, server_to_client)?;

    Ok(())
  }
}
