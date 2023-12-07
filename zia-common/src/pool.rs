use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::sync::{mpsc, Mutex};

pub trait PoolEntry {
  fn is_closed(&self) -> bool;
}

pub struct PoolGuard<T: PoolEntry> {
  inner: Option<T>,
  back: mpsc::UnboundedSender<T>,
  pool_size: Arc<AtomicUsize>,
}

unsafe impl<T: Send + PoolEntry> Send for PoolGuard<T> {}

impl<T: PoolEntry> Drop for PoolGuard<T> {
  fn drop(&mut self) {
    if let Some(inner) = self.inner.take() {
      if inner.is_closed() {
        self.pool_size.fetch_sub(1, Ordering::Relaxed);
      } else if let Err(err) = self.back.send(inner) {
        panic!("Could not put PoolGuard back to pool: {:?}", err);
      }
    }
  }
}

impl<T: PoolEntry> Deref for PoolGuard<T> {
  type Target = T;
  fn deref(&self) -> &Self::Target {
    self.inner.as_ref().unwrap()
  }
}

impl<T: PoolEntry> DerefMut for PoolGuard<T> {
  fn deref_mut(&mut self) -> &mut Self::Target {
    self.inner.as_mut().unwrap()
  }
}

pub struct Pool<T: PoolEntry> {
  size: Arc<AtomicUsize>,
  tx: mpsc::UnboundedSender<T>,
  rx: Mutex<mpsc::UnboundedReceiver<T>>,
}

impl<T: PoolEntry> Default for Pool<T> {
  fn default() -> Self {
    let (tx, rx) = mpsc::unbounded_channel();
    Self {
      size: Arc::new(AtomicUsize::new(0)),
      tx,
      rx: Mutex::new(rx),
    }
  }
}

impl<T: PoolEntry> Pool<T> {
  pub async fn acquire(&self) -> Option<PoolGuard<T>> {
    if self.size.load(Ordering::Relaxed) == 0 {
      return None;
    }

    let inner = self.rx.lock().await.recv().await.unwrap();

    Some(PoolGuard {
      inner: Some(inner),
      back: self.tx.clone(),
      pool_size: self.size.clone(),
    })
  }
}

impl<T: PoolEntry> Pool<T> {
  pub fn push(&self, inner: T) {
    self.size.fetch_add(1, Ordering::Relaxed);
    if let Err(err) = self.tx.send(inner) {
      panic!("Could not put Inner into to pool: {:?}", err);
    }
  }
}
