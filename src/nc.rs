//! NPC-Control session: a second socket, beside the RC link.
//!
//! The 2002 and 2004 staff clients log in here as type 3 with the token
//! `NCL11012`, then keep plain zlib frames (`u16be` length plus a zlib body)
//! for the whole session: no key byte, no compression-type byte, no cipher.
//! Behaviour is taken from Graal_analysis `protocol/npc-control.md`. This
//! module does not copy that note. List, script and weapon payloads are a
//! later slice; a loopback check of any bytes after the password is still
//! open, so this client sends none.

use std::io;
use std::time::{Duration, Instant};

use seihai_core::protocol::Message;

use crate::link::{Codec, Link};
use crate::rc::LoginError;
use crate::wire::{gchar, latin1_bytes, push_gstring, text};

/// Client type on the NPC-Control socket.
pub const NC_LOGIN_TYPE: u8 = 3;
/// Eight-byte version token. Unchanged from the 2002 client to the 2004 one.
pub const NC_TOKEN: &str = "NCL11012";
/// Server to client: the session was refused or ended. Payload is text.
pub const NC_DISCMESSAGE: u8 = 16;

/// The login message body, line feed included.
///
/// ```text
/// GCHAR type   token (8 bytes)   GSTRING account   GSTRING password
/// ```
pub fn login_body(account: &str, password: &str) -> Vec<u8> {
    let mut b = Vec::with_capacity(12 + account.len() + password.len());
    b.push(gchar(NC_LOGIN_TYPE));
    b.extend_from_slice(NC_TOKEN.as_bytes());
    push_gstring(&mut b, &latin1_bytes(account));
    push_gstring(&mut b, &latin1_bytes(password));
    b.push(b'\n');
    b
}

/// A message on the NPC-Control socket, as far as the login slice reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NcEvent {
    /// The server refused or ended the session.
    Disconnect(String),
    /// Any other message. Id and payload only; layouts come in later.
    Message { id: u8, payload: Vec<u8> },
}

fn parse_event(m: &Message) -> NcEvent {
    if m.id == NC_DISCMESSAGE {
        NcEvent::Disconnect(text(&m.payload))
    } else {
        NcEvent::Message {
            id: m.id,
            payload: m.payload.clone(),
        }
    }
}

/// A logged-in NPC-Control session. Frames stay plain zlib both ways.
pub struct NcSession {
    link: Link,
    pending: Vec<NcEvent>,
}

impl NcSession {
    /// Connect to `addr` and send the type-3 login as a plain zlib frame.
    ///
    /// The staff client shows itself connected once that frame is sent.
    /// There is no separate accept packet in the notes, so a link that stays
    /// open counts as accepted. A disconnect message, or a link that closes
    /// before `timeout`, is a refusal. Any other traffic returns at once so
    /// the caller can read it.
    pub fn login(
        addr: &str,
        account: &str,
        password: &str,
        timeout: Duration,
    ) -> Result<Self, LoginError> {
        let deadline = Instant::now() + timeout;
        let mut link = Link::connect(addr, timeout)?;
        link.send_login(&login_body(account, password), Codec::Plain)?;
        let mut s = Self {
            link,
            pending: Vec::new(),
        };
        loop {
            let Some(msgs) = s.link.recv(deadline)? else {
                return if s.link.is_closed() {
                    Err(LoginError::Closed)
                } else {
                    Ok(s)
                };
            };
            let mut traffic = false;
            for m in &msgs {
                match parse_event(m) {
                    NcEvent::Disconnect(why) => return Err(LoginError::Refused(why)),
                    ev => {
                        traffic = true;
                        s.pending.push(ev);
                    }
                }
            }
            if traffic {
                return Ok(s);
            }
        }
    }

    /// The next event, waiting at most until `deadline`. `Ok(None)` when
    /// the deadline passed or the server closed the link.
    pub fn next_event(&mut self, deadline: Instant) -> io::Result<Option<NcEvent>> {
        loop {
            if !self.pending.is_empty() {
                return Ok(Some(self.pending.remove(0)));
            }
            let Some(msgs) = self.link.recv(deadline)? else {
                return Ok(None);
            };
            for m in &msgs {
                self.pending.push(parse_event(m));
            }
        }
    }

    /// True once the server has closed the link.
    pub fn is_closed(&self) -> bool {
        self.link.is_closed()
    }

    /// Close the session.
    pub fn close(self) {
        self.link.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_body_is_type_3_token_and_two_strings() {
        assert_eq!(login_body("admin", "pw"), b"#NCL11012%admin\"pw\n");
    }

    #[test]
    fn login_body_has_no_key_and_no_trailing_byte() {
        let b = login_body("a", "b");
        assert_eq!(b[0], gchar(NC_LOGIN_TYPE));
        assert_eq!(&b[1..9], NC_TOKEN.as_bytes());
        assert!(b.ends_with(b"\n"));
        assert_eq!(b.iter().filter(|c| **c == b'\n').count(), 1);
    }

    #[test]
    fn login_body_keeps_latin1() {
        let b = login_body("é", "ü");
        assert!(b.contains(&0xE9));
        assert!(b.contains(&0xFC));
    }
}
