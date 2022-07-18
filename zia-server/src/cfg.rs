use std::net::SocketAddr;

use config::{Config, Environment};
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct ClientCfg {
  pub(crate) listen_addr: SocketAddr,
  pub(crate) upstream: SocketAddr,
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
