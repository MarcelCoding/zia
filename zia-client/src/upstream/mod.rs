use tokio::net::{TcpStream, ToSocketAddrs};

pub use self::direct::*;
pub use self::ws::*;

mod direct;
mod ws;

#[async_trait::async_trait]
pub trait Upstream {
  type Conn: Connection;

  async fn connect<A: ToSocketAddrs + Send>(&self, addr: A) -> anyhow::Result<Self::Conn>;
}

#[async_trait::async_trait]
pub trait Connection {
  // todo:
  async fn mount(mut self, inbound: TcpStream) -> anyhow::Result<()>;
}
