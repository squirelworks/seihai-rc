//! Stage 2: player actions. Built so far: private messages, the admin
//! message to everyone and to one player, disconnect and warp. Props,
//! rights, comments, the local ban and the RC list come later; our notes
//! have no payload layout for them yet.
//!
//! Plan and message table: `docs/NEXT.md`. How to add a message:
//! `docs/ARCHITECTURE.md`, "Where new features go". Each builder has a
//! byte-exact unit test and a case in `tests/fake_servers.rs`. The layouts
//! for 28, 61, 63, 64 and 82 below are what this client sends; they are marked
//! **check** in `docs/PROTOCOL.md` until a fork lab run confirms them.

use crate::rc::{PlayerEntry, PlayerList};
use crate::wire::{gchar, latin1_bytes, message, push_gshort};

/// Client to server ids for stage 2 (ids are wire facts).
pub mod pli {
    pub const PRIVATEMESSAGE: u8 = 28;
    pub const RC_DISCONNECTPLAYER: u8 = 61;
    pub const RC_ADMINMESSAGE: u8 = 63;
    pub const RC_PRIVADMINMESSAGE: u8 = 64;
    pub const RC_LISTRCS: u8 = 65;
    pub const RC_PLAYERPROPSGET2: u8 = 73;
    pub const RC_PLAYERPROPSGET3: u8 = 74;
    pub const RC_PLAYERPROPSSET2: u8 = 76;
    pub const RC_WARPPLAYER: u8 = 82;
    pub const RC_PLAYERRIGHTSGET: u8 = 83;
    pub const RC_PLAYERRIGHTSSET: u8 = 84;
    pub const RC_PLAYERCOMMENTSGET: u8 = 85;
    pub const RC_PLAYERCOMMENTSSET: u8 = 86;
    pub const RC_PLAYERBANGET: u8 = 87;
    pub const RC_PLAYERBANSET: u8 = 88;
}

/// Server to client ids for stage 2.
pub mod plo {
    pub const RC_PLAYERRIGHTSGET: u8 = 62;
    pub const RC_PLAYERCOMMENTSGET: u8 = 63;
    pub const RC_PLAYERBANGET: u8 = 64;
    pub const RC_PLAYERPROPSGET: u8 = 72;
}

/// The largest value a GSHORT holds (14 bits).
pub const GSHORT_MAX: u16 = (1 << 14) - 1;

/// The largest value a GCHAR holds.
const GCHAR_MAX: u8 = 0xDF;

/// The largest level name this client sends. Longer names are refused,
/// not cut; it matches the GSTRING limit the rest of the crate uses.
pub const LEVEL_MAX: usize = 223;

/// Why a builder refused to produce bytes.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayerError {
    /// A line feed would end the message.
    LineBreak(&'static str),
    /// A player id over 14 bits does not fit in a GSHORT.
    BadId(u16),
    /// A private message needs at least one player.
    NoPlayers,
    /// More players than a GSHORT count can name.
    TooManyPlayers(usize),
    /// Nothing to send.
    Empty(&'static str),
    /// A warp coordinate outside what one GCHAR of half tiles holds.
    BadCoordinate(&'static str, f32),
    /// The level name is longer than [`LEVEL_MAX`].
    LevelTooLong,
}

impl std::fmt::Display for PlayerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlayerError::LineBreak(what) => write!(f, "{what} contains a line break"),
            PlayerError::BadId(id) => {
                write!(
                    f,
                    "player id {id} does not fit in a GSHORT (0 to {GSHORT_MAX})"
                )
            }
            PlayerError::NoPlayers => write!(f, "name at least one player"),
            PlayerError::TooManyPlayers(n) => {
                write!(f, "{n} players do not fit in a GSHORT count")
            }
            PlayerError::Empty(what) => write!(f, "{what} is empty"),
            PlayerError::BadCoordinate(axis, v) => write!(
                f,
                "{axis} {v} is outside 0 to {} in half-tile steps",
                f32::from(GCHAR_MAX) / 2.0
            ),
            PlayerError::LevelTooLong => {
                write!(f, "the level name is longer than {LEVEL_MAX} bytes")
            }
        }
    }
}

impl std::error::Error for PlayerError {}

fn line_bytes(what: &'static str, value: &str) -> Result<Vec<u8>, PlayerError> {
    if value.chars().any(|c| c == '\n' || c == '\r') {
        return Err(PlayerError::LineBreak(what));
    }
    if value.is_empty() {
        return Err(PlayerError::Empty(what));
    }
    Ok(latin1_bytes(value))
}

