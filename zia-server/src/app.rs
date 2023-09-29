use std::convert::Infallible;
use std::future::Future;
use std::mem;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::str::FromStr;
use std::sync::Arc;
use fastwebsockets::{Frame, OpCode, Payload, Role, WebSocket, WebSocketRead, WebSocketWrite};
use hyper::{Body, Request};
use hyper::header::{
    CONNECTION, HOST, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_VERSION, UPGRADE, USER_AGENT,
};
use hyper::upgrade::Upgraded;
use tokio::io::{ReadHalf, split, WriteHalf};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{Mutex as TokioMutex, Mutex};

struct Connection {
    finished: Arc<Mutex<bool>>,
    buf: Arc<Mutex<Box<[u8; MAX_DATAGRAM_SIZE]>>>,
    write: Arc<Mutex<WebSocketWrite<WriteHalf<Upgraded>>>>,
}

const UDP_ADDR: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(10,99,0,0), 51838));

async fn test(_: Frame<'_>) -> Result<(), Infallible> {
    todo!()
}

async fn handle(
    socket: TcpStream,
) -> anyhow::Result<WebSocketWrite<WriteHalf<TcpStream>>> {
    let mut ws = WebSocket::after_handshake(socket, Role::Server);
    ws.set_writev(false);
    ws.set_auto_close(true);
    ws.set_auto_pong(true);

    let (mut ws_read, write) = ws.split(|upgraded| split(upgraded));

    tokio::spawn(async move {
        loop {
            let frame = ws_read.read_frame(&mut test).await.unwrap();

            if !frame.fin {
                println!("unexpected buffer received, expect full udp frame to be in one websocket message");
                continue;
            }

            match frame.opcode {
                OpCode::Binary => match frame.payload {
                    Payload::BorrowedMut(payload) => {
                        socket.send_to(payload, UDP_ADDR).await.unwrap();
                    }
                    Payload::Borrowed(payload) => {
                        socket.send_to(payload, UDP_ADDR).await.unwrap();
                    }
                    Payload::Owned(payload) => {
                        socket.send_to(&payload, UDP_ADDR).await.unwrap();
                    }
                },
                opcode => eprintln!("Unexpected opcode: {:?}", opcode),
            }
        }
    });

    Ok(write)
}

impl Connection {
    async fn new() -> anyhow::Result<(Self, WebSocketRead<ReadHalf<Upgraded>>)> {
        // 135.181.77.88:80

        println!("connected https");

        let (ws, resp) = fastwebsockets::upgrade::upgrade(&SpawnExecutor, req, listener).await?;

        println!("connected websocket");

        let (ws_read, ws_write) = ws.split(|upgraded| split(upgraded));

        Ok((
            Self {
                finished: Arc::new(Mutex::new(true)),
                buf: Arc::new(Mutex::new(datagram_buffer())),
                write: Arc::new(Mutex::new(ws_write)),
            },
            ws_read,
        ))
    }
}

struct WritePool {
    connections: Vec<Connection>,
    next: usize,
}

impl WritePool {
    async fn write(
        &mut self,
        socket: &UdpSocket,
        old_addr: Arc<TokioMutex<Option<SocketAddr>>>,
    ) -> anyhow::Result<()> {
        loop {
            let connection = self
                .connections
                .get(self.next % self.connections.len())
                .unwrap();

            self.next += 1;

            let mut finished = connection.finished.lock().await;
            if *finished {
                *finished = false;

                let mut buf = connection.buf.lock().await;
                let (read, addr) = socket.recv_from(&mut buf[..]).await.unwrap();
                tokio::spawn(async move {
                    *(old_addr.lock().await) = Some(addr);
                });

                let buf = connection.buf.clone();
                let write = connection.write.clone();

                tokio::spawn(async move {
                    let mut buf = buf.lock().await;

                    write
                        .lock()
                        .await
                        .write_frame(Frame::new(
                            true,
                            OpCode::Binary,
                            None,
                            Payload::BorrowedMut(&mut buf[..read]),
                        ))
                        .await
                        .unwrap();
                });
                return Ok(());
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket = Arc::new(UdpSocket::bind("0.0.0.0:0").await?);
    let mut listener = TcpListener::bind("127.0.0.1:3128").await?;

    let mut connections = Arc::new(Mutex::new(Vec::new()));

    println!("connected");


    {
        let connections = connections.clone();
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await?;
                connections.lock().await.push(handle(stream).await?);
            }
        });
    }

    loop {
        connections.lock().
    }
}

// Tie hyper's executor to tokio runtime
struct SpawnExecutor;

impl<Fut> hyper::rt::Executor<Fut> for SpawnExecutor
    where
        Fut: Future + Send + 'static,
        Fut::Output: Send + 'static,
{
    fn execute(&self, fut: Fut) {
        tokio::task::spawn(fut);
    }
}

const MAX_DATAGRAM_SIZE: usize = u16::MAX as usize - mem::size_of::<u16>();

/// Creates and returns a buffer on the heap with enough space to contain any possible
/// UDP datagram.
///
/// This is put on the heap and in a separate function to avoid the 64k buffer from ending
/// up on the stack and blowing up the size of the futures using it.
#[inline(never)]
fn datagram_buffer() -> Box<[u8; MAX_DATAGRAM_SIZE]> {
    Box::new([0u8; MAX_DATAGRAM_SIZE])
}


