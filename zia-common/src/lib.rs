pub use read::*;
use std::mem;
pub use write::*;

mod pool;
mod read;
mod write;
pub mod ws;

pub const MAX_DATAGRAM_SIZE: usize = u16::MAX as usize - mem::size_of::<u16>();