fn check_id(id: u16) -> Result<(), PlayerError> {
    if id > GSHORT_MAX {
        Err(PlayerError::BadId(id))
    } else {
        Ok(())
    }
}

/// Id 63, `PLI_RC_ADMINMESSAGE`: the text, the rest of the line. The
/// server shows it to everyone online (`players say`).
pub fn admin_message(text: &str) -> Result<Vec<u8>, PlayerError> {
    Ok(message(
        pli::RC_ADMINMESSAGE,
        &line_bytes("the message", text)?,
    ))
}

/// Id 28, `PLI_PRIVATEMESSAGE`: GSHORT count, that many GSHORT player ids,
/// then the text as the rest of the line.
pub fn private_message(ids: &[u16], text: &str) -> Result<Vec<u8>, PlayerError> {
    if ids.is_empty() {
        return Err(PlayerError::NoPlayers);
    }
    if ids.len() > usize::from(GSHORT_MAX) {
        return Err(PlayerError::TooManyPlayers(ids.len()));
    }
    let body = line_bytes("the message", text)?;
    let mut payload = Vec::with_capacity(2 + 2 * ids.len() + body.len());
    push_gshort(&mut payload, ids.len() as u16);
    for &id in ids {
        check_id(id)?;
        push_gshort(&mut payload, id);
    }
    payload.extend_from_slice(&body);
    Ok(message(pli::PRIVATEMESSAGE, &payload))
}

/// Id 64, `PLI_RC_PRIVADMINMESSAGE`: GSHORT player id, then the text as
/// the rest of the line. The admin message for one player
/// (`players admin-pm`).
pub fn private_admin_message(id: u16, text: &str) -> Result<Vec<u8>, PlayerError> {
    check_id(id)?;
    let body = line_bytes("the message", text)?;
    let mut payload = Vec::with_capacity(2 + body.len());
    push_gshort(&mut payload, id);
    payload.extend_from_slice(&body);
    Ok(message(pli::RC_PRIVADMINMESSAGE, &payload))
}

/// Id 61, `PLI_RC_DISCONNECTPLAYER`: GSHORT player id, nothing after it.
pub fn disconnect_player(id: u16) -> Result<Vec<u8>, PlayerError> {
    check_id(id)?;
    let mut payload = Vec::with_capacity(2);
    push_gshort(&mut payload, id);
    Ok(message(pli::RC_DISCONNECTPLAYER, &payload))
}

/// One warp coordinate as a GCHAR of half tiles, the width the level warp
/// messages use for x and y.
fn half_tiles(axis: &'static str, v: f32) -> Result<u8, PlayerError> {
    let twice = v * 2.0;
    if !twice.is_finite() || !(0.0..=f32::from(GCHAR_MAX)).contains(&twice) || twice.fract() != 0.0
    {
        return Err(PlayerError::BadCoordinate(axis, v));
    }
    Ok(gchar(twice as u8))
}

/// Id 82, `PLI_RC_WARPPLAYER`: GSHORT player id, GCHAR x and GCHAR y in
/// half tiles, then the level name as the rest of the line.
pub fn warp_player(id: u16, level: &str, x: f32, y: f32) -> Result<Vec<u8>, PlayerError> {
    check_id(id)?;
    let level = line_bytes("the level name", level)?;
    if level.len() > LEVEL_MAX {
        return Err(PlayerError::LevelTooLong);
    }
    let mut payload = Vec::with_capacity(4 + level.len());
    push_gshort(&mut payload, id);
    payload.push(half_tiles("x", x)?);
    payload.push(half_tiles("y", y)?);
    payload.extend_from_slice(&level);
    Ok(message(pli::RC_WARPPLAYER, &payload))
}

