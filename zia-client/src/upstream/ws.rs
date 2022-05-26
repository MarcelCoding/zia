use crate::upstream::{Connection, Upstream};
use std::net::SocketAddr;

pub struct WsUpstream {}

impl Upstream for WsUpstream {
  type Conn = WsConnection;

  fn connect(&self, addr: SocketAddr) -> anyhow::Result<Self::Conn> {
    todo!()
  }
}

pub struct WsConnection {}

impl Connection for WsConnection {
  fn mount() -> anyhow::Result<()> {
    todo!()
  }
}
