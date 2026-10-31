//! The RC session with a gserver: login, the messages the server sends an
//! RC, and the player list built from them.
//!
//! Written from the Seihai protocol notes, the RC lineage notes in the
//! project files and lab runs against the gserver-v2 fork. Ids and wire
//! formats only; no server code.
//!
//! Login message (the first frame, plain zlib):
//!
//! ```text
//! GCHAR type  GCHAR key (type 6 only)  token (8 bytes)  GSTRING account  GSTRING password
//! ```
//!
//! - type 1: the old RCs (2000 to 2002). Plain zlib frames for the whole
//!   session, no key.
//! - type 6: the newer RCs. A key byte follows the type, and every frame
//!   after the login is gen 5.
//!
//! What the server sends after a good login, in the lab with the fork:
//! `SIGNATURE` (25), RC chat lines (74), `ADDPLAYER` (55) for every player
//! and RC already online, then live `ADDPLAYER` / `DELPLAYER` (56) and chat.
//! A refused login gets `DISCMESSAGE` (16) and the link is closed.

use std::collections::BTreeMap;
use std::io;
use std::time::{Duration, Instant};

use seihai_core::protocol::{read_gshort, Message};

use crate::link::{Codec, Link};
use crate::wire::{gchar, latin1_bytes, message, push_gstring, read_gstring, text};

/// Client to server: player props (the RC sends its nickname this way).
pub const PLI_PLAYERPROPS: u8 = 2;
/// Client to server: a line in the RC chat.
pub const PLI_RC_CHAT: u8 = 79;

/// Server to client ids an RC handles.
pub const PLO_DISCMESSAGE: u8 = 16;
pub const PLO_SIGNATURE: u8 = 25;
pub const PLO_RC_ADMINMESSAGE: u8 = 35;
pub const PLO_PRIVATEMESSAGE: u8 = 37;
pub const PLO_ADDPLAYER: u8 = 55;
pub const PLO_DELPLAYER: u8 = 56;
pub const PLO_RC_CHAT: u8 = 74;

/// Player prop ids carried in `ADDPLAYER`.
pub const PROP_NICKNAME: u8 = 0;
pub const PROP_LEVEL: u8 = 20;
pub const PROP_PLAYERLISTSTATUS: u8 = 53;
pub const PROP_COMMUNITYNAME: u8 = 82;

/// Which RC login to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RcKind {
    /// Type 1, plain zlib frames (RCs from 2000 to 2002).
    Classic,
    /// Type 6, gen-5 frames (the newer RCs). The default.
    Gen5,
}

impl RcKind {
    /// The login type value.
    pub fn login_type(self) -> u8 {
        match self {
            RcKind::Classic => 1,
            RcKind::Gen5 => 6,
        }
    }

    /// The version token sent by default: the newest token known for each
    /// kind (`GSERV023` is the 2002/05/02 RC; `GSERV025` the newest
    /// type-6 token the fork knows).
    pub fn default_token(self) -> &'static str {
        match self {
            RcKind::Classic => "GSERV023",
            RcKind::Gen5 => "GSERV025",
        }
    }

    /// Parse `classic` or `gen5`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "classic" | "1" => Some(RcKind::Classic),
            "gen5" | "6" => Some(RcKind::Gen5),
            _ => None,
        }
    }
}

/// What to log in with.
#[derive(Debug, Clone)]
pub struct LoginParams {
    pub kind: RcKind,
    /// Eight-byte version token. Padded or cut to 8 bytes.
    pub token: String,
    pub account: String,
    pub password: String,
    /// Gen-5 key value (0 to 127). Ignored for [`RcKind::Classic`].
    pub key: u8,
    /// Nickname to send after the login, if any. With none the server
    /// shows `*account`.
    pub nickname: Option<String>,
}

/// The login message body for `p`, line feed included.
pub fn login_body(p: &LoginParams) -> Vec<u8> {
    let mut b = vec![gchar(p.kind.login_type())];
    if p.kind == RcKind::Gen5 {
        b.push(gchar(p.key & 0x7F));
    }
    let mut token = p.token.as_bytes().to_vec();
    token.resize(8, b' ');
    b.extend_from_slice(&token);
    push_gstring(&mut b, &latin1_bytes(&p.account));
    push_gstring(&mut b, &latin1_bytes(&p.password));
    b.push(b'\n');
    b
}

/// The nickname message: `PLAYERPROPS` with the nickname prop.
pub fn nickname_message(nick: &str) -> Vec<u8> {
    let mut payload = vec![gchar(PROP_NICKNAME)];
    push_gstring(&mut payload, &latin1_bytes(nick));
    message(PLI_PLAYERPROPS, &payload)
}

/// An RC chat line.
pub fn rc_chat_message(line: &str) -> Vec<u8> {
    message(PLI_RC_CHAT, &latin1_bytes(line))
}

/// One player or RC in the server's list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayerEntry {
    pub id: u16,
    pub account: String,
    pub nickname: String,
    pub level: String,
    pub status: u8,
    pub community: String,
}

