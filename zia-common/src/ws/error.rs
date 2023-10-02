use std::io;
use std::str::Utf8Error;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum WebsocketError {
  #[error("unknown opcode `{0}`")]
  UnknownOpCode(u8),
  #[error("reserve bit must be `0`")]
  ReserveBitMustBeNull,
  #[error("control frame must not be fragmented")]
  ControlFrameMustNotBeFragmented,
  #[error("control frame must have a payload length of 125 bytes or less")]
  ControlFrameMustHaveAPayloadLengthOf125BytesOrLess,
  #[error("payload too large")]
  PayloadTooLarge,
  #[error("io error")]
  Io(#[from] io::Error),
  #[error("not connected")]
  NotConnected,
  #[error("framed messages are not supported")]
  FramedMessagesAreNotSupported,
  #[error("text frames are not supported")]
  TextFramesAreNotSupported,
  #[error("ping frames are not supported")]
  PingFramesAreNotSupported,
  #[error("pong frames are not supported")]
  PongFramesAreNotSupported,
  #[error("invalid utf8")]
  InvalidUtf8(#[from] Utf8Error),
  #[error("invalid close close `{0}`")]
  InvalidCloseCode(u16),
}
