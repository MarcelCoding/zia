use std::net::SocketAddr;

pub use self::ws::*;

mod ws;

pub trait Upstream {
  type Conn: Connection;

  fn connect(&self, addr: SocketAddr) -> anyhow::Result<Self::Conn>;
}

pub trait Connection {
  // todo:
  fn mount() -> anyhow::Result<()>;
}