impl PlayerEntry {
    /// True when the entry looks like an RC: the server gives RCs a level
    /// of a single space, and players always have a level name.
    pub fn is_rc(&self) -> bool {
        self.level.trim().is_empty()
    }
}

/// A message from the server, as far as stage 1 reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The server accepted the login.
    Signature,
    /// The server refused or ended the session.
    Disconnect(String),
    /// A line in the RC chat.
    RcChat(String),
    /// An admin message (`title§text`).
    AdminMessage(String),
    /// A private message from player `from`.
    PrivateMessage { from: u16, text: String },
    /// A player or RC is online, or its listed props changed.
    AddPlayer(PlayerEntry),
    /// A player or RC left.
    DelPlayer(u16),
    /// Anything else (id, payload).
    Other(u8, Vec<u8>),
}

/// Read one server message.
pub fn parse_event(m: &Message) -> Event {
    let p = &m.payload;
    match m.id {
        PLO_SIGNATURE => Event::Signature,
        PLO_DISCMESSAGE => Event::Disconnect(text(p)),
        PLO_RC_CHAT => Event::RcChat(text(p)),
        PLO_RC_ADMINMESSAGE => Event::AdminMessage(text(p)),
        PLO_PRIVATEMESSAGE => match parse_pm(p) {
            Some(e) => e,
            None => Event::Other(m.id, p.clone()),
        },
        PLO_ADDPLAYER => match parse_addplayer(p) {
            Some(e) => Event::AddPlayer(e),
            None => Event::Other(m.id, p.clone()),
        },
        PLO_DELPLAYER => match read_gshort(p) {
            Some(id) => Event::DelPlayer(id),
            None => Event::Other(m.id, p.clone()),
        },
        id => Event::Other(id, p.clone()),
    }
}

/// `PRIVATEMESSAGE`: GSHORT sender, then the text. The text is a quoted,
/// comma-separated list as the server tokenizes it; shown as received.
fn parse_pm(p: &[u8]) -> Option<Event> {
    let from = read_gshort(p)?;
    Some(Event::PrivateMessage {
        from,
        text: text(&p[2..]),
    })
}

/// `ADDPLAYER`: GSHORT id, GSTRING account, then props as `GCHAR id` +
/// value. The props read here are nickname, level, community name (all
/// GSTRING) and list status (GCHAR). Reading stops at any other prop.
pub fn parse_addplayer(p: &[u8]) -> Option<PlayerEntry> {
    let id = read_gshort(p)?;
    let mut i = 2;
    let account = text(&read_gstring(p, &mut i)?);
    let mut e = PlayerEntry {
        id,
        account,
        ..Default::default()
    };
    while let Some(&b) = p.get(i) {
        i += 1;
        match b.wrapping_sub(0x20) {
            PROP_NICKNAME => e.nickname = text(&read_gstring(p, &mut i)?),
            PROP_LEVEL => e.level = text(&read_gstring(p, &mut i)?),
            PROP_COMMUNITYNAME => e.community = text(&read_gstring(p, &mut i)?),
            PROP_PLAYERLISTSTATUS => {
                e.status = p.get(i)?.wrapping_sub(0x20);
                i += 1;
            }
            _ => break,
        }
    }
    Some(e)
}

/// The players and RCs online, by id.
#[derive(Debug, Clone, Default)]
pub struct PlayerList {
    pub players: BTreeMap<u16, PlayerEntry>,
}

impl PlayerList {
    /// Apply an event; true when the list changed.
    pub fn apply(&mut self, ev: &Event) -> bool {
        match ev {
            Event::AddPlayer(e) => self.players.insert(e.id, e.clone()).as_ref() != Some(e),
            Event::DelPlayer(id) => self.players.remove(id).is_some(),
            _ => false,
        }
    }
}

/// Why a login failed.
#[derive(Debug)]
pub enum LoginError {
    Io(io::Error),
    /// The server sent a disconnect message.
    Refused(String),
    /// The link closed with no message.
    Closed,
    /// No answer before the timeout.
    Timeout,
}

impl std::fmt::Display for LoginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoginError::Io(e) => write!(f, "{e}"),
            LoginError::Refused(why) => write!(f, "refused: {why}"),
            LoginError::Closed => write!(f, "the server closed the link"),
            LoginError::Timeout => write!(f, "no answer from the server"),
        }
    }
}

impl std::error::Error for LoginError {}

impl From<io::Error> for LoginError {
    fn from(e: io::Error) -> Self {
        LoginError::Io(e)
    }
}

/// A logged-in RC session.
pub struct Session {
    link: Link,
    /// Events read during login that the caller has not taken yet.
    pending: Vec<Event>,
    pub players: PlayerList,
}

