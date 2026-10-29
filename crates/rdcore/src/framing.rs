//! Xabarlarni oqim (stream) orqali yuborish va o'qish.

use anyhow::{bail, Result};
use protocol::{Message, MAX_FRAME_LEN};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub async fn write_message<W: AsyncWrite + Unpin>(w: &mut W, msg: &Message) -> Result<()> {
    let frame = protocol::encode_frame(msg);
    w.write_all(&frame).await?;
    Ok(())
}

pub async fn read_message<R: AsyncRead + Unpin>(r: &mut R) -> Result<Message> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > MAX_FRAME_LEN {
        bail!("noto'g'ri frame uzunligi: {len}");
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload).await?;
    Ok(Message::decode(&payload)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn write_then_read() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        let msg = Message::Ping { id: 7, sent_ms: 99 };
        write_message(&mut a, &msg).await.unwrap();
        assert_eq!(read_message(&mut b).await.unwrap(), msg);
    }

    #[tokio::test]
    async fn rejects_oversized_frame() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        a.write_all(&((MAX_FRAME_LEN as u32 + 1).to_be_bytes())).await.unwrap();
        assert!(read_message(&mut b).await.is_err());
    }

    #[tokio::test]
    async fn eof_is_error() {
        let (a, mut b) = tokio::io::duplex(1024);
        drop(a);
        assert!(read_message(&mut b).await.is_err());
    }
}
