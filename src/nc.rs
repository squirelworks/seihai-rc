//! NPC-Control session: a second socket, beside the RC link.
//!
//! The 2002 and 2004 staff clients log in here as type 3 with the token
//! `NCL11012`, then keep plain zlib frames (`u16be` length plus a zlib body)
//! for the whole session: no key byte, no compression-type byte, no cipher.
//! Behaviour is taken from Graal_analysis `protocol/npc-control.md`. This
//! module does not copy that note. Payloads that note still leaves for a
//! lab check are not given a layout here.
//!
//! Nothing is sent after the password. A loopback capture may still show an
//! extra byte; until that run, this client does not add one.

use std::collections::BTreeMap;
use std::io;
use std::time::{Duration, Instant};

use seihai_core::protocol::{read_gint3, read_gshort, Message};

use crate::link::{Codec, Link};
use crate::rc::LoginError;
use crate::wire::{gchar, latin1_bytes, message, push_gint3, push_gstring, read_gstring, text};

/// Client type on the NPC-Control socket.
pub const NC_LOGIN_TYPE: u8 = 3;
/// Eight-byte version token. Unchanged from the 2002 client to the 2004 one.
pub const NC_TOKEN: &str = "NCL11012";

/// Largest value a GINT3 can carry.
pub const GINT3_MAX: u32 = (1 << 21) - 1;
const GSTRING_MAX: usize = 223;

/// Server to client: the session was refused or ended. Payload is text.
pub const NC_DISCMESSAGE: u8 = 16;
/// Server to client: one weapon script.
pub const NC_WEAPON_SCRIPT: u8 = 33;
/// Server to client: attribute text for the NPC just asked about.
pub const NC_NPC_ATTRIBUTES: u8 = 157;
/// Server to client: one NPC row, added or updated.
pub const NC_NPC_ROW: u8 = 158;
/// Server to client: drop one NPC row.
pub const NC_NPC_REMOVED: u8 = 159;
/// Server to client: one NPC script.
pub const NC_NPC_SCRIPT: u8 = 160;
/// Server to client: one class script.
pub const NC_CLASS_SCRIPT: u8 = 162;
/// Server to client: one class name.
pub const NC_CLASS_NAME: u8 = 163;
/// Server to client: the weapon list.
pub const NC_WEAPON_NAMES: u8 = 167;
/// Server to client: a class was deleted. `GSTRING` name.
pub const NC_CLASS_DELETE_NOTIFY: u8 = 188;

/// Client to server: ask for an NPC's attributes.
pub const NC_NPC_GET: u8 = 103;
/// Client to server: delete an NPC.
pub const NC_NPC_DELETE: u8 = 104;
/// Client to server: ask for an NPC script.
pub const NC_NPC_SCRIPT_GET: u8 = 106;
/// Client to server: save an NPC script.
pub const NC_NPC_SCRIPT_SET: u8 = 109;
/// Client to server: open a class.
pub const NC_CLASS_GET: u8 = 112;
/// Client to server: save a class.
pub const NC_CLASS_SET: u8 = 113;
/// Client to server: ask for the weapon list.
pub const NC_WEAPON_LIST_GET: u8 = 115;
/// Client to server: open a weapon.
pub const NC_WEAPON_GET: u8 = 116;
/// Client to server: save a weapon.
pub const NC_WEAPON_SET: u8 = 117;
/// Client to server: delete a weapon.
pub const NC_WEAPON_DELETE: u8 = 118;
/// Client to server: delete a class. `GSTRING` name, as in [`class_set_message`].
pub const NC_CLASS_DELETE: u8 = 119;

/// Property ids inside an NPC row.
pub const NPC_PROP_NAME: u8 = 50;
pub const NPC_PROP_TYPE: u8 = 51;
pub const NPC_PROP_LEVEL: u8 = 52;

/// Why a builder refused to produce bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NcError {
    /// The notes name the field and still disagree about its bytes.
    NeedsLabCheck {
        packet: &'static str,
        field: &'static str,
    },
    /// A line feed would end the message, and the raw-data form is not used.
    LineBreak(&'static str),
    /// A GSTRING length is one GCHAR, so 223 bytes is the limit.
    TooLong(&'static str),
    /// The id does not fit in a GINT3.
    BadId,
}

impl std::fmt::Display for NcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NcError::NeedsLabCheck { packet, field } => write!(
                f,
                "{packet}: {field} still needs a lab check; nothing was sent"
            ),
            NcError::LineBreak(what) => write!(f, "{what} contains a line break"),
            NcError::TooLong(what) => {
                write!(f, "{what} is longer than a GSTRING ({GSTRING_MAX} bytes)")
            }
            NcError::BadId => write!(f, "NPC id does not fit in a GINT3"),
        }
    }
}

