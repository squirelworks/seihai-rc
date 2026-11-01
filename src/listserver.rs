//! The listserver's RC list: every server, type-tagged.
//!
//! Login shape (gen 5, the same key for both directions):
//!
//! 1. A plain zlib frame with id 5 = `GCHAR key` + the RC's 8-byte token.
//! 2. A gen-5 frame with id 1 = `GSTRING account` + `GSTRING password`.
//!
//! The secure form ([`fetch_rc_list_secure`]) uses the same id 5 and the
//! same reply. The account goes in C2 instead of a gen-5 id 1. See
//! `docs/PROTOCOL.md`.
//!
//! The listserver answers with gen-5 frames: id 0 the server list (as for
//! the 2.22 client list, names carrying a type tag such as `P ` or `H `),
//! then id 2 status, id 3 site URL and id 5 upgrade URL. A refused login
//! gets id 4 with the error and the link is closed.

use std::io;
use std::time::{Duration, Instant};

use seihai_core::protocol::seclogin::{self, ClientHandshake};
use seihai_core::protocol::{parse_list_reply, split_messages, ListReply, ServerEntry};

use crate::link::{Codec, Link};
use crate::wire::{gchar, latin1_bytes, message, push_gstring, text};

/// Client to listserver: RC list handshake (key + token).
pub const PLI_V2SERVERLISTRC: u8 = 5;
/// Client to listserver: account and password.
pub const PLI_SERVERLIST: u8 = 1;

/// The handshake body: id 5, key, token, line feed.
pub fn handshake_body(key: u8, token: &str) -> Vec<u8> {
    let mut t = token.as_bytes().to_vec();
    t.resize(8, b' ');
    let mut b = vec![gchar(PLI_V2SERVERLISTRC), gchar(key & 0x7F)];
    b.extend_from_slice(&t);
    b.push(b'\n');
    b
}

/// The account message: id 1, account, password.
pub fn account_message(account: &str, password: &str) -> Vec<u8> {
    let mut p = Vec::new();
    push_gstring(&mut p, &latin1_bytes(account));
    push_gstring(&mut p, &latin1_bytes(password));
    message(PLI_SERVERLIST, &p)
}

/// Why the RC list could not be read.
#[derive(Debug)]
pub enum ListError {
    Io(io::Error),
    /// The listserver sent an error (id 4).
    Refused(String),
    /// No list came before the link closed or the timeout passed.
    NoList,
    /// The secure handshake failed, or the pinned key did not match.
    /// Nothing after C1 was sent when the key did not match.
    Secure(String),
}

