//! Relay xabarlari: bitta qatorli matn (`TEG maydon maydon ...`), frame = `u16` uzunlik + UTF-8.
//! Oxirgi maydon (nom, matn) bo'sh joy saqlashi mumkin. Bo'sh qiymat `-` bilan yoziladi.

use anyhow::{bail, Context, Result};
use protocol::{NONCE_LEN, PROOF_LEN, SALT_LEN};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::hexfmt;

const MAX_LEN: usize = 4096;
pub const MAX_NAME: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RMsg {
    // client -> server
    Hello { id: String, token: String, name: String },
    Proof { proof: [u8; PROOF_LEN] },
    Sub(Vec<String>),
    Connect { target: String, req: String },
    /// Qabul qiluvchi javobi. `pw`: bir martalik parol (`None`: Viewer o'z parolini kiritadi).
    Reply { sid: String, accept: bool, pw: Option<String> },
    /// Ma'lumot oqimining birinchi xabari (control'dan alohida oqimda).
    Data { sid: String },
    // server -> client
    Challenge { salt: [u8; SALT_LEN], nonce: [u8; NONCE_LEN] },
    Ok { id: String, token: String },
    Err { text: String },
    Pres { id: String, online: bool, name: String },
    Incoming { sid: String, from: String, name: String },
    Result { req: String, ok: bool, sid: String, pw: Option<String>, text: String },
}

fn dash(s: &str) -> &str {
    if s.is_empty() {
        "-"
    } else {
        s
    }
}

fn undash(s: &str) -> String {
    if s == "-" {
        String::new()
    } else {
        s.to_string()
    }
}

/// Qator uzilishini va boshqaruv belgilarini olib tashlaydi, uzunlikni cheklaydi.
pub fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect::<String>().trim().to_string()
}

fn arr<const N: usize>(s: &str) -> Result<[u8; N]> {
    hexfmt::decode(s)
        .and_then(|v| <[u8; N]>::try_from(v).ok())
        .context("noto'g'ri hex maydon")
}

impl RMsg {
    pub fn encode(&self) -> String {
        match self {
            RMsg::Hello { id, token, name } => format!("HELLO {} {} {}", dash(id), dash(token), dash(&clean(name))),
            RMsg::Proof { proof } => format!("PROOF {}", hexfmt::encode(proof)),
            RMsg::Sub(ids) => format!("SUB {}", dash(&ids.join(","))),
            RMsg::Connect { target, req } => format!("CONNECT {target} {req}"),
            RMsg::Reply { sid, accept, pw } => {
                format!("REPLY {sid} {} {}", *accept as u8, dash(pw.as_deref().unwrap_or("")))
            }
            RMsg::Data { sid } => format!("DATA {sid}"),
            RMsg::Challenge { salt, nonce } => format!("CHALLENGE {} {}", hexfmt::encode(salt), hexfmt::encode(nonce)),
            RMsg::Ok { id, token } => format!("OK {id} {token}"),
            RMsg::Err { text } => format!("ERR {}", dash(&clean(text))),
            RMsg::Pres { id, online, name } => format!("PRES {id} {} {}", *online as u8, dash(&clean(name))),
            RMsg::Incoming { sid, from, name } => format!("INCOMING {sid} {from} {}", dash(&clean(name))),
            RMsg::Result { req, ok, sid, pw, text } => format!(
                "RESULT {req} {} {} {} {}",
                *ok as u8,
                dash(sid),
                dash(pw.as_deref().unwrap_or("")),
                dash(&clean(text))
            ),
        }
    }

