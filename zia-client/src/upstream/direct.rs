use tokio::{io, try_join};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use crate::upstream::{Connection, Upstream};

pub struct DirectUpstream {}

#[async_trait::async_trait]
impl Upstream for DirectUpstream {
  type Conn = DirectConnection;

  async fn connect(&self, addr: &str) -> anyhow::Result<Self::Conn> {
    let outbound = TcpStream::connect(addr).await?;
    Ok(DirectConnection { outbound })
  }
}

pub struct DirectConnection {
  outbound: TcpStream,
}

#[async_trait::async_trait]
impl Connection for DirectConnection {
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
