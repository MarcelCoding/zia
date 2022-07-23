// see: https://github.com/mullvad/udp-over-tcp/blob/main/src/forward_traffic.rs

use std::convert::{Infallible, TryFrom};
use std::mem;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use futures_util::future::select;
use futures_util::pin_mut;
use tokio::io::{split, AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::{TcpStream, UdpSocket};
use tracing::error;

use crate::{maybe_timeout, Stream};

/// A UDP datagram header has a 16 bit field containing an unsigned integer
/// describing the length of the datagram (including the header itself).
/// The max value is 2^16 = 65536 bytes. But since that includes the
/// UDP header, this constant is 8 bytes more than any UDP socket
/// read operation would ever return. We are going to use that extra space
/// to store our 2 byte udp-over-tcp header.
const MAX_DATAGRAM_SIZE: usize = u16::MAX as usize;
const HEADER_LEN: usize = mem::size_of::<u16>();

/// Forward traffic between the given UDP and TCP sockets in both directions.
/// This async function runs until one of the sockets are closed or there is an error.
/// Both sockets are closed before returning.
pub async fn process_udp_over_tcp(
  udp_socket: UdpSocket,
  tcp_stream: Stream<TcpStream>,
  tcp_recv_timeout: Option<Duration>,
) {
  let udp_in = Arc::new(udp_socket);
  let udp_out = udp_in.clone();
  let (tcp_in, tcp_out) = split(tcp_stream);

  let tcp2udp = async move {
    if let Err(error) = process_tcp2udp(tcp_in, udp_out, tcp_recv_timeout).await {
      error!("Error: {}", error);
    }
  };

  let udp2tcp = async move {
    if let Err(error) = process_udp2tcp(udp_in, tcp_out).await {
      error!("Error: {}", error);
    }
  };

  pin_mut!(tcp2udp);
  pin_mut!(udp2tcp);

  // Wait until the UDP->TCP or TCP->UDP future terminates.
  select(tcp2udp, udp2tcp).await;
}

/// Reads from `tcp_in` and extracts UDP datagrams. Writes the datagrams to `udp_out`.
/// Returns if the TCP socket is closed, or an IO error happens on either socket.
async fn process_tcp2udp(
  mut tcp_in: ReadHalf<Stream<TcpStream>>,
  udp_out: Arc<UdpSocket>,
  tcp_recv_timeout: Option<Duration>,
) -> anyhow::Result<()> {
  let mut buffer = datagram_buffer();
  // `buffer` has unprocessed data from the TCP socket up until this index.
  let mut unprocessed_i = 0;
  loop {
    let tcp_read_len = maybe_timeout(tcp_recv_timeout, tcp_in.read(&mut buffer[unprocessed_i..]))
      .await
      .context("Timeout while reading from TCP")?
      .context("Failed reading from TCP")?;
    if tcp_read_len == 0 {
      break;
    }
    unprocessed_i += tcp_read_len;

    let processed_i = forward_datagrams_in_buffer(&udp_out, &buffer[..unprocessed_i])
      .await
      .context("Failed writing to UDP")?;

    // If we have read data that was not forwarded, because it was not a complete datagram,
    // move it to the start of the buffer and start over
    if unprocessed_i > processed_i {
      buffer.copy_within(processed_i..unprocessed_i, 0);
    }
    unprocessed_i -= processed_i;
  }

  Ok(())
}

/// Forward all complete datagrams in `buffer` to `udp_out`.
/// Returns the number of processed bytes.
async fn forward_datagrams_in_buffer(udp_out: &UdpSocket, buffer: &[u8]) -> anyhow::Result<usize> {
  let mut header_start = 0;
  loop {
    let header_end = header_start + HEADER_LEN;
    // "parse" the header
    let header = match buffer.get(header_start..header_end) {
      Some(header) => <[u8; HEADER_LEN]>::try_from(header).unwrap(),
      // Buffer does not contain entire header for next datagram
      None => break Ok(header_start),
    };
    let datagram_len = usize::from(u16::from_be_bytes(header));
    let datagram_start = header_end;
    let datagram_end = datagram_start + datagram_len;

    let datagram_data = match buffer.get(datagram_start..datagram_end) {
      Some(datagram_data) => datagram_data,
      // The buffer does not contain the entire datagram
      None => break Ok(header_start),
    };

    let udp_write_len = udp_out.send(datagram_data).await?;
    assert_eq!(
      udp_write_len, datagram_len,
      "Did not send entire UDP datagram"
    );

    header_start = datagram_end;
  }
}

/// Reads datagrams from `udp_in` and writes them (with the 16 bit header containing the length)
/// to `tcp_out` indefinitely, or until an IO error happens on either socket.
async fn process_udp2tcp(
  udp_in: Arc<UdpSocket>,
  mut tcp_out: WriteHalf<Stream<TcpStream>>,
) -> anyhow::Result<Infallible> {
  // A buffer large enough to hold any possible UDP datagram plus its 16 bit length header.
  let mut buffer = datagram_buffer();
  loop {
    let udp_read_len = udp_in
      .recv(&mut buffer[HEADER_LEN..])
      .await
      .context("Failed reading from UDP")?;

    // Set the "header" to the length of the datagram.
    let datagram_len = u16::try_from(udp_read_len).expect("UDP datagram can't be larger than 2^16");
    buffer[..HEADER_LEN].copy_from_slice(&datagram_len.to_be_bytes()[..]);

    tcp_out
      .write_all(&buffer[..HEADER_LEN + udp_read_len])
      .await
      .context("Failed writing to TCP")?;
  }
}

/// Creates and returns a buffer on the heap with enough space to contain any possible
/// UDP datagram.
///
/// This is put on the heap and in a separate function to avoid the 64k buffer from ending
/// up on the stack and blowing up the size of the futures using it.
#[inline(never)]
fn datagram_buffer() -> Box<[u8; MAX_DATAGRAM_SIZE]> {
  Box::new([0u8; MAX_DATAGRAM_SIZE])
}
