use tokio::net::UdpSocket;

pub(crate) use self::ws::*;

mod ws;

#[async_trait::async_trait]
pub(crate) trait Upstream {
  type Conn: Connection;

  async fn connect(&self) -> anyhow::Result<Self::Conn>;
}

#[async_trait::async_trait]
pub(crate) trait Connection {
  async fn mount(mut self, inbound: UdpSocket) -> anyhow::Result<()>;
}
