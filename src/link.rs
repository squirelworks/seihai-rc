//! One TCP link to a gserver or listserver, with its frame codec.
//!
//! Two codecs are used by RCs:
//!
//! - **Plain** (old RCs from 2000 to 2002, type 1): every frame is
//!   `u16be length` + zlib body. No cipher.
//! - **Gen 5** (newer RCs, type 6, and the listserver RC list): the first
//!   frame (the login) is plain zlib with no compression byte and no XOR.
//!   After it, every frame in both directions is `u16be length` +
//!   compression byte (`0x02` none, `0x04` zlib, `0x06` bz2) + a body XORed
//!   with the gen-5 stream keyed by the login key. Each direction has its
//!   own stream.
//!
//! The body of a frame is one or more messages, each ending in `\n`.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use seihai_core::protocol::{
    bz2_decompress, choose_comp, split_messages, write_frame, write_zlib_frame, zlib_compress,
    zlib_decompress, Gen5, Gen5Comp, Message,
};

/// Frame codec of a link.
#[derive(Debug, Clone)]
pub enum Codec {
    /// `u16be length` + zlib, no cipher.
    Plain,
    /// Gen-5 partial XOR after the login frame. `out` encodes what we send,
    /// `inp` decodes what we receive.
    Gen5 { out: Gen5, inp: Gen5 },
}

impl Codec {
    /// A gen-5 codec for the login `key` (the value, not the GCHAR byte).
    pub fn gen5(key: u8) -> Self {
        Codec::Gen5 {
            out: Gen5::new(key),
            inp: Gen5::new(key),
        }
    }

    /// Encode one frame body (messages with their line feeds) as a whole
    /// frame, length included.
    pub fn encode(&mut self, body: &[u8]) -> io::Result<Vec<u8>> {
        match self {
            Codec::Plain => {
                let frame = write_zlib_frame(body)?;
                // `write_zlib_frame` stores the length as a u16. A compressed
                // body that does not fit would be truncated; refuse it.
                let n = usize::from(u16::from_be_bytes([frame[0], frame[1]]));
                if frame.len() != 2 + n {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "frame too long",
                    ));
                }
                Ok(frame)
            }
            Codec::Gen5 { out, .. } => {
                // Short bodies go uncompressed, as the clients send them;
                // bz2 is never needed for what an RC sends.
                let (comp, data) = match choose_comp(body.len()) {
                    Gen5Comp::None => (Gen5Comp::None, body.to_vec()),
                    _ => (Gen5Comp::Zlib, zlib_compress(body)?),
                };
                let data = out.encode(data, comp);
                if data.len() + 1 > usize::from(u16::MAX) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "frame too long",
                    ));
                }
                Ok(write_frame(comp, &data))
            }
        }
    }

    /// Decode one frame (without its length) to the message body.
    pub fn decode(&mut self, frame: &[u8]) -> io::Result<Vec<u8>> {
        match self {
            Codec::Plain => zlib_decompress(frame),
            Codec::Gen5 { inp, .. } => {
                let Some((&c, rest)) = frame.split_first() else {
                    return Ok(Vec::new());
                };
                let comp = match c {
                    0x02 => Gen5Comp::None,
                    0x04 => Gen5Comp::Zlib,
                    0x06 => Gen5Comp::Bz2,
                    other => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("unknown gen-5 compression byte {other:#04x}"),
                        ))
                    }
                };
                let body = inp.decode(rest.to_vec(), comp);
                match comp {
                    Gen5Comp::None => Ok(body),
                    Gen5Comp::Zlib => zlib_decompress(&body),
                    Gen5Comp::Bz2 => bz2_decompress(&body),
                }
            }
        }
    }
}

/// A connected link.
pub struct Link {
    stream: TcpStream,
    codec: Codec,
    buf: Vec<u8>,
    closed: bool,
}

/// Resolve `addr` (`host:port`) to its first socket address.
pub fn resolve(addr: &str) -> io::Result<SocketAddr> {
    addr.to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no address for {addr}")))
}

impl Link {
    /// Connect to `addr` within `timeout`.
    pub fn connect(addr: &str, timeout: Duration) -> io::Result<Self> {
        let stream = TcpStream::connect_timeout(&resolve(addr)?, timeout)?;
        stream.set_nodelay(true)?;
        Ok(Self {
            stream,
            codec: Codec::Plain,
            buf: Vec::new(),
            closed: false,
        })
    }

    /// Send the login frame: `body` (with its line feed) as a plain zlib
    /// frame. Then switch to `codec` for everything after it.
    pub fn send_login(&mut self, body: &[u8], codec: Codec) -> io::Result<()> {
        self.send_plain(body)?;
        self.codec = codec;
        Ok(())
    }

