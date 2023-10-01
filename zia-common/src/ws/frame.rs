use tokio::io::{AsyncWrite, AsyncWriteExt};

pub struct Frame<'a> {
  pub fin: bool,
  pub opcode: u8,
  pub data: &'a [u8],
}

impl<'a> Frame<'a> {
  #[inline]
  pub fn binary(data: &'a [u8]) -> Self {
    Self {
      fin: true,
      opcode: 2,
      data,
    }
  }

  #[inline]
  pub async fn write_without_mask<W: Unpin + AsyncWrite>(
    self,
    write: &mut W,
  ) -> anyhow::Result<()> {
    self.write_header(write, 0).await?;
    write.write_all(self.data).await?;

    Ok(())
  }

  #[inline]
  pub async fn write_with_mask<W: Unpin + AsyncWrite>(
    self,
    write: &mut W,
    mask: [u8; 4],
  ) -> anyhow::Result<()> {
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
  ) -> anyhow::Result<()> {
    write
      .write_u8(((self.fin as u8) << 7) | self.opcode)
      .await?;

    if self.data.len() < 126 {
      write.write_u8(mask_bit | self.data.len() as u8).await?;
    } else if self.data.len() < 65536 {
      write.write_u8(mask_bit | 126).await?;
      write.write_u16(self.data.len() as u16).await?;
    } else {
      write.write_u8(mask_bit | 127).await?;
      write.write_u64(self.data.len() as u64).await?;
    }

    Ok(())
  }
}
