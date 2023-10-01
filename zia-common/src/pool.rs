use std::ops::{Deref, DerefMut};

use tokio::sync::{mpsc, Mutex};

pub struct PoolGuard<T> {
  inner: Option<T>,
  back: mpsc::UnboundedSender<T>,
}

unsafe impl<T: Send> Send for PoolGuard<T> {}

impl<T> Drop for PoolGuard<T> {
  fn drop(&mut self) {
    if let Err(err) = self.back.send(self.inner.take().unwrap()) {
      panic!("Could not put PoolGuard back to pool: {:?}", err);
    }
  }
}

impl<T> Deref for PoolGuard<T> {
  type Target = T;
  fn deref(&self) -> &Self::Target {
    self.inner.as_ref().unwrap()
  }
}

impl<T> DerefMut for PoolGuard<T> {
  fn deref_mut(&mut self) -> &mut Self::Target {
    self.inner.as_mut().unwrap()
  }
}

pub struct Pool<T> {
  tx: mpsc::UnboundedSender<T>,
  rx: Mutex<mpsc::UnboundedReceiver<T>>,
}

impl<T> Pool<T> {
  pub fn new() -> Self {
    let (tx, rx) = mpsc::unbounded_channel();
    Self {
      tx,
      rx: Mutex::new(rx),
    }
  }

  pub async fn acquire(&self) -> PoolGuard<T> {
    let inner = self.rx.lock().await.recv().await.unwrap();

    PoolGuard {
      inner: Some(inner),
      back: self.tx.clone(),
    }
  }

  pub fn push(&self, inner: T) {
    if let Err(err) = self.tx.send(inner) {
      panic!("Could not put Inner into to pool: {:?}", err);
    }
  }
}