impl Session {
    /// Connect to `addr`, log in, and wait for the server's answer.
    ///
    /// The gserver checks the account with its listserver first, so the
    /// answer can take a moment; `timeout` covers the connect and the
    /// wait.
    pub fn login(addr: &str, p: &LoginParams, timeout: Duration) -> Result<Self, LoginError> {
        let deadline = Instant::now() + timeout;
        let mut link = Link::connect(addr, timeout)?;
        let codec = match p.kind {
            RcKind::Classic => Codec::Plain,
            RcKind::Gen5 => Codec::gen5(p.key & 0x7F),
        };
        link.send_login(&login_body(p), codec)?;
        let mut s = Self {
            link,
            pending: Vec::new(),
            players: PlayerList::default(),
        };
        loop {
            let Some(msgs) = s.link.recv(deadline)? else {
                return Err(if s.link.is_closed() {
                    LoginError::Closed
                } else {
                    LoginError::Timeout
                });
            };
            let mut ok = false;
            for m in &msgs {
                let ev = parse_event(m);
                if let Event::Disconnect(why) = ev {
                    return Err(LoginError::Refused(why));
                }
                ok |= ev == Event::Signature;
                s.players.apply(&ev);
                s.pending.push(ev);
            }
            if ok {
                break;
            }
        }
        if let Some(nick) = &p.nickname {
            s.link.send(&[nickname_message(nick)])?;
        }
        Ok(s)
    }

    /// The next event, waiting at most until `deadline`. `Ok(None)` when
    /// the deadline passed or the server closed the link (see
    /// [`Self::is_closed`]).
    pub fn next_event(&mut self, deadline: Instant) -> io::Result<Option<Event>> {
        loop {
            if !self.pending.is_empty() {
                return Ok(Some(self.pending.remove(0)));
            }
            let Some(msgs) = self.link.recv(deadline)? else {
                return Ok(None);
            };
            for m in &msgs {
                let ev = parse_event(m);
                self.players.apply(&ev);
                self.pending.push(ev);
            }
        }
    }

    /// Read events until none has come for `quiet`, or until `deadline`.
    /// Used to let the login burst (the player list) arrive in full: the
    /// server sends no end marker. Returns the events read.
    pub fn settle(&mut self, quiet: Duration, deadline: Instant) -> io::Result<Vec<Event>> {
        let mut out = Vec::new();
        loop {
            let until = (Instant::now() + quiet).min(deadline);
            match self.next_event(until)? {
                Some(ev) => out.push(ev),
                None => return Ok(out),
            }
            if Instant::now() >= deadline {
                return Ok(out);
            }
        }
    }

    /// Send a line in the RC chat.
    pub fn send_rc_chat(&mut self, line: &str) -> io::Result<()> {
        self.link.send(&[rc_chat_message(line)])
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

    fn params(kind: RcKind) -> LoginParams {
        LoginParams {
            kind,
            token: kind.default_token().into(),
            account: "admin".into(),
            password: "pw".into(),
            key: 0x2A,
            nickname: None,
        }
    }

    #[test]
    fn gen5_login_body() {
        assert_eq!(login_body(&params(RcKind::Gen5)), b"&JGSERV025%admin\"pw\n");
    }

    #[test]
    fn classic_login_body_has_no_key() {
        assert_eq!(
            login_body(&params(RcKind::Classic)),
            b"!GSERV023%admin\"pw\n"
        );
    }

    #[test]
    fn short_token_is_padded() {
        let mut p = params(RcKind::Classic);
        p.token = "AB".into();
        assert_eq!(&login_body(&p)[1..9], b"AB      ");
    }

    #[test]
    fn nickname_message_bytes() {
        assert_eq!(nickname_message("Staff"), b"\" %Staff");
    }

    #[test]
    fn addplayer_reads_rc_props() {
        // id 3, account "bob", level "a.nw", status 2, nick "Bob", community "bob"
        let mut p = vec![0x20, 0x23];
        push_gstring(&mut p, b"bob");
        p.push(gchar(PROP_LEVEL));
        push_gstring(&mut p, b"a.nw");
        p.push(gchar(PROP_PLAYERLISTSTATUS));
        p.push(gchar(2));
        p.push(gchar(PROP_NICKNAME));
        push_gstring(&mut p, b"Bob");
        p.push(gchar(PROP_COMMUNITYNAME));
        push_gstring(&mut p, b"bob");
        let e = parse_addplayer(&p).unwrap();
        assert_eq!(
            e,
            PlayerEntry {
                id: 3,
                account: "bob".into(),
                nickname: "Bob".into(),
                level: "a.nw".into(),
                status: 2,
                community: "bob".into()
            }
        );
        assert!(!e.is_rc());
    }

    #[test]
    fn list_add_and_remove() {
        let mut l = PlayerList::default();
        let e = PlayerEntry {
            id: 7,
            account: "x".into(),
            level: " ".into(),
            ..Default::default()
        };
        assert!(l.apply(&Event::AddPlayer(e.clone())));
        assert!(!l.apply(&Event::AddPlayer(e)));
        assert!(l.players[&7].is_rc());
        assert!(l.apply(&Event::DelPlayer(7)));
        assert!(l.players.is_empty());
    }

    #[test]
    fn gshort_id_over_127() {
        let mut p = vec![0x20 + 1, 0x20 + 2]; // (1 << 7) | 2 = 130
        push_gstring(&mut p, b"a");
        assert_eq!(parse_addplayer(&p).unwrap().id, 130);
    }
}