impl std::error::Error for NcError {}

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

fn check_id(id: u32) -> Result<(), NcError> {
    if id > GINT3_MAX {
        Err(NcError::BadId)
    } else {
        Ok(())
    }
}

fn no_line_break(what: &'static str, value: &str) -> Result<(), NcError> {
    if value.chars().any(|c| c == '\n' || c == '\r') {
        Err(NcError::LineBreak(what))
    } else {
        Ok(())
    }
}

/// A script is the rest of one message. A line break ends that message.
/// The notes still need a lab check before a raw-data frame is used instead.
fn script_bytes(script: &str) -> Result<Vec<u8>, NcError> {
    if script.chars().any(|c| c == '\n' || c == '\r') {
        Err(NcError::NeedsLabCheck {
            packet: "script set (109, 113, 117)",
            field: "a script that contains a line break, and the raw-data path for a large set",
        })
    } else {
        Ok(latin1_bytes(script))
    }
}

fn gstring_bytes(what: &'static str, value: &str) -> Result<Vec<u8>, NcError> {
    no_line_break(what, value)?;
    let bytes = latin1_bytes(value);
    if bytes.len() > GSTRING_MAX {
        Err(NcError::TooLong(what))
    } else {
        Ok(bytes)
    }
}

fn gint3(id: u32) -> Vec<u8> {
    let mut payload = Vec::with_capacity(3);
    push_gint3(&mut payload, id);
    payload
}

/// Id 103: GINT3 id.
pub fn npc_get_message(id: u32) -> Result<Vec<u8>, NcError> {
    check_id(id)?;
    Ok(message(NC_NPC_GET, &gint3(id)))
}

/// Id 104: GINT3 id.
pub fn npc_delete_message(id: u32) -> Result<Vec<u8>, NcError> {
    check_id(id)?;
    Ok(message(NC_NPC_DELETE, &gint3(id)))
}

/// Id 106: GINT3 id.
pub fn npc_script_get_message(id: u32) -> Result<Vec<u8>, NcError> {
    check_id(id)?;
    Ok(message(NC_NPC_SCRIPT_GET, &gint3(id)))
}

/// Id 109: GINT3 id, then the script.
pub fn npc_script_set_message(id: u32, script: &str) -> Result<Vec<u8>, NcError> {
    check_id(id)?;
    let mut payload = gint3(id);
    payload.extend(script_bytes(script)?);
    Ok(message(NC_NPC_SCRIPT_SET, &payload))
}

/// Id 112: the class name as the rest of the line.
///
/// The notes write `str` where they saw a length byte. They do not write it
/// for this packet. A lab check should confirm it is not a GSTRING.
pub fn class_get_message(name: &str) -> Result<Vec<u8>, NcError> {
    no_line_break("class name", name)?;
    Ok(message(NC_CLASS_GET, &latin1_bytes(name)))
}

/// Id 113: GSTRING name, then the script.
pub fn class_set_message(name: &str, script: &str) -> Result<Vec<u8>, NcError> {
    let mut payload = Vec::new();
    push_gstring(&mut payload, &gstring_bytes("class name", name)?);
    payload.extend(script_bytes(script)?);
    Ok(message(NC_CLASS_SET, &payload))
}

/// Id 119: `GSTRING` name, the same string form as [`class_set_message`].
///
/// A bare class name (the rest of the line, as id 112 sends) is the other
/// reading still open. It is not sent.
pub fn class_delete_message(name: &str) -> Result<Vec<u8>, NcError> {
    let mut payload = Vec::new();
    push_gstring(&mut payload, &gstring_bytes("class name", name)?);
    Ok(message(NC_CLASS_DELETE, &payload))
}

/// Id 188: `GSTRING` name, and only when that string is the whole payload.
///
/// A bare name is not read.
pub fn parse_class_delete(payload: &[u8]) -> Option<String> {
    let mut i = 0;
    let name = text(&read_gstring(payload, &mut i)?);
    if i != payload.len() {
        return None;
    }
    Some(name)
}

/// Id 115: no payload.
pub fn weapon_list_message() -> Vec<u8> {
    message(NC_WEAPON_LIST_GET, b"")
}

