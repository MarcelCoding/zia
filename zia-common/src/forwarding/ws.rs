// see: https://github.com/mullvad/udp-over-tcp/blob/main/src/forward_traffic.rs

use std::convert::Infallible;
use std::mem;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use futures_util::future::select;
use futures_util::pin_mut;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Normal;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tracing::error;

use crate::{maybe_timeout, Stream};

/// A UDP datagram header has a 16 bit field containing an unsigned integer
/// describing the length of the datagram (including the header itself).
/// The max value is 2^16 = 65536 bytes. But since that includes the
/// UDP header, this constant is 8 bytes more than any UDP socket
/// read operation would ever return. We are going to save that extra space.
const MAX_DATAGRAM_SIZE: usize = u16::MAX as usize - mem::size_of::<u16>();

/// Forward traffic between the given UDP and WS sockets in both directions.
/// This async function runs until one of the sockets are closed or there is an error.
/// Both sockets are closed before returning.
pub async fn process_udp_over_ws(
  udp_socket: UdpSocket,
  ws_stream: WebSocketStream<Stream<TcpStream>>,
  ws_recv_timeout: Option<Duration>,
) {
  let (mut ws_out, mut ws_in) = ws_stream.split();

  {
    let udp_in = Arc::new(udp_socket);
    let udp_out = udp_in.clone();

    let ws2udp = async {
      if let Err(error) = process_ws2udp(&mut ws_in, udp_out, ws_recv_timeout).await {
        error!("Error: {}", error);
      }
    };

    let udp2ws = async {
      if let Err(error) = process_udp2ws(udp_in, &mut ws_out).await {
        error!("Error: {}", error);
      }
    };

    pin_mut!(ws2udp);
    pin_mut!(udp2ws);

    // Wait until the UDP->WS or WS->UDP future terminates.
    select(ws2udp, udp2ws).await;
  }

  if let Err(err) = ws_in
    .reunite(ws_out)
    .unwrap()
    .close(Some(CloseFrame {
      code: Normal,
      reason: "Timeout".into(),
    }))
    .await
  {
    error!("Unable to close ws stream: {}", err);
  }
}

/// Reads from `ws_in` and extracts UDP datagrams. Writes the datagrams to `udp_out`.
/// Returns if the WS socket is closed, or an IO error happens on either socket.
async fn process_ws2udp(
  ws_in: &mut SplitStream<WebSocketStream<Stream<TcpStream>>>,
  udp_out: Arc<UdpSocket>,
  ws_recv_timeout: Option<Duration>,
) -> anyhow::Result<()> {
  while let Some(message) = maybe_timeout(ws_recv_timeout, ws_in.next())
    .await
    .context("Timeout while reading from WS")?
    .transpose()
    .context("Failed reading from WS")?
  {
    let data = message.into_data();

    forward_datagrams_in_buffer(&udp_out, &data)
      .await
      .context("Failed writing to UDP")?;
  }

  Ok(())
}

/// Forward the datagram in `buffer` to `udp_out`.
/// Returns the number of processed bytes.
async fn forward_datagrams_in_buffer(udp_out: &UdpSocket, buffer: &Vec<u8>) -> anyhow::Result<()> {
  let udp_write_len = udp_out.send(buffer).await?;
  assert_eq!(
    udp_write_len,
    buffer.len(),
    "Did not send entire UDP datagram"
  );

  Ok(())
}

/// Reads datagrams from `udp_in` and writes them (with the 16 bit header containing the length)
/// to `ws_out` indefinitely, or until an IO error happens on either socket.
async fn process_udp2ws(
  udp_in: Arc<UdpSocket>,
  ws_out: &mut SplitSink<WebSocketStream<Stream<TcpStream>>, Message>,
) -> anyhow::Result<Infallible> {
  // A buffer large enough to hold any possible UDP datagram plus its 16 bit length header.
  let mut buffer = datagram_buffer();

  loop {
    let udp_read_len = udp_in
      .recv(&mut buffer[..])
      .await
      .context("Failed reading from UDP")?;

    ws_out
      .send(Message::Binary(buffer[..udp_read_len].to_vec()))
      .await
      .context("Failed writing to WS")?;
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