    pub fn decode(line: &str) -> Result<RMsg> {
        let (tag, rest) = line.split_once(' ').unwrap_or((line, ""));
        let mut f = rest.splitn(match tag {
            "HELLO" | "PRES" | "INCOMING" => 3,
            "RESULT" => 5,
            "REPLY" => 3,
            "ERR" => 1,
            _ => 2,
        }, ' ');
        let mut next = || f.next().unwrap_or("");
        Ok(match tag {
            "HELLO" => RMsg::Hello { id: undash(next()), token: undash(next()), name: undash(next()) },
            "PROOF" => RMsg::Proof { proof: arr(next())? },
            "SUB" => {
                let s = next();
                RMsg::Sub(if s == "-" || s.is_empty() { vec![] } else { s.split(',').map(str::to_string).collect() })
            }
            "CONNECT" => RMsg::Connect { target: next().to_string(), req: next().to_string() },
            "REPLY" => {
                let sid = next().to_string();
                let accept = next() == "1";
                let pw = undash(next().trim());
                RMsg::Reply { sid, accept, pw: (!pw.is_empty()).then_some(pw) }
            }
            "DATA" => RMsg::Data { sid: next().to_string() },
            "CHALLENGE" => RMsg::Challenge { salt: arr(next())?, nonce: arr(next())? },
            "OK" => RMsg::Ok { id: next().to_string(), token: next().to_string() },
            "ERR" => RMsg::Err { text: undash(next()) },
            "PRES" => RMsg::Pres { id: next().to_string(), online: next() == "1", name: undash(next()) },
            "INCOMING" => RMsg::Incoming { sid: next().to_string(), from: next().to_string(), name: undash(next()) },
            "RESULT" => {
                let req = next().to_string();
                let ok = next() == "1";
                let sid = undash(next());
                let pw = undash(next());
                let text = undash(next());
                RMsg::Result { req, ok, sid, pw: (!pw.is_empty()).then_some(pw), text }
            }
            other => bail!("noma'lum relay xabari: {other}"),
        })
    }
}

pub async fn write_msg<W: AsyncWrite + Unpin>(w: &mut W, m: &RMsg) -> Result<()> {
    let text = m.encode();
    let b = text.as_bytes();
    if b.len() > MAX_LEN {
        bail!("relay xabari juda uzun");
    }
    let mut frame = Vec::with_capacity(2 + b.len());
    frame.extend_from_slice(&(b.len() as u16).to_be_bytes());
    frame.extend_from_slice(b);
    w.write_all(&frame).await?;
    Ok(())
}

pub async fn read_msg<R: AsyncRead + Unpin>(r: &mut R) -> Result<RMsg> {
    let mut len = [0u8; 2];
    r.read_exact(&mut len).await?;
    let n = u16::from_be_bytes(len) as usize;
    if n == 0 || n > MAX_LEN {
        bail!("noto'g'ri relay frame uzunligi: {n}");
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).await?;
    RMsg::decode(std::str::from_utf8(&buf).context("UTF-8 emas")?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let msgs = vec![
            RMsg::Hello { id: String::new(), token: String::new(), name: "Ali PC".into() },
            RMsg::Hello { id: "123456789".into(), token: "abcd".into(), name: "x".into() },
            RMsg::Proof { proof: [7; PROOF_LEN] },
            RMsg::Sub(vec![]),
            RMsg::Sub(vec!["111111111".into(), "222222222".into()]),
            RMsg::Connect { target: "123456789".into(), req: "r1".into() },
            RMsg::Reply { sid: "s".into(), accept: true, pw: Some("pw1".into()) },
            RMsg::Reply { sid: "s".into(), accept: false, pw: None },
            RMsg::Data { sid: "s".into() },
            RMsg::Challenge { salt: [1; SALT_LEN], nonce: [2; NONCE_LEN] },
            RMsg::Ok { id: "123456789".into(), token: "t".into() },
            RMsg::Err { text: "xato bor".into() },
            RMsg::Pres { id: "123456789".into(), online: true, name: "Ali PC".into() },
            RMsg::Pres { id: "123456789".into(), online: false, name: String::new() },
            RMsg::Incoming { sid: "s".into(), from: "111111111".into(), name: "Vali noutbuk".into() },
            RMsg::Result { req: "r".into(), ok: true, sid: "s".into(), pw: None, text: String::new() },
            RMsg::Result { req: "r".into(), ok: false, sid: String::new(), pw: None, text: "rad etildi".into() },
        ];
        for m in msgs {
            assert_eq!(RMsg::decode(&m.encode()).unwrap(), m, "{m:?}");
        }
    }

    #[test]
    fn unknown_tag_rejected() {
        assert!(RMsg::decode("XYZ 1").is_err());
    }
}
