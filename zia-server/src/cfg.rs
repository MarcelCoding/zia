use std::fmt::{Display, Formatter};
use std::net::SocketAddr;

use config::{Config, Environment};
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct ClientCfg {
  pub(crate) listen_addr: SocketAddr,
  pub(crate) upstream: SocketAddr,
  pub(crate) mode: Mode,
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum Mode {
  Ws,
  Tcp,
}

impl ClientCfg {
  pub(crate) fn load() -> anyhow::Result<Self> {
    Ok(
      Config::builder()
        .add_source(Environment::with_prefix("ZIA"))
        .build()?
        .try_deserialize()?,
    )
  }
}

impl Display for Mode {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::Ws => f.write_str("ws"),
      Self::Tcp => f.write_str("tcp"),
    }
  }
}
