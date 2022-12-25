pub(crate) use self::tcp::*;
pub(crate) use self::ws::*;

mod tcp;
mod ws;

#[async_trait::async_trait]
pub(crate) trait Listener {
  async fn listen(&self, upstream: &str) -> anyhow::Result<()>;
}
