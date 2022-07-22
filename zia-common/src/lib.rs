use std::future::Future;
use std::time::Duration;

use tokio::time::error::Elapsed;
use tokio::time::timeout;

pub use crate::forwarding::*;
pub use crate::stream::*;

mod forwarding;
mod stream;

pub async fn maybe_timeout<F: Future>(
  duration: Option<Duration>,
  future: F,
) -> Result<F::Output, Elapsed> {
  match duration {
    Some(duration) => timeout(duration, future).await,
    None => Ok(future.await),
  }
}
