//! Wire format between the OBS plugin and its client (the Vesktop patch, or `odl-client`).
//!
//! The plugin listens on a Unix stream socket, [`socket_path`]. Every message is
//! a little-endian `u32` length of what follows, one type byte, then the payload.
//! Strings are a `u8` length and UTF-8 bytes.
//!
//! A session is: the client connects and sends [`ClientMsg::Start`]; the plugin
//! starts its OBS output and answers [`ServerMsg::Config`], then one
//! [`ServerMsg::Frame`] per encoded frame until the client sends
//! [`ClientMsg::Stop`] or disconnects. One client at a time; a new connection
//! replaces the old one.

use std::io::{self, Read, Write};
use std::path::PathBuf;

/// Upper bound on one message, so a corrupt length cannot allocate gigabytes.
pub const MAX_MESSAGE: u32 = 16 << 20;

pub fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").unwrap_or_else(|| "/tmp".into());
    PathBuf::from(dir).join("obs-discord-live.sock")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMsg {
    /// `codec` is the WebRTC codec name the peer connection negotiated:
    /// `H264`, `AV1`, `VP8` or `VP9`, compared case-insensitively.
    Start { codec: String, bitrate_kbps: u32 },
    /// The receiver lost the picture (a PLI or FIR); the next frame is an IDR.
    KeyframeRequest,
    SetBitrate { kbps: u32 },
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMsg {
    Config {
        codec: String,
        encoder: String,
        width: u32,
        height: u32,
        fps_num: u32,
        fps_den: u32,
    },
    /// One access unit. H.264 is Annex B with SPS and PPS in front of every IDR;
    /// AV1 is a low-overhead OBU stream with a sequence header on every keyframe.
    Frame { pts_us: u64, keyframe: bool, data: Vec<u8> },
    Error { message: String },
}

const C_START: u8 = 0x01;
const C_KEYFRAME: u8 = 0x02;
const C_BITRATE: u8 = 0x03;
const C_STOP: u8 = 0x04;
const S_CONFIG: u8 = 0x81;
const S_FRAME: u8 = 0x82;
const S_ERROR: u8 = 0x83;

struct Writer(Vec<u8>);

impl Writer {
    fn new(kind: u8) -> Self {
        Writer(vec![0, 0, 0, 0, kind])
    }
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn str(&mut self, s: &str) -> &mut Self {
        let b = &s.as_bytes()[..s.len().min(255)];
        self.0.push(b.len() as u8);
        self.0.extend_from_slice(b);
        self
    }
    fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.0.extend_from_slice(b);
        self
    }
    fn finish(&mut self) -> Vec<u8> {
        let len = (self.0.len() - 4) as u32;
        self.0[..4].copy_from_slice(&len.to_le_bytes());
        std::mem::take(&mut self.0)
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> io::Result<&[u8]> {
        if self.0.len() < n {
            return Err(invalid("message ends early"));
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn str(&mut self) -> io::Result<String> {
        let n = self.u8()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| invalid("string is not UTF-8"))
    }
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

impl ClientMsg {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            ClientMsg::Start { codec, bitrate_kbps } => {
                Writer::new(C_START).str(codec).u32(*bitrate_kbps).finish()
            }
            ClientMsg::KeyframeRequest => Writer::new(C_KEYFRAME).finish(),
            ClientMsg::SetBitrate { kbps } => Writer::new(C_BITRATE).u32(*kbps).finish(),
            ClientMsg::Stop => Writer::new(C_STOP).finish(),
        }
    }

    fn decode(kind: u8, mut r: Reader) -> io::Result<Self> {
        Ok(match kind {
            C_START => ClientMsg::Start { codec: r.str()?, bitrate_kbps: r.u32()? },
            C_KEYFRAME => ClientMsg::KeyframeRequest,
            C_BITRATE => ClientMsg::SetBitrate { kbps: r.u32()? },
            C_STOP => ClientMsg::Stop,
            _ => return Err(invalid("unknown client message type")),
        })
    }
}

impl ServerMsg {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            ServerMsg::Config { codec, encoder, width, height, fps_num, fps_den } => {
                Writer::new(S_CONFIG)
                    .str(codec)
                    .str(encoder)
                    .u32(*width)
                    .u32(*height)
                    .u32(*fps_num)
                    .u32(*fps_den)
                    .finish()
            }
            ServerMsg::Frame { pts_us, keyframe, data } => Writer::new(S_FRAME)
                .u64(*pts_us)
                .u8(*keyframe as u8)
                .bytes(data)
                .finish(),
            ServerMsg::Error { message } => Writer::new(S_ERROR).str(message).finish(),
        }
    }

    fn decode(kind: u8, mut r: Reader) -> io::Result<Self> {
        Ok(match kind {
            S_CONFIG => ServerMsg::Config {
                codec: r.str()?,
                encoder: r.str()?,
                width: r.u32()?,
                height: r.u32()?,
                fps_num: r.u32()?,
                fps_den: r.u32()?,
            },
            S_FRAME => ServerMsg::Frame {
                pts_us: r.u64()?,
                keyframe: r.u8()? != 0,
                data: r.0.to_vec(),
            },
            S_ERROR => ServerMsg::Error { message: r.str()? },
            _ => return Err(invalid("unknown server message type")),
        })
    }
}

