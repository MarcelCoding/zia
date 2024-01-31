use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use hyper::Uri;
use tokio::net::UdpSocket;
use tokio::select;
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info};
use url::Url;

pub(crate) use tcp::TcpConnectionManager;
pub(crate) use ws::WsConnectionManager;
use zia_common::{PoolEntry, ReadConnection, ReadPool, WriteConnection, WritePool};

mod tcp;
mod ws;

pub struct Upstream {
  uri: Uri,
  host: String,
  port: u16,
  tls: bool,
  proxy: Option<Url>,
  ws_masking: bool,
}

pub(crate) struct Handler<W: ConnectionManager> {
  upstream: Arc<Upstream>,
  write_pool: Arc<WritePool<W::WriteConnection>>,
  read_pool: Arc<ReadPool>,
}

impl<C: ConnectionManager> Handler<C> {
  pub(crate) fn new(
    socket: UdpSocket,
    upstream: Url,
    proxy: Option<Url>,
    ws_masking: bool,
  ) -> anyhow::Result<Self> {
    let socket = Arc::new(socket);
    let addr = Arc::new(RwLock::new(None));

    let write_pool = Arc::new(WritePool::new(socket.clone(), addr.clone()));
    let read_pool = Arc::new(ReadPool::new(socket, addr));

    let upstream_host = upstream
      .host_str()
      .ok_or_else(|| anyhow!("Upstream url is missing host"))?
      .to_string();
    let upstream_port = upstream
      .port_or_known_default()
      .ok_or_else(|| anyhow!("Upstream url is missing port"))?;

    Ok(Self {
      upstream: Arc::new(Upstream {
        uri: upstream.to_string().parse().unwrap(),
        host: upstream_host,
        port: upstream_port,
        tls: upstream.scheme() == "wss",
        proxy,
        ws_masking,
      }),
      write_pool,
      read_pool,
    })
  }

  pub(crate) async fn run(&self, conn_count: usize) -> anyhow::Result<()> {
    let (dead_conn_tx, mut dead_conn_rx) = mpsc::channel(128);

    let conn_backpressure = async {
      let mut missing = conn_count;
      while missing > 0 || dead_conn_rx.recv().await.is_some() {
        missing = missing.saturating_sub(1);

        match C::open_connection(&self.upstream).await {
          Ok((read, write)) => {
            self.read_pool.push(read).await;
            self.write_pool.push(write).await;
          }
          Err(err) => {
            error!("Unable to open connection, trying again in 1s: {err:?}");
            missing += 1;
            tokio::time::sleep(Duration::from_secs(1)).await;
          }
        };
      }
    };

    let write_pool = self.write_pool.clone();
    let read_pool = self.read_pool.clone();

    let write = tokio::spawn(async move { write_pool.join().await });

    select! {
      _ = conn_backpressure => info!("Conn Backpressure finished?"),
      result = write => {
        info!("Write pool finished");
        result?;
      },
      result = read_pool.join(Some(&dead_conn_tx)) => {
        info!("Read pool finished");
        result?;
      },
    }
    Ok(())
  }
}

// Tie hyper's executor to tokio runtime
struct SpawnExecutor;

impl<Fut> hyper::rt::Executor<Fut> for SpawnExecutor
where
  Fut: Future + Send + 'static,
  Fut::Output: Send + 'static,
{
  fn execute(&self, fut: Fut) {
    tokio::task::spawn(fut);
  }
}

#[async_trait::async_trait]
pub trait ConnectionManager {
  type ReadConnection: ReadConnection + Send + 'static;
  type WriteConnection: WriteConnection + PoolEntry + Send + 'static;

  async fn open_connection(
    upstream: &Upstream,
  ) -> anyhow::Result<(Self::ReadConnection, Self::WriteConnection)>;
}
