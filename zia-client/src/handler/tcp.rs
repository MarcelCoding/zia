use crate::handler::{ConnectionManager, Upstream};
use crate::tls::{tls, MaybeTlsStream};
use anyhow::anyhow;
use async_http_proxy::{http_connect_tokio, http_connect_tokio_with_basic_auth};
use tokio::io::{split, BufStream, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tracing::info;
use zia_common::{ReadTcpConnection, WriteTcpConnection};

pub(crate) struct TcpConnectionManager;

#[async_trait::async_trait]
impl ConnectionManager for TcpConnectionManager {
  type ReadConnection = ReadTcpConnection<ReadHalf<MaybeTlsStream<BufStream<TcpStream>>>>;
  type WriteConnection = WriteTcpConnection<WriteHalf<MaybeTlsStream<BufStream<TcpStream>>>>;

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

    let stream = if upstream.tls {
      let stream = MaybeTlsStream::Tls {
        io: tls(stream, &upstream.host).await?,
      };
      info!("Upgraded to tls");
      stream
    } else {
      MaybeTlsStream::Raw { io: stream }
    };

    info!("Finished websocket handshake");

    let (read, write) = split(stream);

    Ok((ReadTcpConnection::new(read), WriteTcpConnection::new(write)))
  }
}
