use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::ws::WebsocketError;

#[repr(u8)]
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub(crate) enum OpCode {
  Continuation = 0x0,
  Text = 0x1,
  Binary = 0x2,
  Close = 0x8,
  Ping = 0x9,
  Pong = 0xA,
}

impl TryFrom<u8> for OpCode {
  type Error = WebsocketError;

  fn try_from(value: u8) -> Result<Self, Self::Error> {
    match value {
      0x0 => Ok(Self::Continuation),
      0x1 => Ok(Self::Text),
      0x2 => Ok(Self::Binary),
      0x8 => Ok(Self::Close),
      0x9 => Ok(Self::Ping),
      0xA => Ok(Self::Pong),
      value => Err(WebsocketError::UnknownOpCode(value)),
    }
  }
}

pub(crate) struct Frame<'a> {
  pub(crate) fin: bool,
  pub(crate) opcode: OpCode,
  pub(crate) data: &'a [u8],
}

impl<'a> Frame<'a> {
  #[inline]
  pub(crate) fn new(fin: bool, opcode: OpCode, data: &'a [u8]) -> Self {
    Self { fin, opcode, data }
  }

  /// ### WebSocket Frame Header
  /// <https://datatracker.ietf.org/doc/html/rfc6455#section-5.2>
  ///
  /// ```txt
  ///  0                   1                   2                   3
  ///  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
  /// +-+-+-+-+-------+-+-------------+-------------------------------+
  /// |F|R|R|R| opcode|M| Payload len |    Extended payload length    |
  /// |I|S|S|S|  (4)  |A|     (7)     |             (16/64)           |
  /// |N|V|V|V|       |S|             |   (if payload len==126/127)   |
  /// | |1|2|3|       |K|             |                               |
  /// +-+-+-+-+-------+-+-------------+ - - - - - - - - - - - - - - - +
  /// |     Extended payload length continued, if payload len == 127  |
  /// + - - - - - - - - - - - - - - - +-------------------------------+
  /// |                               |Masking-key, if MASK set to 1  |
  /// +-------------------------------+-------------------------------+
  /// | Masking-key (continued)       |          Payload Data         |
  /// +-------------------------------- - - - - - - - - - - - - - - - +
  /// :                     Payload Data continued ...                :
  /// + - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - +
  /// |                     Payload Data continued ...                |
  /// +---------------------------------------------------------------+
  /// ```
  pub(crate) async fn read<R: Unpin + AsyncRead>(
    read: &mut R,
    buf: &'a mut [u8],
    max_payload_len: usize,
  ) -> Result<Frame<'a>, WebsocketError> {
    let [b1, b2] = {
      let mut header = [0u8; 2];
      read.read_exact(&mut header).await?;
      header
    };

    let fin = b1 & 0b1000_0000 != 0;
    let rsv = b1 & 0b0111_0000;
    let opcode = OpCode::try_from(b1 & 0b0000_1111)?;

    let len = (b2 & 0b0111_1111) as usize;
    let masked = b2 & 0b_1000_0000 != 0;

    if rsv != 0 {
      return Err(WebsocketError::ReserveBitMustBeNull);
    }

    let len = match opcode {
      OpCode::Continuation | OpCode::Text | OpCode::Binary => match len {
        126 => read.read_u16().await? as usize,
        127 => read.read_u64().await? as usize,
        len => len,
      },
      OpCode::Close | OpCode::Ping | OpCode::Pong => {
        if !fin {
          return Err(WebsocketError::ControlFrameMustNotBeFragmented);
        }

        if len > 125 {
          return Err(WebsocketError::ControlFrameMustHaveAPayloadLengthOf125BytesOrLess);
        }

        len
      }
    };

    if len > max_payload_len {
      return Err(WebsocketError::PayloadTooLarge);
    }

    read_payload(read, &mut buf[..len], masked).await?;

    Ok(Self {
      fin,
      opcode,
      data: &buf[..len],
    })
  }

  pub(crate) async fn write_without_mask<W: Unpin + AsyncWrite>(
    self,
    write: &mut W,
  ) -> Result<(), WebsocketError> {
    self.write_header(write, 0).await?;
    write.write_all(self.data).await?;

    Ok(())
  }

  pub async fn write_with_mask<W: Unpin + AsyncWrite>(
    self,
    write: &mut W,
    mask: [u8; 4],
  ) -> Result<(), WebsocketError> {
    self.write_header(write, 0x80).await?;
    write.write_all(&mask).await?;

    for i in 0..self.data.len() {
      // TODO: Use SIMD wherever possible for best performance
      write
        .write_u8(unsafe { self.data.get_unchecked(i) ^ mask.get_unchecked(i & 3) })
        .await?
    }

    Ok(())
  }

  async fn write_header<W: Unpin + AsyncWrite>(
    &self,
    write: &mut W,
    mask_bit: u8,
  ) -> Result<(), WebsocketError> {
    write
      .write_u8(((self.fin as u8) << 7) | self.opcode as u8)
      .await?;

    let len = self.data.len();

    if len < 126 {
      write.write_u8(mask_bit | len as u8).await?;
    } else if len < 65536 {
      write.write_u8(mask_bit | 126).await?;
      write.write_u16(len as u16).await?;
    } else {
      write.write_u8(mask_bit | 127).await?;
      write.write_u64(len as u64).await?;
    }

    Ok(())
  }
}

async fn read_payload<R: Unpin + AsyncRead>(
  read: &mut R,
  buf: &mut [u8],
  masked: bool,
) -> Result<(), WebsocketError> {
  if masked {
    let mut mask = [0u8; 4];
    read.read_exact(&mut mask).await?;
    read.read_exact(buf).await?;
    // TODO: Use SIMD wherever possible for best performance
    for i in 0..buf.len() {
      buf[i] ^= mask[i & 3];
    }
  } else {
    read.read_exact(buf).await?;
  }

  Ok(())
}