impl std::fmt::Display for ListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ListError::Io(e) => write!(f, "{e}"),
            ListError::Refused(why) => write!(f, "listserver refused the login: {why}"),
            ListError::NoList => write!(f, "the listserver sent no server list"),
            ListError::Secure(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for ListError {}

impl From<io::Error> for ListError {
    fn from(e: io::Error) -> Self {
        ListError::Io(e)
    }
}

/// Log in to the listserver at `addr` and read the RC server list.
///
/// After the list arrives, waits up to `quiet` for the status and URL
/// messages that follow it, then closes the link.
pub fn fetch_rc_list(
    addr: &str,
    token: &str,
    account: &str,
    password: &str,
    key: u8,
    timeout: Duration,
) -> Result<ListReply, ListError> {
    let deadline = Instant::now() + timeout;
    let mut link = Link::connect(addr, timeout)?;
    link.send_login(&handshake_body(key, token), Codec::gen5(key & 0x7F))?;
    link.send(&[account_message(account, password)])?;
    let reply = read_reply(&mut link, deadline);
    link.shutdown();
    reply
}

/// The id 1 payload sealed in C2: `GSTRING` account, `GSTRING` password.
/// The id byte itself stays out; it already went in the classic message.
fn account_tail(account: &str, password: &str) -> Vec<u8> {
    let mut tail = Vec::new();
    push_gstring(&mut tail, &latin1_bytes(account));
    push_gstring(&mut tail, &latin1_bytes(password));
    tail
}

/// An unpadded base64 Ed25519 public key, as `--server-key` prints it.
pub fn decode_server_key(text: &str) -> Option<[u8; 32]> {
    seclogin::decode_key(text)
}

/// Log in through the secure handshake and read the RC server list.
///
/// C1 is a plain zlib frame: id 5, the key byte, `SEI00001`, the RC token,
/// the client's X25519 key. There is no list name after that. C2 is a
/// second plain zlib frame sealing [`account_tail`]. The player-list form
/// (id 7 and a list name) is not sent. After C2 the link is gen 5 with
/// `key`, and the reply is the same one [`fetch_rc_list`] reads.
///
/// `server_key` is the listserver's Ed25519 public key. On a mismatch this
/// returns [`ListError::Secure`] and does not send C2.
pub fn fetch_rc_list_secure(
    addr: &str,
    token: &str,
    account: &str,
    password: &str,
    key: u8,
    server_key: &[u8; 32],
    timeout: Duration,
) -> Result<ListReply, ListError> {
    let deadline = Instant::now() + timeout;
    let key = key & 0x7F;
    let mut link = Link::connect(addr, timeout)?;
    let prefix = [gchar(PLI_V2SERVERLISTRC), gchar(key)];
    let handshake =
        ClientHandshake::new(&prefix, token, b"").map_err(|e| ListError::Secure(e.to_string()))?;
    let mut c1 = handshake.hello().to_vec();
    c1.push(b'\n');
    link.send_plain(&c1)?;
    let Some(body) = link.recv_body(deadline)? else {
        link.shutdown();
        return Err(ListError::NoList);
    };
    let line = body
        .split(|b| *b == b'\n')
        .next()
        .unwrap_or(body.as_slice());
    if !line.starts_with(seclogin::TOKEN.as_bytes()) {
        let refused = split_messages(&body).into_iter().find(|m| m.id == 4);
        link.shutdown();
        return match refused {
            Some(m) => Err(ListError::Refused(text(&m.payload))),
            None => Err(ListError::Secure(
                "the listserver did not send a secure hello".into(),
            )),
        };
    }
    let verified = handshake
        .finish(line)
        .map_err(|e| ListError::Secure(e.to_string()))?;
    if verified.server_key != *server_key {
        link.shutdown();
        return Err(ListError::Secure(format!(
            "server key {} is not the given key {}; not sending the password",
            seclogin::fingerprint(&verified.server_key),
            seclogin::fingerprint(server_key),
        )));
    }
    let mut c2 = verified
        .sealer
        .seal(&account_tail(account, password))
        .map_err(|e| ListError::Secure(e.to_string()))?;
    c2.push(b'\n');
    link.send_plain(&c2)?;
    link.set_codec(Codec::gen5(key));
    let reply = read_reply(&mut link, deadline);
    link.shutdown();
    reply
}

/// Read the gen-5 RC list. The link is already on that codec.
fn read_reply(link: &mut Link, deadline: Instant) -> Result<ListReply, ListError> {
    let mut reply = ListReply::default();
    let mut got_list = false;
    let quiet = Duration::from_millis(300);
    loop {
        let until = if got_list {
            (Instant::now() + quiet).min(deadline)
        } else {
            deadline
        };
        let Some(msgs) = link.recv(until)? else { break };
        let mut body = Vec::new();
        for m in msgs {
            got_list |= m.id == 0;
            body.extend_from_slice(&message(m.id, &m.payload));
            body.push(b'\n');
        }
        let part = parse_list_reply(&body);
        reply.servers.extend(part.servers);
        reply.status = part.status.or(reply.status);
        reply.site_url = part.site_url.or(reply.site_url);
        reply.upgrade_url = part.upgrade_url.or(reply.upgrade_url);
        if let Some(err) = part.error {
            return Err(ListError::Refused(err));
        }
    }
    if got_list {
        Ok(reply)
    } else {
        Err(ListError::NoList)
    }
}

/// Pick a server from the list: by its index (from 1, as listed), or by
/// name (exact, ignoring case, then the only name that contains it).
pub fn pick<'a>(servers: &'a [ServerEntry], which: &str) -> Option<&'a ServerEntry> {
    if let Ok(n) = which.parse::<usize>() {
        return n.checked_sub(1).and_then(|i| servers.get(i));
    }
    let w = which.to_lowercase();
    if let Some(s) = servers.iter().find(|s| s.name.to_lowercase() == w) {
        return Some(s);
    }
    let mut hits = servers
        .iter()
        .filter(|s| s.name.to_lowercase().contains(&w));
    match (hits.next(), hits.next()) {
        (Some(s), None) => Some(s),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_bytes() {
        assert_eq!(handshake_body(0x2A, "GSERV025"), b"%JGSERV025\n");
    }

    #[test]
    fn account_bytes() {
        assert_eq!(account_message("a", "bc"), b"!!a\"bc");
    }

    fn entry(name: &str) -> ServerEntry {
        ServerEntry {
            name: name.into(),
            ..Default::default()
        }
    }

    #[test]
    fn pick_by_index_and_name() {
        let list = [entry("My Server"), entry("Lab Classic")];
        assert_eq!(pick(&list, "2").unwrap().name, "Lab Classic");
        assert!(pick(&list, "0").is_none());
        assert_eq!(pick(&list, "my server").unwrap().name, "My Server");
        assert_eq!(pick(&list, "classic").unwrap().name, "Lab Classic");
        // "lab" matches only one; "s" matches both and is refused.
        assert!(pick(&list, "s").is_none());
    }
}