/// Id 116: the weapon name as the rest of the line.
///
/// Same reading as [`class_get_message`]: the notes do not write `str` here,
/// and a lab check should confirm there is no length byte.
pub fn weapon_get_message(name: &str) -> Result<Vec<u8>, NcError> {
    no_line_break("weapon name", name)?;
    Ok(message(NC_WEAPON_GET, &latin1_bytes(name)))
}

/// Id 117: GSTRING name, GSTRING image, then the script.
pub fn weapon_set_message(name: &str, image: &str, script: &str) -> Result<Vec<u8>, NcError> {
    let mut payload = Vec::new();
    push_gstring(&mut payload, &gstring_bytes("weapon name", name)?);
    push_gstring(&mut payload, &gstring_bytes("weapon image", image)?);
    payload.extend(script_bytes(script)?);
    Ok(message(NC_WEAPON_SET, &payload))
}

/// Id 118: the weapon name as the rest of the line. See [`weapon_get_message`].
pub fn weapon_delete_message(name: &str) -> Result<Vec<u8>, NcError> {
    no_line_break("weapon name", name)?;
    Ok(message(NC_WEAPON_DELETE, &latin1_bytes(name)))
}

/// One NPC row from id 158. Absent properties were not in the payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpcRow {
    pub id: u32,
    pub name: Option<String>,
    pub kind: Option<String>,
    pub level: Option<String>,
}

/// A weapon script from id 33.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponScript {
    pub name: String,
    pub image: String,
    pub script: String,
}

/// Id 158. Properties may arrive in any order and any subset of name (50),
/// type (51) and level (52), each a GCHAR id plus a GSTRING. Any other
/// property id fails the parse: its width is not in the notes, so the whole
/// payload stays raw.
pub fn parse_npc_add(payload: &[u8]) -> Option<NpcRow> {
    let id = read_gint3(payload)? as u32;
    let mut i = 3;
    let mut row = NpcRow {
        id,
        name: None,
        kind: None,
        level: None,
    };
    while i < payload.len() {
        let prop = payload.get(i)?.wrapping_sub(0x20);
        i += 1;
        let value = text(&read_gstring(payload, &mut i)?);
        match prop {
            NPC_PROP_NAME => row.name = Some(value),
            NPC_PROP_TYPE => row.kind = Some(value),
            NPC_PROP_LEVEL => row.level = Some(value),
            _ => return None,
        }
    }
    Some(row)
}

/// Id 159, and only when the payload is exactly the GINT3. Extra bytes still
/// need a lab check.
pub fn parse_id_only(payload: &[u8]) -> Option<u32> {
    if payload.len() != 3 {
        return None;
    }
    read_gint3(payload).map(|n| n as u32)
}

/// Id 160: GINT3 id, then the script.
pub fn parse_npc_script(payload: &[u8]) -> Option<(u32, String)> {
    let id = read_gint3(payload)? as u32;
    Some((id, text(&payload[3..])))
}

/// Id 162: GSTRING name, then the script.
pub fn parse_class_script(payload: &[u8]) -> Option<(String, String)> {
    let mut i = 0;
    let name = text(&read_gstring(payload, &mut i)?);
    Some((name, text(&payload[i..])))
}

/// Id 167: repeated GSTRING names. A truncated string fails the whole list.
pub fn parse_weapon_names(payload: &[u8]) -> Option<Vec<String>> {
    let mut i = 0;
    let mut names = Vec::new();
    while i < payload.len() {
        names.push(text(&read_gstring(payload, &mut i)?));
    }
    Some(names)
}

/// Id 33: GSTRING name, GSTRING image, GSHORT length, then that many script
/// bytes. A length that runs past the message fails. A GINT3 length, which
/// the notes still need a lab check for, is not tried.
pub fn parse_weapon_script(payload: &[u8]) -> Option<WeaponScript> {
    let mut i = 0;
    let name = text(&read_gstring(payload, &mut i)?);
    let image = text(&read_gstring(payload, &mut i)?);
    let len = usize::from(read_gshort(payload.get(i..)?)?);
    i += 2;
    let script = text(payload.get(i..i + len)?);
    if i + len != payload.len() {
        return None;
    }
    Some(WeaponScript {
        name,
        image,
        script,
    })
}

