pub use frame::Frame;
pub use ws::WebSocket;

mod frame;
mod ws;

pub enum Role {
  Server,
  Client { masking: bool },
}

pub enum Event<'a> {
  Data(&'a [u8]),
  Close { code: CloseCode, reason: String },
}

/// When closing an established connection an endpoint MAY indicate a reason for closure.
#[derive(Debug, Clone, Copy)]
pub enum CloseCode {
  /// The purpose for which the connection was established has been fulfilled
  Normal = 1000,
  /// Server going down or a browser having navigated away from a page
  Away = 1001,
  /// An endpoint is terminating the connection due to a protocol error.
  ProtocolError = 1002,
  /// It has received a type of data it cannot accept
  Unsupported = 1003,

  // reserved 1004
  /// MUST NOT be set as a status code in a Close control frame by an endpoint.
  ///
  /// No status code was actually present.
  NoStatusRcvd = 1005,
  /// MUST NOT be set as a status code in a Close control frame by an endpoint.
  ///
  /// Connection was closed abnormally.
  Abnormal = 1006,
  /// Application has received data within a message that was not consistent with the type of the message.
  InvalidPayload = 1007,
  /// This is a generic status code that can be returned when there is no other more suitable status code.
  PolicyViolation = 1008,
  /// Message that is too big for it to process.
  MessageTooBig = 1009,
  /// It has expected the server to negotiate one or more extension.
  MandatoryExt = 1010,
  /// The server has encountered an unexpected condition that prevented it from fulfilling the request.
  InternalError = 1011,
  /// MUST NOT be set as a status code in a Close control frame by an endpoint.
  ///
  /// The connection was closed due to a failure to perform a TLS handshake.
  TLSHandshake = 1015,
}

impl From<CloseCode> for u16 {
  #[inline]
  fn from(code: CloseCode) -> Self {
    code as u16
  }
}

impl From<u16> for CloseCode {
  #[inline]
  fn from(value: u16) -> Self {
    match value {
      1000 => CloseCode::Normal,
      1001 => CloseCode::Away,
      1002 => CloseCode::ProtocolError,
      1003 => CloseCode::Unsupported,
      1005 => CloseCode::NoStatusRcvd,
      1006 => CloseCode::Abnormal,
      1007 => CloseCode::InvalidPayload,
      1009 => CloseCode::MessageTooBig,
      1010 => CloseCode::MandatoryExt,
      1011 => CloseCode::InternalError,
      1015 => CloseCode::TLSHandshake,
      _ => CloseCode::PolicyViolation,
    }
  }
}

impl PartialEq<u16> for CloseCode {
  #[inline]
  fn eq(&self, other: &u16) -> bool {
    (*self as u16) == *other
  }
}

/// This trait is responsible for encoding websocket closed frame.
pub trait CloseReason {
  /// Encoded close reason as bytes
  type Bytes;
  /// Encode websocket close frame.
  fn to_bytes(self) -> Self::Bytes;
}

impl CloseReason for () {
  type Bytes = [u8; 0];
  fn to_bytes(self) -> Self::Bytes {
    [0; 0]
  }
}

impl CloseReason for u16 {
  type Bytes = [u8; 2];
  fn to_bytes(self) -> Self::Bytes {
    self.to_be_bytes()
  }
}

impl CloseReason for CloseCode {
  type Bytes = [u8; 2];
  fn to_bytes(self) -> Self::Bytes {
    (self as u16).to_be_bytes()
  }
}

impl CloseReason for &str {
  type Bytes = Vec<u8>;
  fn to_bytes(self) -> Self::Bytes {
    CloseReason::to_bytes((CloseCode::Normal, self))
  }
}

impl<Code, Msg> CloseReason for (Code, Msg)
where
  Code: Into<u16>,
  Msg: AsRef<[u8]>,
{
  type Bytes = Vec<u8>;
  fn to_bytes(self) -> Self::Bytes {
    let (code, reason) = (self.0.into(), self.1.as_ref());
    let mut data = Vec::with_capacity(2 + reason.len());
    data.extend_from_slice(&code.to_be_bytes());
    data.extend_from_slice(reason);
    data
  }
}
