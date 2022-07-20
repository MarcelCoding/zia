use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::net::UdpSocket;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::try_join;
use tokio_tungstenite::client_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::warn;
use url::Url;
use zia_common::Stream;

pub(crate) async fn transmit(
  socket: UdpSocket,
  upstream: &Url,
  proxy: &Option<Url>,
) -> anyhow::Result<()> {
  let stream = Stream::connect(upstream, proxy).await?;

  // websocket handshake
  let (upstream, _) = client_async(upstream, stream).await?;
  let (mut wu, mut ru) = upstream.split();

  // udp remote address -> no active connection
  let remote_addr = RwLock::new(None);

  let socket_a = Arc::new(socket);
  let socket_b = socket_a.clone();

  let remote_addr_a = Arc::new(remote_addr);
  let remote_addr_b = remote_addr_a.clone();

  let client_to_server = async move {
    // TODO: can this be abused -> receive endless data from the other udp side
    let mut buf = vec![0; 65507];
    let (mut read, mut new_addr) = socket_a.recv_from(&mut buf).await?;
    *remote_addr_a.write().await = Some(new_addr);
    while read != 0 {
      wu.send(Message::binary(&buf[..read])).await?;
      (read, new_addr) = socket_a.recv_from(&mut buf).await?;
      if remote_addr_a
        .read()
        .await
        .map(|addr| addr != new_addr)
        .unwrap_or(false)
      {
        *remote_addr_a.write().await = Some(new_addr);
      }
    }
    wu.close().await?;
    Ok::<_, anyhow::Error>(())
  };

  let server_to_client = async move {
    while let Some(next) = ru.next().await {
      let msg = next?;
      if let Some(remote_addr) = *remote_addr_b.read().await {
        socket_b.send_to(&msg.into_data(), remote_addr).await?;
      } else {
        warn!(
          "Dropped a websocket message of length {} because no downstream client is registered.",
          msg.len()
        );
      }
    }
    Ok::<_, anyhow::Error>(())
  };

  try_join!(
    flat(tokio::spawn(client_to_server)),
    flat(tokio::spawn(server_to_client)),
  )?;

  Ok(())
}

async fn flat<V>(r: JoinHandle<anyhow::Result<V>>) -> anyhow::Result<V> {
  r.await?
}
