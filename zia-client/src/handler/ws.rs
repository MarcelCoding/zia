use crate::handler::{ConnectionManager, Upstream};
use crate::tls::tls;
use anyhow::anyhow;
use async_http_proxy::{http_connect_tokio, http_connect_tokio_with_basic_auth};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use tokio::io::{BufStream, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tracing::info;
use wsocket::handshake;
use zia_common::{ReadWsConnection, WriteWsConnection, MAX_DATAGRAM_SIZE};

pub struct WsConnectionManager;

#[async_trait::async_trait]
impl ConnectionManager for WsConnectionManager {
  type ReadConnection = ReadWsConnection<ReadHalf<TokioIo<Upgraded>>>;
  type WriteConnection = WriteWsConnection<WriteHalf<TokioIo<Upgraded>>>;

  async fn open_connection(
    upstream: &Upstream,
  ) -> anyhow::Result<(Self::ReadConnection, Self::WriteConnection)> {
    let stream = match &upstream.proxy {
      None => {
        let stream = TcpStream::connect((upstream.host.as_str(), upstream.port)).await?;
        stream.set_nodelay(true)?;

        info!("Connected to tcp");

        stream
      }
      Some(proxy) => {
        assert_eq!(proxy.scheme(), "http");

        let proxy_host = proxy
          .host_str()
          .ok_or_else(|| anyhow!("Proxy url is missing host"))?;
        let proxy_port = proxy
          .port_or_known_default()
          .ok_or_else(|| anyhow!("Proxy url is missing port"))?;

        let mut stream = TcpStream::connect((proxy_host, proxy_port)).await?;
        stream.set_nodelay(true)?;

        info!("Connected to tcp");

        match proxy.password() {
          Some(password) => {
            http_connect_tokio_with_basic_auth(
              &mut stream,
              &upstream.host,
              upstream.port,
              proxy.username(),
              password,
            )
            .await?
          }
          None => http_connect_tokio(&mut stream, &upstream.host, upstream.port).await?,
        };

        info!("Proxy handshake");

        stream
      }
    };

    let stream = BufStream::new(stream);

    let (ws, _) = if upstream.tls {
      let stream = tls(stream, &upstream.host).await?;

      info!("Upgraded to tls");

      handshake(
        stream,
        &upstream.uri,
        &upstream.host,
        upstream.port,
        "zia",
        MAX_DATAGRAM_SIZE,
        upstream.ws_masking,
      )
      .await?
    } else {
      handshake(
        stream,
        &upstream.uri,
        &upstream.host,
        upstream.port,
        "zia",
        MAX_DATAGRAM_SIZE,
        upstream.ws_masking,
      )
      .await?
    };

    info!("Finished websocket handshake");

    let (read, write) = ws.split();

    Ok((ReadWsConnection::new(read), WriteWsConnection::new(write)))
  }
}
