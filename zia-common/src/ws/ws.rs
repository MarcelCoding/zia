use std::sync::Arc;

use tokio::io::{split, AsyncRead, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::sync::RwLock;

use crate::ws::frame::{Frame, OpCode};
use crate::ws::WebsocketError::PayloadTooLarge;
use crate::ws::{CloseCode, Message, Role, WebsocketError};

pub struct WebSocket<IO> {
  io: IO,
  max_payload_len: usize,
  role: Role,
  closed: Arc<RwLock<bool>>,
}

impl<IO> WebSocket<IO> {
  #[inline]
  pub fn new(stream: IO, max_payload_len: usize, role: Role) -> Self {
    Self {
      io: stream,
      max_payload_len,
      role,
      closed: Arc::new(RwLock::new(false)),
    }
  }
}

impl<IO: AsyncWrite + AsyncRead> WebSocket<IO> {
  pub fn split(self) -> (WebSocket<ReadHalf<IO>>, WebSocket<WriteHalf<IO>>) {
    let (read, write) = split(self.io);
    (
      WebSocket {
        io: read,
        max_payload_len: self.max_payload_len,
        role: self.role,
        closed: self.closed.clone(),
      },
      WebSocket {
        io: write,
        max_payload_len: self.max_payload_len,
        role: self.role,
        closed: self.closed,
      },
    )
  }
}

impl<W: Unpin + AsyncWrite> WebSocket<W> {
  pub async fn send(&mut self, message: Message<'_>) -> Result<(), WebsocketError> {
    if *self.closed.read().await {
      return Err(WebsocketError::NotConnected)?;
    }

    let res = match message {
      Message::Binary(data) => {
        let frame = Frame::new(true, OpCode::Binary, data);
        self.send_frame(frame).await
      }
      Message::Close { code, reason } => {
        let buf = encode_close_body(code, reason);
        let frame = Frame::new(true, OpCode::Close, &buf);
        let res = self.send_frame(frame).await;
        *(self.closed.write().await) = true;
        res
      }
    };

    if res.is_err() {
      *(self.closed.write().await) = true;
    }

    res
  }

  async fn send_frame(&mut self, frame: Frame<'_>) -> Result<(), WebsocketError> {
    if frame.data.len() > self.max_payload_len {
      return Err(PayloadTooLarge);
    }

    match self.role {
      Role::Server => frame.write_without_mask(&mut self.io).await?,
      Role::Client { masking } => {
        if masking {
          let mask = rand::random::<u32>().to_ne_bytes();
          frame.write_with_mask(&mut self.io, mask).await?;
        } else {
          frame.write_without_mask(&mut self.io).await?;
        }
      }
    }

    self.io.flush().await?;

    Ok(())
  }

  pub async fn flush(&mut self) -> Result<(), WebsocketError> {
    self.io.flush().await?;
    Ok(())
  }
}

impl<R: Unpin + AsyncRead> WebSocket<R> {
  pub async fn recv<'a>(&mut self, buf: &'a mut [u8]) -> Result<Message<'a>, WebsocketError> {
    if *self.closed.read().await {
      return Err(WebsocketError::NotConnected)?;
    }

    let event = self.recv_message(buf).await;

    // set connection to closed
    if let Ok(Message::Close { .. }) | Err(..) = event {
      *(self.closed.write().await) = true;
    }

    event
  }

  async fn recv_message<'a>(&mut self, buf: &'a mut [u8]) -> Result<Message<'a>, WebsocketError> {
    let frame = Frame::read(&mut self.io, buf, self.max_payload_len).await?;

    if !frame.fin {
      return Err(WebsocketError::FramedMessagesAreNotSupported);
    }

    match frame.opcode {
      OpCode::Continuation => Err(WebsocketError::FramedMessagesAreNotSupported),
      OpCode::Text => Err(WebsocketError::TextFramesAreNotSupported),
      OpCode::Binary => Ok(Message::Binary(frame.data)),
      OpCode::Close => Ok(parse_close_body(frame.data)?),
      OpCode::Ping => Err(WebsocketError::PingFramesAreNotSupported),
      OpCode::Pong => Err(WebsocketError::PongFramesAreNotSupported),
    }
  }
}

fn encode_close_body(code: CloseCode, reason: Option<&str>) -> Vec<u8> {
  if let Some(reason) = reason {
    let mut buf = Vec::with_capacity(2 + reason.len());
    buf.copy_from_slice(&(code as u16).to_be_bytes());
    buf.copy_from_slice(reason.as_ref());
    buf
  } else {
    let mut buf = Vec::with_capacity(2);
    buf.copy_from_slice(&(code as u16).to_be_bytes());
    buf
  }
}

fn parse_close_body(msg: &[u8]) -> Result<Message, WebsocketError> {
  let code = msg
    .get(..2)
    .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
    .unwrap_or(1000);

  match code {
    1000..=1003 | 1007..=1011 | 1015 | 3000..=3999 | 4000..=4999 => {
      let msg = msg.get(2..).map(std::str::from_utf8).transpose()?;

      Ok(Message::Close {
        code: code.into(),
        reason: msg,
      })
    }
    code => Err(WebsocketError::InvalidCloseCode(code)),
  }
}
