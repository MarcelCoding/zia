use tokio::net::TcpStream;

pub use self::direct::*;
pub use self::ws::*;

mod direct;
mod ws;

#[async_trait::async_trait]
pub trait Upstream {
  type Conn: Connection;

  async fn connect(&self, addr: &str) -> anyhow::Result<Self::Conn>;
}

#[async_trait::async_trait]
pub trait Connection {
  async fn mount(mut self, inbound: TcpStream) -> anyhow::Result<()>;
}
