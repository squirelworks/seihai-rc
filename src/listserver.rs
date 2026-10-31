//! The listserver's RC list: every server, type-tagged.
//!
//! Login shape (gen 5, the same key for both directions):
//!
//! 1. A plain zlib frame with id 5 = `GCHAR key` + the RC's 8-byte token.
//! 2. A gen-5 frame with id 1 = `GSTRING account` + `GSTRING password`.
//!
//! The listserver answers with gen-5 frames: id 0 the server list (as for
//! the 2.22 client list, names carrying a type tag such as `P ` or `H `),
//! then id 2 status, id 3 site URL and id 5 upgrade URL. A refused login
//! gets id 4 with the error and the link is closed.

use std::io;
use std::time::{Duration, Instant};

use seihai_core::protocol::{parse_list_reply, ListReply, ServerEntry};

use crate::link::{Codec, Link};
use crate::wire::{gchar, latin1_bytes, message, push_gstring};

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
}

impl std::fmt::Display for ListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ListError::Io(e) => write!(f, "{e}"),
            ListError::Refused(why) => write!(f, "listserver refused the login: {why}"),
            ListError::NoList => write!(f, "the listserver sent no server list"),
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
            link.shutdown();
            return Err(ListError::Refused(err));
        }
    }
    link.shutdown();
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
