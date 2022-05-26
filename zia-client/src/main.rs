extern crate core;

use std::net::SocketAddr;
use std::str;

use anyhow::anyhow;
use httparse::Request;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::ReadHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::{io, select, try_join};

mod upstream;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  let addr: SocketAddr = "127.0.0.1:8080".parse()?;

  select! {
    result = accept_connections(addr) => {
      result?;
      println!("Socket closed, quitting...");
    },
    result = tokio::signal::ctrl_c() => {
      result?;
      println!("Received ctrl-c, quitting...");
    },
  }

  Ok(())
}

async fn accept_connections(addr: SocketAddr) -> anyhow::Result<()> {
  let listener = TcpListener::bind(addr).await?;
  println!("Listening on {}...", addr);

  loop {
    let (sock, _) = listener.accept().await?;
    tokio::spawn(handle(sock));
  }
}

// \r\n\r\n
const HTTP_HEADER_END: [u8; 4] = [0xd, 0xa, 0xd, 0xa];

// 200 Ok\r\n\r\n
const HTTP_OK: [u8; 10] = [0x32, 0x30, 0x30, 0x20, 0x4f, 0x6b, 0xd, 0xa, 0xd, 0xa];

async fn handle(mut inbound: TcpStream) -> anyhow::Result<()> {
  // inbound
  let (mut ri, mut wi) = inbound.split();

  let dest = read_host(&mut ri).await?;

  // outbound
  let mut outbound = create_outbound(&dest).await?;
  let (mut ro, mut wo) = outbound.split();

  // connection successful
  wi.write_all(&HTTP_OK).await?;

  let client_to_server = async {
    io::copy(&mut ri, &mut wo).await?;
    wo.shutdown().await
  };

  let server_to_client = async {
    io::copy(&mut ro, &mut wi).await?;
    wi.shutdown().await
  };

  try_join!(client_to_server, server_to_client)?;

  println!("- Connection to {} closed...", dest);

  Ok(())
}

async fn create_outbound(dest: &String) -> anyhow::Result<TcpStream> {
  println!("> Connecting to {}...", dest);
  let outbound = TcpStream::connect(dest).await?;
  println!("+ Opened connection to {}...", dest);
  Ok(outbound)
}

async fn read_host(rx: &mut ReadHalf<'_>) -> anyhow::Result<String> {
  let mut buf = Vec::with_capacity(1024);

  // read till end of http header "\r\n\r\n"
  rx.read_buf(&mut buf).await?;
  while !buf.ends_with(&HTTP_HEADER_END) {
    if rx.read_buf(&mut buf).await? == 0 {
      return Err(anyhow!("Socket closed"));
    }
  }

  let mut headers = [httparse::EMPTY_HEADER; 64];
  let mut req = Request::new(&mut headers);

  req.parse(&buf)?;

  let path = req.path.unwrap().to_lowercase();

  for header in req.headers {
    if header.name.to_lowercase() == "host" {
      let host = str::from_utf8(header.value)?;

      if !host.contains(':') {
        if path.starts_with("http://") {
          return Ok(host.to_owned() + ":80");
        } else if path.starts_with("https://") {
          return Ok(host.to_owned() + ":443");
        } else {
          panic!("Missing port");
        }
      }

      return Ok(host.to_owned());
    }
  }

  Err(anyhow!("Unable to extract destination host"))
}
