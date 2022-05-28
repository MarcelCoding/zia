use tokio::net::{TcpStream, ToSocketAddrs};

use crate::upstream::{Connection, Upstream};

pub struct WsUpstream {}

#[async_trait::async_trait]
impl Upstream for WsUpstream {
  type Conn = WsConnection;

  async fn connect<A: ToSocketAddrs + Send>(&self, addr: A) -> anyhow::Result<Self::Conn> {
    todo!()
  }
}

pub struct WsConnection {}

#[async_trait::async_trait]
impl Connection for WsConnection {
  async fn mount(mut self, inbound: TcpStream) -> anyhow::Result<()> {
    todo!()
  }
}
