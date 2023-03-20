use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::upstream::{Upstream, UpstreamSink, UpstreamStream};

pub(crate) struct UdpHandler<U: Upstream> {
  upstream: U,
  known: Arc<Mutex<HashMap<SocketAddr, U::Sink>>>,
}

impl<U: Upstream + Send> UdpHandler<U> {
  pub(crate) fn new(upstream: U) -> Self {
    UdpHandler {
      upstream,
      known: Arc::new(Mutex::new(HashMap::new())),
    }
  }

  pub(crate) async fn listen(self, socket: Arc<UdpSocket>) -> anyhow::Result<()>
  where
    <U as Upstream>::Stream: Send + 'static,
    <U as Upstream>::Sink: Send + 'static,
  {
    let mut buf = U::buffer();

    loop {
      let (read, addr) = socket.recv_from(&mut buf[U::BUF_SKIP..]).await?;

      let mut known = self.known.lock().await;
      let mut upstream = match known.entry(addr) {
        Entry::Occupied(occupied) => occupied,
        Entry::Vacant(vacant) => {
          info!("New socket at {}/udp, opening upstream connection...", addr);

          let (sink, mut stream) = match self.upstream.open().await {
            Ok(conn) => conn,
            Err(err) => {
              warn!("Error while opening upstream connection: {err}");
              continue;
            }
          };

          let known = self.known.clone();
          let socket = socket.clone();

          tokio::spawn(async move {
            if let Err(err) = stream.connect(&socket, addr).await {
              warn!(
                "Unable to read from upstream or write to udp socket: {}",
                err
              );
            }

            if let Some(mut sink) = known.lock().await.remove(&addr) {
              if let Err(err) = sink.close().await {
                error!("Unable to close upstream sink, closing...: {}", err);
              }
            }
          });

          vacant.insert_entry(sink)
        }
      };

      let sink = upstream.get_mut();
      if let Err(err) = sink.write(&mut buf[..U::BUF_SKIP + read]).await {
        warn!("Unable to write to upstream, closing...: {}", err);

        if let Err(err) = upstream.remove().close().await {
          error!("Unable to close upstream sink: {}", err);
        }
      };
    }
  }
}
