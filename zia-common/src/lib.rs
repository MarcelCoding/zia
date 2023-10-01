pub use read::*;
use std::mem;
pub use write::*;

mod pool;
mod read;
mod write;
pub mod ws;

pub const MAX_DATAGRAM_SIZE: usize = u16::MAX as usize - mem::size_of::<u16>();

/// Creates  and returns a buffer on the heap with enough space to contain any possible
/// UDP datagram.
///
/// This is put on the heap and in a separate function to avoid the 64k buffer from ending
/// up on the stack and blowing up the size of the futures using it.
#[inline(never)]
pub fn datagram_buffer() -> Box<[u8; MAX_DATAGRAM_SIZE]> {
  Box::new([0u8; MAX_DATAGRAM_SIZE])
}
