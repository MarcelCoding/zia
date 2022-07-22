use std::time::Duration;

use tokio::net::UdpSocket;
use url::Url;

use zia_common::{process_udp_over_tcp, Stream};

pub(crate) async fn transmit(
  socket: UdpSocket,
  upstream: &Url,
  proxy: &Option<Url>,
) -> anyhow::Result<()> {
  let upstream = Stream::connect(upstream, proxy).await?;

  {
    let mut tmp_buffer = [0; u16::MAX as usize];
    let (_udp_read_len, udp_peer_addr) = socket.peek_from(tmp_buffer.as_mut()).await?;

    // Connect the UDP socket to whoever sent the first datagram. This is where
    // all the returned traffic will be sent to.
    socket.connect(udp_peer_addr).await?;
  };

  process_udp_over_tcp(socket, upstream, Some(Duration::from_secs(60))).await;

  Ok(())
}
