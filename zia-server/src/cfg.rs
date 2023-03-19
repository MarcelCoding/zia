use std::fmt::{Display, Formatter};
use std::net::SocketAddr;

use clap::{Parser, ValueEnum};

#[derive(Parser)]
#[clap(version)]
pub(crate) struct ClientCfg {
  #[arg(short, long, env = "ZIA_LISTEN_ADDR", default_value = "0.0.0.0:1234")]
  pub(crate) listen_addr: SocketAddr,
  #[arg(short, long, env = "ZIA_UPSTREAM")]
  pub(crate) upstream: String,
  #[arg(short, long, env = "ZIA_MODE", default_value = "WS", value_enum)]
  pub(crate) mode: Mode,
}

#[derive(ValueEnum, Clone)]
pub(crate) enum Mode {
  #[value(rename_all = "SCREAMING_SNAKE_CASE")]
  Ws,
  #[value(rename_all = "SCREAMING_SNAKE_CASE")]
  Tcp,
}

impl Display for Mode {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::Ws => f.write_str("ws"),
      Self::Tcp => f.write_str("tcp"),
    }
  }
}