/// Find one player by id (digits) or by exact account name in `list`.
/// An account that more than one entry has is refused, so a script never
/// acts on the wrong one.
pub fn resolve<'a>(list: &'a PlayerList, who: &str) -> Result<&'a PlayerEntry, String> {
    if !who.is_empty() && who.bytes().all(|b| b.is_ascii_digit()) {
        let id: u16 = who
            .parse()
            .map_err(|_| format!("player id {who} does not fit in a GSHORT"))?;
        return list
            .players
            .get(&id)
            .ok_or_else(|| format!("no player or RC with id {id} is online"));
    }
    let mut hits = list.players.values().filter(|e| e.account == who);
    match (hits.next(), hits.next()) {
        (Some(e), None) => Ok(e),
        (None, _) => Err(format!("no player or RC with account {who:?} is online")),
        (Some(_), Some(_)) => Err(format!(
            "more than one entry has account {who:?}; give the id instead"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u16, account: &str) -> PlayerEntry {
        PlayerEntry {
            id,
            account: account.into(),
            level: "a.nw".into(),
            ..Default::default()
        }
    }

    #[test]
    fn resolve_by_id_or_unique_account() {
        let mut list = PlayerList::default();
        for e in [entry(2, "alice"), entry(3, "bob"), entry(4, "bob")] {
            list.players.insert(e.id, e);
        }
        assert_eq!(resolve(&list, "2").unwrap().account, "alice");
        assert_eq!(resolve(&list, "alice").unwrap().id, 2);
        assert!(resolve(&list, "bob").unwrap_err().contains("more than one"));
        assert!(resolve(&list, "9").unwrap_err().contains("id 9"));
        assert!(resolve(&list, "carol").is_err());
        assert!(resolve(&list, "99999").is_err());
    }

    #[test]
    fn admin_message_is_the_text() {
        // 63 + 0x20 = '_'.
        assert_eq!(admin_message("Restart in 5").unwrap(), b"_Restart in 5");
        assert_eq!(
            admin_message("a\nb"),
            Err(PlayerError::LineBreak("the message"))
        );
        assert_eq!(admin_message(""), Err(PlayerError::Empty("the message")));
    }

    #[test]
    fn private_message_counts_then_ids_then_text() {
        // 28 + 0x20 = '<'; count 2; ids 3 and 130 ((1 << 7) | 2).
        assert_eq!(
            private_message(&[3, 130], "hi").unwrap(),
            b"< \" #!\"hi".to_vec()
        );
        assert_eq!(private_message(&[], "hi"), Err(PlayerError::NoPlayers));
        assert_eq!(
            private_message(&[GSHORT_MAX + 1], "hi"),
            Err(PlayerError::BadId(GSHORT_MAX + 1))
        );
        assert_eq!(
            private_message(&[3], "x\ry"),
            Err(PlayerError::LineBreak("the message"))
        );
    }

    #[test]
    fn private_message_text_is_latin1() {
        let m = private_message(&[1], "caf\u{e9} \u{263a}").unwrap();
        assert_eq!(&m[5..], b"caf\xe9 ?");
    }

    #[test]
    fn private_admin_message_is_the_id_then_text() {
        // 64 + 0x20 = '`'; id 130 = (1 << 7) | 2.
        assert_eq!(
            private_admin_message(130, "Stop that").unwrap(),
            b"`!\"Stop that"
        );
        assert_eq!(
            private_admin_message(2, "a\nb"),
            Err(PlayerError::LineBreak("the message"))
        );
        assert_eq!(
            private_admin_message(2, ""),
            Err(PlayerError::Empty("the message"))
        );
        assert_eq!(
            private_admin_message(GSHORT_MAX + 1, "x"),
            Err(PlayerError::BadId(GSHORT_MAX + 1))
        );
    }

    #[test]
    fn disconnect_is_the_id_only() {
        // 61 + 0x20 = ']'; id 2.
        assert_eq!(disconnect_player(2).unwrap(), b"] \"");
        assert_eq!(
            disconnect_player(GSHORT_MAX + 1),
            Err(PlayerError::BadId(GSHORT_MAX + 1))
        );
    }

    #[test]
    fn warp_is_id_half_tiles_then_level() {
        // 82 + 0x20 = 'r'; id 2; x 30.5 -> 61; y 12 -> 24.
        let mut want = vec![b'r', 0x20, 0x22, 0x20 + 61, 0x20 + 24];
        want.extend_from_slice(b"cave.nw");
        assert_eq!(warp_player(2, "cave.nw", 30.5, 12.0).unwrap(), want);
    }

    #[test]
    fn warp_refuses_what_does_not_fit() {
        assert_eq!(
            warp_player(2, "a.nw", 30.25, 1.0),
            Err(PlayerError::BadCoordinate("x", 30.25))
        );
        assert_eq!(
            warp_player(2, "a.nw", 1.0, -1.0),
            Err(PlayerError::BadCoordinate("y", -1.0))
        );
        assert_eq!(
            warp_player(2, "a.nw", 112.0, 1.0),
            Err(PlayerError::BadCoordinate("x", 112.0))
        );
        assert!(warp_player(2, "a.nw", 111.5, 0.0).is_ok());
        assert_eq!(
            warp_player(2, "", 1.0, 1.0),
            Err(PlayerError::Empty("the level name"))
        );
        assert_eq!(
            warp_player(2, &"a".repeat(LEVEL_MAX + 1), 1.0, 1.0),
            Err(PlayerError::LevelTooLong)
        );
    }
}
