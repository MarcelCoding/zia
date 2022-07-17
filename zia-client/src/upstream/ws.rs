use std::net::SocketAddr;

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::RwLock;
use tokio::try_join;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tracing::warn;
use url::Url;

use crate::upstream::{Connection, Upstream};

pub(crate) struct WsUpstream {
  pub(crate) url: Url,
}

#[async_trait::async_trait]
impl Upstream for WsUpstream {
  type Conn = WsConnection;

  async fn connect(&self) -> anyhow::Result<Self::Conn> {
    let (outbound, _) = connect_async(&self.url).await?;
    Ok(WsConnection {
      inner: outbound,
      remote_addr: RwLock::new(None),
    })
  }
}

pub(crate) struct WsConnection {
  inner: WebSocketStream<MaybeTlsStream<TcpStream>>,
  remote_addr: RwLock<Option<SocketAddr>>,
}

#[async_trait::async_trait]
impl Connection for WsConnection {
  async fn mount(mut self, inbound: UdpSocket) -> anyhow::Result<()> {
    let (mut wo, mut ro) = self.inner.split();

    self.remote_addr = RwLock::new(None);

    let client_to_server = async {
      // TODO: can this be abused -> receive endless data from the other udp side
      let mut buf = vec![0; 65507];

      let (mut read, mut new_addr) = inbound.recv_from(&mut buf[..]).await?;
      *self.remote_addr.write().await = Some(new_addr);

      while read > 0 {
        wo.send(Message::binary(&buf[0..read])).await?;
        (read, new_addr) = inbound.recv_from(&mut buf[..]).await?;

        if Some(true) == self.remote_addr.read().await.map(|addr| addr != new_addr) {
          *self.remote_addr.write().await = Some(new_addr);
        }
      }

      wo.close().await?;
      Ok::<_, anyhow::Error>(())
    };

    let server_to_client = async {
      while let Some(next) = ro.next().await {
        let msg = next?;

        if let Some(remote_addr) = *self.remote_addr.read().await {
          inbound.send_to(&msg.into_data(), remote_addr).await?;
        } else {
          warn!(
            "Dropped a websocket message of length {} because no downstream client is registered.",
            msg.len()
          );
        }
      }

      Ok::<_, anyhow::Error>(())
    };

    try_join!(client_to_server, server_to_client)?;

    // TODO: close

    Ok(())
  }
}