    /// Send `body` (with its line feed) as a plain zlib frame.
    ///
    /// The codec stays as it is. The secure RC-list handshake sends two
    /// plain frames, C1 and C2, and only then switches to gen 5.
    pub fn send_plain(&mut self, body: &[u8]) -> io::Result<()> {
        let frame = write_zlib_frame(body)?;
        self.stream.write_all(&frame)?;
        self.stream.flush()
    }

    /// Use `codec` for every frame after this call.
    pub fn set_codec(&mut self, codec: Codec) {
        self.codec = codec;
    }

    /// Send messages (without line feeds) in one frame.
    pub fn send(&mut self, msgs: &[Vec<u8>]) -> io::Result<()> {
        let mut body = Vec::new();
        for m in msgs {
            if m.contains(&b'\n') {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "a message may not hold a line feed",
                ));
            }
            body.extend_from_slice(m);
            body.push(b'\n');
        }
        let frame = self.codec.encode(&body)?;
        self.stream.write_all(&frame)?;
        self.stream.flush()
    }

    /// Send a frame body that is already complete, including any raw block.
    ///
    /// Unlike [`Self::send`], `body` may contain line feeds. A file upload
    /// needs that: the bytes sit in a length-prefixed raw block, and a level
    /// file contains line feeds.
    pub fn send_body(&mut self, body: &[u8]) -> io::Result<()> {
        let frame = self.codec.encode(body)?;
        self.stream.write_all(&frame)?;
        self.stream.flush()
    }

    /// True once the peer has closed the link.
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// The next frame's messages, waiting at most until `deadline`.
    /// `Ok(None)` when the deadline passed or the peer closed the link
    /// (see [`Self::is_closed`]).
    pub fn recv(&mut self, deadline: Instant) -> io::Result<Option<Vec<Message>>> {
        Ok(self.recv_body(deadline)?.map(|body| split_messages(&body)))
    }

    /// The next frame's decoded body, waiting at most until `deadline`.
    ///
    /// Unlike [`Self::recv`], the body is not split into messages. The
    /// secure hello is one line that is not a `GCHAR` id.
    pub fn recv_body(&mut self, deadline: Instant) -> io::Result<Option<Vec<u8>>> {
        loop {
            if let Some(frame) = self.take_frame() {
                return Ok(Some(self.codec.decode(&frame)?));
            }
            if self.closed {
                return Ok(None);
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            self.stream
                .set_read_timeout(Some((deadline - now).max(Duration::from_millis(1))))?;
            let mut chunk = [0u8; 16384];
            match self.stream.read(&mut chunk) {
                Ok(0) => self.closed = true,
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted
                    ) =>
                {
                    self.closed = true
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Take one whole frame (without its length) off the buffer.
    fn take_frame(&mut self) -> Option<Vec<u8>> {
        if self.buf.len() < 2 {
            return None;
        }
        let n = usize::from(u16::from_be_bytes([self.buf[0], self.buf[1]]));
        if self.buf.len() < 2 + n {
            return None;
        }
        let frame = self.buf[2..2 + n].to_vec();
        self.buf.drain(..2 + n);
        Some(frame)
    }

    /// Close the link.
    pub fn shutdown(&self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gen5_round_trip_all_sizes() {
        for len in [0usize, 1, 5, 47, 48, 300, 5000] {
            let body: Vec<u8> = (0..len).map(|i| b'a' + (i % 26) as u8).collect();
            let mut a = Codec::gen5(0x2A);
            let mut b = Codec::gen5(0x2A);
            let frame = a.encode(&body).unwrap();
            assert_eq!(
                usize::from(u16::from_be_bytes([frame[0], frame[1]])),
                frame.len() - 2
            );
            // b decodes with its *input* stream, which matches a's output.
            assert_eq!(b.decode(&frame[2..]).unwrap(), body, "len {len}");
        }
    }

    #[test]
    fn plain_round_trip() {
        let mut c = Codec::Plain;
        let frame = c.encode(b"!hello\n").unwrap();
        assert_eq!(c.decode(&frame[2..]).unwrap(), b"!hello\n");
    }

    #[test]
    fn plain_frame_over_u16_is_rejected() {
        // A counter compresses. Mix each index so the zlib form cannot
        // fit in a u16 length.
        let body: Vec<u8> = (0u32..120_000)
            .map(|i| {
                let mut x = i.wrapping_add(0x9E37_79B9);
                x = x.wrapping_mul(0x85EB_CA6B);
                x ^= x >> 13;
                x = x.wrapping_mul(0xC2B2_AE35);
                x ^= x >> 16;
                x as u8
            })
            .collect();
        let mut c = Codec::Plain;
        let err = c.encode(&body).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(err.to_string(), "frame too long");
    }
}
