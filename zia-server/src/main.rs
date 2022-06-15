use std::{env, io::Error};

use tokio::net::{TcpListener, TcpStream};

#[tokio::main]
async fn main() -> Result<(), Error> {
  let addr = env::args().nth(1).unwrap_or_else(|| "127.0.0.1:8080".to_string());

  // Create the event loop and TCP listener we'll accept connections on.
  let try_socket = TcpListener::bind(&addr).await;
  let listener = try_socket.expect("Failed to bind");
  println!("Listening on: {}", addr);

  while let Ok((stream, _)) = listener.accept().await {
    tokio::spawn(accept_connection(stream));
  }

  Ok(())
}

async fn accept_connection(stream: TcpStream) {
  let addr = stream.peer_addr().expect("connected streams should have a peer address");
  println!("Peer address: {}", addr);

  let ws_stream = tokio_tungstenite::accept_async(stream)
    .await
    .expect("Error during the websocket handshake occurred");


  println!("New WebSocket connection: {}", addr);

  let (write, read) = ws_stream.split();

  read.

  // We should not forward messages other than text or binary.
  // read.try_filter(|msg| future::ready(msg.is_text() || msg.is_binary()))
  //   .forward(write)
  //   .await
  //   .expect("Failed to forward messages")
}