/// A message on the NPC-Control socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NcEvent {
    /// The server refused or ended the session.
    Disconnect(String),
    /// Id 158.
    Npc(NpcRow),
    /// Id 159.
    NpcRemoved(u32),
    /// Id 157. The rest of the line; no NPC id is in this payload.
    Attributes(String),
    /// Id 160.
    NpcScript { id: u32, script: String },
    /// Id 163. One class name, the rest of the line.
    Class(String),
    /// Id 162.
    ClassScript { name: String, script: String },
    /// Id 167.
    Weapons(Vec<String>),
    /// Id 33.
    Weapon(WeaponScript),
    /// Id 188. The class name.
    ClassDeleted(String),
    /// Any other message, or a known id whose payload did not match.
    Message { id: u8, payload: Vec<u8> },
}

/// Read one server message.
pub fn parse_event(m: &Message) -> NcEvent {
    let p = &m.payload;
    match m.id {
        NC_DISCMESSAGE => NcEvent::Disconnect(text(p)),
        NC_WEAPON_SCRIPT => match parse_weapon_script(p) {
            Some(weapon) => NcEvent::Weapon(weapon),
            None => NcEvent::Message {
                id: m.id,
                payload: p.clone(),
            },
        },
        NC_NPC_ATTRIBUTES => NcEvent::Attributes(text(p)),
        NC_NPC_ROW => match parse_npc_add(p) {
            Some(row) => NcEvent::Npc(row),
            None => NcEvent::Message {
                id: m.id,
                payload: p.clone(),
            },
        },
        NC_NPC_REMOVED => match parse_id_only(p) {
            Some(id) => NcEvent::NpcRemoved(id),
            None => NcEvent::Message {
                id: m.id,
                payload: p.clone(),
            },
        },
        NC_NPC_SCRIPT => match parse_npc_script(p) {
            Some((id, script)) => NcEvent::NpcScript { id, script },
            None => NcEvent::Message {
                id: m.id,
                payload: p.clone(),
            },
        },
        NC_CLASS_SCRIPT => match parse_class_script(p) {
            Some((name, script)) => NcEvent::ClassScript { name, script },
            None => NcEvent::Message {
                id: m.id,
                payload: p.clone(),
            },
        },
        NC_CLASS_NAME => NcEvent::Class(text(p)),
        NC_WEAPON_NAMES => match parse_weapon_names(p) {
            Some(names) => NcEvent::Weapons(names),
            None => NcEvent::Message {
                id: m.id,
                payload: p.clone(),
            },
        },
        NC_CLASS_DELETE_NOTIFY => match parse_class_delete(p) {
            Some(name) => NcEvent::ClassDeleted(name),
            None => NcEvent::Message {
                id: m.id,
                payload: p.clone(),
            },
        },
        _ => NcEvent::Message {
            id: m.id,
            payload: p.clone(),
        },
    }
}

/// NPC rows, class names and the latest weapon list.
///
/// A missing property on a later id 158 leaves the stored field. Whether the
/// server instead clears that field still needs a lab check. Each id 167
/// replaces the weapon list. Whether a second one is a full list or an
/// addition still needs a lab check; the notes describe the packet as the
/// list. Id 188 removes a class whose name matches.
#[derive(Debug, Clone, Default)]
pub struct NcLists {
    pub npcs: BTreeMap<u32, NpcRow>,
    pub classes: Vec<String>,
    pub weapons: Vec<String>,
}

impl NcLists {
    /// Fold one event into the lists.
    pub fn apply(&mut self, event: &NcEvent) {
        match event {
            NcEvent::Npc(row) => match self.npcs.get_mut(&row.id) {
                Some(stored) => {
                    if let Some(name) = &row.name {
                        stored.name = Some(name.clone());
                    }
                    if let Some(kind) = &row.kind {
                        stored.kind = Some(kind.clone());
                    }
                    if let Some(level) = &row.level {
                        stored.level = Some(level.clone());
                    }
                }
                None => {
                    self.npcs.insert(row.id, row.clone());
                }
            },
            NcEvent::NpcRemoved(id) => {
                self.npcs.remove(id);
            }
            NcEvent::Class(name) => self.classes.push(name.clone()),
            NcEvent::ClassDeleted(name) => self.classes.retain(|stored| stored != name),
            NcEvent::Weapons(names) => self.weapons.clone_from(names),
            _ => {}
        }
    }
}

/// A logged-in NPC-Control session. Frames stay plain zlib both ways.
pub struct NcSession {
    link: Link,
    pending: Vec<NcEvent>,
    pub lists: NcLists,
}

