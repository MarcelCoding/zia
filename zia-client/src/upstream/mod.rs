use tokio::net::TcpStream;

pub(crate) use self::direct::*;
pub(crate) use self::ws::*;

mod direct;
mod ws;

#[async_trait::async_trait]
pub(crate) trait Upstream {
  type Conn: Connection;

  async fn connect(&self, addr: &str) -> anyhow::Result<Self::Conn>;
}

#[async_trait::async_trait]
pub(crate) trait Connection {
  async fn mount(mut self, inbound: TcpStream) -> anyhow::Result<()>;
}
