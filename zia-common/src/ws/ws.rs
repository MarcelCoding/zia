use anyhow::anyhow;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

use crate::ws::frame::OpCode;
use crate::ws::{Event, Frame, Role};

/// WebSocket implementation for both client and server
pub struct WebSocket<IO> {
  /// it is a low-level abstraction that represents the underlying byte stream over which WebSocket messages are exchanged.
  pub io: IO,

  /// Maximum allowed payload length in bytes.
  pub max_payload_len: usize,

  role: Role,
  is_closed: bool,
}

impl<IO> WebSocket<IO> {
  #[inline]
  pub fn new(stream: IO, max_payload_len: usize, role: Role) -> Self {
    Self {
      io: stream,
      max_payload_len,
      role,
      is_closed: false,
    }
  }
}

impl<W: Unpin + AsyncWrite> WebSocket<W> {
  pub async fn send(&mut self, frame: Frame<'_>) -> anyhow::Result<()> {
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

  // TODO: implement close
  // pub async fn close<T>(mut self, reason: T) -> anyhow::Result<()>
  // where
  //   T: CloseReason,
  //   T::Bytes: AsRef<[u8]>,
  // {
  //   let frame = Frame {
  //     fin: true,
  //     opcode: 8,
  //     data: reason.to_bytes().as_ref(),
  //   };
  //
  //   self.send(frame).await?;
  //   self.flush().await?;
  //   Ok(())
  // }

  pub async fn flush(&mut self) -> anyhow::Result<()> {
    self.io.flush().await?;
    Ok(())
  }
}

// ------------------------------------------------------------------------

impl<R> WebSocket<R>
where
  R: Unpin + AsyncRead,
{
  /// reads [Event] from websocket stream.
  pub async fn recv<'a>(&mut self, buf: &'a mut [u8]) -> anyhow::Result<Event<'a>> {
    if self.is_closed {
      return Err(std::io::Error::new(
        std::io::ErrorKind::NotConnected,
        "read after close",
      ))?;
    }
    let event = self.recv_event(buf).await;
    if let Ok(Event::Close { .. }) | Err(..) = event {
      self.is_closed = true;
    }
    event
  }

  // ### WebSocket Frame Header
  //
  // ```txt
  //  0                   1                   2                   3
  //  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
  // +-+-+-+-+-------+-+-------------+-------------------------------+
  // |F|R|R|R| opcode|M| Payload len |    Extended payload length    |
  // |I|S|S|S|  (4)  |A|     (7)     |             (16/64)           |
  // |N|V|V|V|       |S|             |   (if payload len==126/127)   |
  // | |1|2|3|       |K|             |                               |
  // +-+-+-+-+-------+-+-------------+ - - - - - - - - - - - - - - - +
  // |     Extended payload length continued, if payload len == 127  |
  // + - - - - - - - - - - - - - - - +-------------------------------+
  // |                               |Masking-key, if MASK set to 1  |
  // +-------------------------------+-------------------------------+
  // | Masking-key (continued)       |          Payload Data         |
  // +-------------------------------- - - - - - - - - - - - - - - - +
  // :                     Payload Data continued ...                :
  // + - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - +
  // |                     Payload Data continued ...                |
  // +---------------------------------------------------------------+
  // ```
  /// reads [Event] from websocket stream.
  pub async fn recv_event<'a>(&mut self, buf: &'a mut [u8]) -> anyhow::Result<Event<'a>> {
    let frame = Frame::read(&mut self.io, buf, self.max_payload_len).await?;

    if !frame.fin {
      return Err(anyhow!("framed messages are not supported"));
    }

    match frame.opcode {
      OpCode::Continuation => Err(anyhow!("framed messages are not supported")),
      OpCode::Text => Err(anyhow!("text frames are not supported")),
      OpCode::Binary => Ok(Event::Data(frame.data)),
      OpCode::Close => Ok(parse_close_body(frame.data)?),
      OpCode::Ping => Err(anyhow!("ping frames are not supported")),
      OpCode::Pong => Err(anyhow!("pong frames are not supported")),
    }
  }
}

fn parse_close_body(msg: &[u8]) -> anyhow::Result<Event> {
  let code = msg
    .get(..2)
    .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
    .unwrap_or(1000);

  match code {
    1000..=1003 | 1007..=1011 | 1015 | 3000..=3999 | 4000..=4999 => {
      match msg.get(2..).map(|data| String::from_utf8(data.to_vec())) {
        Some(Ok(msg)) => Ok(Event::Close {
          code: code.into(),
          reason: msg,
        }),
        None => Ok(Event::Close {
          code: code.into(),
          reason: "".into(),
        }),
        Some(Err(_)) => Err(anyhow!("invalid utf-8 payload")),
      }
    }
    _ => Err(anyhow!("invalid close code")),
  }
}