impl NcSession {
    /// Connect and send the type-3 login. Returns as soon as that frame is
    /// written. Call [`Self::peek`] to catch an immediate refusal.
    pub fn connect(
        addr: &str,
        account: &str,
        password: &str,
        timeout: Duration,
    ) -> Result<Self, LoginError> {
        let mut link = Link::connect(addr, timeout)?;
        link.send_login(&login_body(account, password), Codec::Plain)?;
        Ok(Self {
            link,
            pending: Vec::new(),
            lists: NcLists::default(),
        })
    }

    /// Read one frame, if one arrives before `deadline`. A disconnect message
    /// in that frame is a refusal. Other messages stay queued.
    pub fn peek(&mut self, deadline: Instant) -> Result<(), LoginError> {
        if !self.pending.is_empty() {
            return Ok(());
        }
        match self.link.recv(deadline)? {
            Some(msgs) => {
                for m in &msgs {
                    self.push_parsed(m)?;
                }
                Ok(())
            }
            None => Ok(()),
        }
    }

    fn push_parsed(&mut self, m: &Message) -> Result<(), LoginError> {
        match parse_event(m) {
            NcEvent::Disconnect(why) => Err(LoginError::Refused(why)),
            ev => {
                self.lists.apply(&ev);
                self.pending.push(ev);
                Ok(())
            }
        }
    }

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
        let mut s = Self::connect(addr, account, password, timeout)?;
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
                s.push_parsed(m)?;
                traffic = true;
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
                let ev = parse_event(m);
                self.lists.apply(&ev);
                self.pending.push(ev);
            }
        }
    }

    /// Read events until none has come for `quiet`, or until `deadline`.
    pub fn settle(&mut self, quiet: Duration, deadline: Instant) -> io::Result<Vec<NcEvent>> {
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

    /// Send messages (without line feeds) in one frame.
    pub fn send(&mut self, msgs: &[Vec<u8>]) -> io::Result<()> {
        self.link.send(msgs)
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

    #[test]
    fn npc_get_and_delete_are_a_gint3() {
        let mut id = Vec::new();
        push_gint3(&mut id, 12);
        let get = npc_get_message(12).unwrap();
        assert_eq!(get[0], gchar(NC_NPC_GET));
        assert_eq!(&get[1..], id);
        assert_eq!(&npc_delete_message(12).unwrap()[1..], id);
        assert_eq!(
            npc_script_get_message(12).unwrap()[0],
            gchar(NC_NPC_SCRIPT_GET)
        );
        assert!(npc_get_message(GINT3_MAX + 1).is_err());
    }

    #[test]
    fn npc_row_reads_props_in_any_order() {
        let mut payload = Vec::new();
        push_gint3(&mut payload, 12);
        payload.push(gchar(NPC_PROP_LEVEL));
        push_gstring(&mut payload, b"start.nw");
        payload.push(gchar(NPC_PROP_NAME));
        push_gstring(&mut payload, b"Guard");
        let row = parse_npc_add(&payload).unwrap();
        assert_eq!(
            row,
            NpcRow {
                id: 12,
                name: Some("Guard".into()),
                kind: None,
                level: Some("start.nw".into()),
            }
        );
        payload.push(gchar(99));
        payload.push(gchar(1));
        payload.push(b'x');
        assert!(parse_npc_add(&payload).is_none());
    }

    #[test]
    fn missing_npc_prop_leaves_the_stored_field() {
        let mut lists = NcLists::default();
        lists.apply(&NcEvent::Npc(NpcRow {
            id: 3,
            name: Some("Guard".into()),
            kind: Some("POLICE".into()),
            level: Some("start.nw".into()),
        }));
        lists.apply(&NcEvent::Npc(NpcRow {
            id: 3,
            name: None,
            kind: None,
            level: Some("cave.nw".into()),
        }));
        let row = &lists.npcs[&3];
        assert_eq!(row.name.as_deref(), Some("Guard"));
        assert_eq!(row.kind.as_deref(), Some("POLICE"));
        assert_eq!(row.level.as_deref(), Some("cave.nw"));
        lists.apply(&NcEvent::NpcRemoved(3));
        assert!(lists.npcs.is_empty());
    }

    #[test]
    fn class_delete_uses_the_class_set_string() {
        let set = class_set_message("Guard", "return;").unwrap();
        let delete = class_delete_message("Guard").unwrap();
        assert_eq!(delete[0], gchar(NC_CLASS_DELETE));
        // GSTRING name, the bytes class set writes before the script.
        assert_eq!(&delete[1..], b"%Guard");
        assert_eq!(&set[1..1 + delete.len() - 1], &delete[1..]);
        // A bare name is the other reading. It is not what we send.
        assert_ne!(&delete[1..], b"Guard");
        assert!(class_delete_message("a\nb").is_err());
    }

    #[test]
    fn script_line_break_is_not_sent() {
        assert!(matches!(
            npc_script_set_message(1, "a\nb"),
            Err(NcError::NeedsLabCheck { .. })
        ));
        assert!(class_set_message("A", "a\nb").is_err());
        assert!(weapon_set_message("w", "img", "a\nb").is_err());
    }

    #[test]
    fn class_and_weapon_names_use_the_note_reading() {
        let class_get = class_get_message("Guard").unwrap();
        assert_eq!(class_get[0], gchar(NC_CLASS_GET));
        assert_eq!(&class_get[1..], b"Guard");
        let set = class_set_message("Guard", "return;").unwrap();
        assert_eq!(set[0], gchar(NC_CLASS_SET));
        assert_eq!(&set[1..], b"%Guardreturn;");
        let weapon_get = weapon_get_message("sword").unwrap();
        assert_eq!(&weapon_get[1..], b"sword");
        let delete = weapon_delete_message("sword").unwrap();
        assert_eq!(delete[0], gchar(NC_WEAPON_DELETE));
        assert_eq!(&delete[1..], b"sword");
        assert_eq!(weapon_list_message(), vec![gchar(NC_WEAPON_LIST_GET)]);
    }

    #[test]
    fn weapon_script_uses_a_gshort_length() {
        let mut payload = Vec::new();
        push_gstring(&mut payload, b"sword");
        push_gstring(&mut payload, b"sword.png");
        crate::wire::push_gshort(&mut payload, 7);
        payload.extend_from_slice(b"return;");
        let weapon = parse_weapon_script(&payload).unwrap();
        assert_eq!(weapon.name, "sword");
        assert_eq!(weapon.image, "sword.png");
        assert_eq!(weapon.script, "return;");
        payload.pop();
        assert!(parse_weapon_script(&payload).is_none());
    }

    #[test]
    fn weapon_list_is_repeated_gstrings() {
        let mut payload = Vec::new();
        push_gstring(&mut payload, b"sword");
        push_gstring(&mut payload, b"bow");
        assert_eq!(parse_weapon_names(&payload).unwrap(), ["sword", "bow"]);
        payload.push(0x20 + 5);
        assert!(parse_weapon_names(&payload).is_none());
    }

    #[test]
    fn class_delete_notify_is_a_gstring_name() {
        let mut payload = Vec::new();
        push_gstring(&mut payload, b"Foo");
        let ev = parse_event(&Message {
            id: NC_CLASS_DELETE_NOTIFY,
            payload: payload.clone(),
        });
        assert_eq!(ev, NcEvent::ClassDeleted("Foo".into()));
        let mut lists = NcLists::default();
        lists.classes.push("Foo".into());
        lists.classes.push("Bar".into());
        lists.apply(&ev);
        assert_eq!(lists.classes, ["Bar"]);
        // A bare name is not read, and it does not drop a class.
        let bare = parse_event(&Message {
            id: NC_CLASS_DELETE_NOTIFY,
            payload: b"Foo".to_vec(),
        });
        assert!(matches!(bare, NcEvent::Message { id: 188, .. }));
        lists.apply(&bare);
        assert_eq!(lists.classes, ["Bar"]);
        payload.push(b'x');
        assert!(parse_class_delete(&payload).is_none());
    }

    #[test]
    fn second_weapon_list_replaces_the_stored_one() {
        let mut lists = NcLists::default();
        lists.apply(&NcEvent::Weapons(vec!["a".into()]));
        lists.apply(&NcEvent::Weapons(vec!["b".into(), "c".into()]));
        assert_eq!(lists.weapons, ["b", "c"]);
    }

    #[test]
    fn npc_removed_rejects_extra_bytes() {
        let mut payload = Vec::new();
        push_gint3(&mut payload, 9);
        assert_eq!(parse_id_only(&payload), Some(9));
        payload.push(b'x');
        assert!(parse_id_only(&payload).is_none());
        let (id, script) = parse_npc_script({
            payload.pop();
            payload.extend_from_slice(b"return;");
            &payload
        })
        .unwrap();
        assert_eq!((id, script.as_str()), (9, "return;"));
    }
}