/// Reads one message body; `Ok(None)` is a clean end of stream before a length.
fn read_body(r: &mut impl Read, buf: &mut Vec<u8>) -> io::Result<Option<u8>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        other => other?,
    }
    let len = u32::from_le_bytes(len);
    if len == 0 || len > MAX_MESSAGE {
        return Err(invalid("message length out of range"));
    }
    buf.resize(len as usize, 0);
    r.read_exact(buf)?;
    Ok(Some(buf[0]))
}

pub fn read_client(r: &mut impl Read) -> io::Result<Option<ClientMsg>> {
    let mut buf = Vec::new();
    match read_body(r, &mut buf)? {
        None => Ok(None),
        Some(kind) => ClientMsg::decode(kind, Reader(&buf[1..])).map(Some),
    }
}

pub fn read_server(r: &mut impl Read) -> io::Result<Option<ServerMsg>> {
    let mut buf = Vec::new();
    match read_body(r, &mut buf)? {
        None => Ok(None),
        Some(kind) => ServerMsg::decode(kind, Reader(&buf[1..])).map(Some),
    }
}

pub fn write(w: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    w.write_all(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let client = [
            ClientMsg::Start { codec: "H264".into(), bitrate_kbps: 2500 },
            ClientMsg::KeyframeRequest,
            ClientMsg::SetBitrate { kbps: 1200 },
            ClientMsg::Stop,
        ];
        let mut wire = Vec::new();
        for m in &client {
            wire.extend(m.encode());
        }
        let mut r = &wire[..];
        for m in &client {
            assert_eq!(read_client(&mut r).unwrap().as_ref(), Some(m));
        }
        assert_eq!(read_client(&mut r).unwrap(), None);

        let server = [
            ServerMsg::Config {
                codec: "H264".into(),
                encoder: "libx264".into(),
                width: 1280,
                height: 720,
                fps_num: 30,
                fps_den: 1,
            },
            ServerMsg::Frame { pts_us: 33_333, keyframe: true, data: vec![0, 0, 0, 1, 0x67] },
            ServerMsg::Error { message: "no encoder".into() },
        ];
        let mut wire = Vec::new();
        for m in &server {
            wire.extend(m.encode());
        }
        let mut r = &wire[..];
        for m in &server {
            assert_eq!(read_server(&mut r).unwrap().as_ref(), Some(m));
        }
        assert_eq!(read_server(&mut r).unwrap(), None);
    }

    #[test]
    fn rejects_oversized_length() {
        let wire = (MAX_MESSAGE + 1).to_le_bytes();
        assert!(read_client(&mut &wire[..]).is_err());
    }
}
