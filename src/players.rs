//! Stage 2: player actions. Built so far: private messages, the admin
//! message to everyone and to one player, disconnect, warp, and the
//! comments, account ban and rights of an account, and reading a player's
//! props. Setting props (76) is not sent yet; see `docs/PROTOCOL.md`.
//!
//! Comments (85/86 → 63), the ban (87/88 → 64), rights (83/84 → 62) and
//! props (73 → 72) follow the 2026-11-04
//! loopback capture of a real RC (`GSERV024`) against the gserver-v2 fork:
//! `docs/lab/2026-11-04-rc-player-admin.md`.
//!
//! Plan and message table: `docs/NEXT.md`. How to add a message:
//! `docs/ARCHITECTURE.md`, "Where new features go". Each builder has a
//! byte-exact unit test and a case in `tests/fake_servers.rs`. The layouts
//! for 28, 61, 63, 64 and 82 below are what this client sends; they are marked
//! **check** in `docs/PROTOCOL.md` until a fork lab run confirms them.

use crate::rc::{PlayerEntry, PlayerList};
use crate::wire::{
    gchar, gdetokenize, gtokenize, latin1_bytes, message, push_gint5, push_gshort, push_gstring,
    read_gint5, read_gstring, text,
};

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
    /// An account name is longer than a GSTRING holds.
    AccountTooLong,
    /// A text longer than its field holds (the field is named).
    TooLong(&'static str),
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
            PlayerError::TooLong(what) => write!(f, "{what} is too long for its field"),
            PlayerError::AccountTooLong => {
                write!(f, "the account name is longer than {LEVEL_MAX} bytes")
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
/// then the text tokenized as the rest of the line. The Seihai client
/// (`seihai-core` `send_pm`) sends the text the same way, and the gserver
/// reads it back with its tokenizer.
pub fn private_message(ids: &[u16], text: &str) -> Result<Vec<u8>, PlayerError> {
    if ids.is_empty() {
        return Err(PlayerError::NoPlayers);
    }
    if ids.len() > usize::from(GSHORT_MAX) {
        return Err(PlayerError::TooManyPlayers(ids.len()));
    }
    line_bytes("the message", text)?;
    let body = latin1_bytes(&gtokenize(text));
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

fn account_bytes(account: &str) -> Result<Vec<u8>, PlayerError> {
    let b = line_bytes("the account name", account)?;
    if b.len() > LEVEL_MAX {
        Err(PlayerError::AccountTooLong)
    } else {
        Ok(b)
    }
}

/// Lines as tokenized text. Each line of `lines` is one item; `\r\n`
/// counts as one line break.
fn tokenized_lines(lines: &str) -> Vec<u8> {
    latin1_bytes(&gtokenize(&lines.replace("\r\n", "\n")))
}

/// Id 85, `PLI_RC_PLAYERCOMMENTSGET`: the account as the rest of the line,
/// no length byte (capture, 2026-11-04).
pub fn comments_get(account: &str) -> Result<Vec<u8>, PlayerError> {
    Ok(message(pli::RC_PLAYERCOMMENTSGET, &account_bytes(account)?))
}

/// Id 86, `PLI_RC_PLAYERCOMMENTSSET`: GSTRING account, then the comments
/// tokenized, one item per line (capture, 2026-11-04).
pub fn comments_set(account: &str, comments: &str) -> Result<Vec<u8>, PlayerError> {
    let mut payload = Vec::new();
    push_gstring(&mut payload, &account_bytes(account)?);
    payload.extend_from_slice(&tokenized_lines(comments));
    Ok(message(pli::RC_PLAYERCOMMENTSSET, &payload))
}

/// Id 87, `PLI_RC_PLAYERBANGET`: the account as the rest of the line.
pub fn ban_get(account: &str) -> Result<Vec<u8>, PlayerError> {
    Ok(message(pli::RC_PLAYERBANGET, &account_bytes(account)?))
}

/// Id 88, `PLI_RC_PLAYERBANSET`: GSTRING account, GCHAR banned (0 or 1),
/// then the reason tokenized (capture, 2026-11-04).
pub fn ban_set(account: &str, banned: bool, reason: &str) -> Result<Vec<u8>, PlayerError> {
    let mut payload = Vec::new();
    push_gstring(&mut payload, &account_bytes(account)?);
    payload.push(gchar(u8::from(banned)));
    payload.extend_from_slice(&tokenized_lines(reason));
    Ok(message(pli::RC_PLAYERBANSET, &payload))
}

/// Id 63, `PLO_RC_PLAYERCOMMENTSGET`: whose comments, and the lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comments {
    pub account: String,
    pub lines: Vec<String>,
}

/// Read id 63: GSTRING account, then the comments tokenized.
pub fn parse_comments(payload: &[u8]) -> Option<Comments> {
    let mut i = 0;
    let account = text(&read_gstring(payload, &mut i)?);
    Some(Comments {
        account,
        lines: gdetokenize(&text(&payload[i..])),
    })
}

/// Id 64, `PLO_RC_PLAYERBANGET`: whose ban, whether it is on, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ban {
    pub account: String,
    pub banned: bool,
    /// The reason, one entry per line. The capture only saw an empty
    /// reason in this reply; it is read as tokenized text, which leaves a
    /// plain reason without commas unchanged.
    pub reason: Vec<String>,
}

/// Read id 64: GSTRING account, GCHAR banned, then the reason.
pub fn parse_ban(payload: &[u8]) -> Option<Ban> {
    let mut i = 0;
    let account = text(&read_gstring(payload, &mut i)?);
    let banned = payload.get(i)?.wrapping_sub(0x20) != 0;
    Some(Ban {
        account,
        banned,
        reason: gdetokenize(&text(&payload[i + 1..])),
    })
}

/// Rights bits seen in the 2026-11-04 capture. The dialog lists its
/// checkboxes in bit order from bit 0; only these two were ticked there.
pub const RIGHT_WARPTO_XY: u32 = 1 << 0;
pub const RIGHT_ADMIN_MESSAGE: u32 = 1 << 9;

/// One account's staff rights, as 62 carries them and 84 sends them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rights {
    pub account: String,
    /// GINT5 bit set; see [`RIGHT_WARPTO_XY`] and [`RIGHT_ADMIN_MESSAGE`].
    pub rights: u32,
    pub ip_range: String,
    /// Folder rights, one per line (`rw levels/*`).
    pub folders: Vec<String>,
}

/// Id 83, `PLI_RC_PLAYERRIGHTSGET`: the account as the rest of the line.
pub fn rights_get(account: &str) -> Result<Vec<u8>, PlayerError> {
    Ok(message(pli::RC_PLAYERRIGHTSGET, &account_bytes(account)?))
}

/// Id 84, `PLI_RC_PLAYERRIGHTSSET`: GSTRING account, GINT5 rights, GSTRING
/// IP range, GSHORT length, then the folder lines tokenized (capture,
/// 2026-11-04).
pub fn rights_set(r: &Rights) -> Result<Vec<u8>, PlayerError> {
    let mut payload = Vec::new();
    push_gstring(&mut payload, &account_bytes(&r.account)?);
    push_gint5(&mut payload, r.rights);
    let ip = latin1_bytes(&r.ip_range);
    if r.ip_range.contains(['\n', '\r']) {
        return Err(PlayerError::LineBreak("the IP range"));
    }
    if ip.len() > LEVEL_MAX {
        return Err(PlayerError::TooLong("the IP range"));
    }
    push_gstring(&mut payload, &ip);
    if r.folders.iter().any(|l| l.contains(['\n', '\r'])) {
        return Err(PlayerError::LineBreak("a folder line"));
    }
    let folders = latin1_bytes(&gtokenize(&r.folders.join("\n")));
    if folders.len() > usize::from(GSHORT_MAX) {
        return Err(PlayerError::TooLong("the folder rights"));
    }
    push_gshort(&mut payload, folders.len() as u16);
    payload.extend_from_slice(&folders);
    Ok(message(pli::RC_PLAYERRIGHTSSET, &payload))
}

/// Read id 62: the same layout as 84. The folder text is read as tokenized
/// lines, so the fork's untokenized `rw levels/*` stays one line (the real
/// RC splits it at the space). A GSHORT length that runs past the message
/// is refused.
pub fn parse_rights(payload: &[u8]) -> Option<Rights> {
    let mut i = 0;
    let account = text(&read_gstring(payload, &mut i)?);
    let rights = read_gint5(payload, &mut i)?;
    let ip_range = text(&read_gstring(payload, &mut i)?);
    let n = usize::from(seihai_core::protocol::read_gshort(payload.get(i..)?)?);
    let folders = payload.get(i + 2..i + 2 + n)?;
    Some(Rights {
        account,
        rights,
        ip_range,
        folders: gdetokenize(&text(folders)),
    })
}

/// Id 73, `PLI_RC_PLAYERPROPSGET2`: GSHORT player id (capture, 2026-11-04).
pub fn props_get(id: u16) -> Result<Vec<u8>, PlayerError> {
    check_id(id)?;
    let mut payload = Vec::with_capacity(2);
    push_gshort(&mut payload, id);
    Ok(message(pli::RC_PLAYERPROPSGET2, &payload))
}

/// Id 74, `PLI_RC_PLAYERPROPSGET3`: GSTRING account, then an empty
/// GSTRING (capture 2, 2026-11-04: the real RC sent `%test1` and one 0x20
/// for an offline account). The reply is the same 72, with player id 0.
pub fn props_get_account(account: &str) -> Result<Vec<u8>, PlayerError> {
    let mut payload = Vec::new();
    push_gstring(&mut payload, &account_bytes(account)?);
    push_gstring(&mut payload, b"");
    Ok(message(pli::RC_PLAYERPROPSGET3, &payload))
}

/// Id 72, `PLO_RC_PLAYERPROPSGET`, as far as the captures show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropsReply {
    /// The player id; 0 for an account asked for by name (74).
    pub id: u16,
    pub account: String,
    pub world: String,
    /// The props block, decoded with `seihai-core`'s prop table. Decoding
    /// stops at the first id that table does not know.
    pub props: Vec<seihai_core::protocol::PlayerProp>,
    /// True when the props block decoded to its last byte.
    pub props_complete: bool,
    /// The NPC script flags: a GSHORT count, then a GSTRING each (seen in
    /// the real RC's 76). The gserver-v2 fork sends a count of 0 even for
    /// an account with flags stored.
    pub flags: Vec<String>,
    /// The opened-chest count, the GSHORT after the flags. Only 0 has
    /// been seen; what follows a non-zero count is not known.
    pub chests: u16,
    /// The weapon names. `None` when the chest count is not 0, as the
    /// weapons cannot then be found.
    pub weapons: Option<Vec<String>>,
}

/// Read id 72: GSHORT player id, GSTRING account, GSTRING world, GCHAR
/// props length N, N bytes of props, GSHORT flag count and a GSTRING per
/// flag, GSHORT chest count, GCHAR weapon count, then a GSTRING per weapon.
pub fn parse_props(payload: &[u8]) -> Option<PropsReply> {
    use seihai_core::protocol::{parse_player_props, read_gshort};
    let id = read_gshort(payload)?;
    let mut i = 2;
    let account = text(&read_gstring(payload, &mut i)?);
    let world = text(&read_gstring(payload, &mut i)?);
    let block = read_gstring(payload, &mut i)?;
    let props = parse_player_props(&block);
    let used: usize = props.iter().map(|p| 1 + p.raw.len()).sum();
    let nflags = read_gshort(payload.get(i..)?)?;
    i += 2;
    let mut flags = Vec::with_capacity(usize::from(nflags));
    for _ in 0..nflags {
        flags.push(text(&read_gstring(payload, &mut i)?));
    }
    let chests = read_gshort(payload.get(i..)?)?;
    i += 2;
    let weapons = if chests == 0 {
        let n = payload.get(i)?.wrapping_sub(0x20);
        i += 1;
        let mut names = Vec::with_capacity(usize::from(n));
        for _ in 0..n {
            names.push(text(&read_gstring(payload, &mut i)?));
        }
        Some(names)
    } else {
        None
    };
    Some(PropsReply {
        id,
        account,
        world,
        props_complete: used == block.len(),
        props,
        flags,
        chests,
        weapons,
    })
}

/// The prop ids the real RC sends in 76, in its order (both 2026-11-04
/// captures). It leaves out 0 (nickname), 30 (last IP), 34 (account) and
/// 36 (rating), which 72 carries.
pub const SET_PROP_IDS: [u8; 25] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 16, 18, 20, 21, 22, 23, 26, 27, 28, 29, 32, 35,
];

/// What a props set may change, by name. Each is one GCHAR except
/// gralats (GINT3).
pub const PROP_EDIT_NAMES: [(&str, u8); 7] = [
    ("max-hearts", 1),
    ("hearts", 2),
    ("gralats", 3),
    ("arrows", 4),
    ("bombs", 5),
    ("glove", 6),
    ("ap", 32),
];

/// The largest value a GINT3 holds.
const GINT3_MAX: u32 = (1 << 21) - 1;

/// One change for [`props_set`], from `NAME=VALUE`. Hearts take halves
/// (`2.5`); every other value is a whole number.
pub fn parse_prop_edit(item: &str) -> Result<(u8, Vec<u8>), String> {
    let (name, value) = item
        .split_once('=')
        .ok_or_else(|| format!("{item}: give NAME=VALUE"))?;
    let name = name.trim();
    let value = value.trim();
    let id = PROP_EDIT_NAMES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, id)| *id)
        .ok_or_else(|| {
            let names: Vec<&str> = PROP_EDIT_NAMES.iter().map(|(n, _)| *n).collect();
            format!("{name}: not a prop this client sets ({})", names.join(", "))
        })?;
    let bad = || format!("{name}={value}: not a value this prop holds");
    let raw = match id {
        1 | 2 => {
            let v: f32 = value.parse().map_err(|_| bad())?;
            let halves = v * 2.0;
            if !halves.is_finite() || halves.fract() != 0.0 || !(0.0..=40.0).contains(&halves) {
                return Err(bad());
            }
            if id == 1 && halves.rem_euclid(2.0) != 0.0 {
                return Err(bad());
            }
            // Max power is whole hearts; power is half hearts.
            let n = if id == 1 { halves / 2.0 } else { halves };
            vec![gchar(n as u8)]
        }
        3 => {
            let v: u32 = value.parse().map_err(|_| bad())?;
            if v > GINT3_MAX {
                return Err(bad());
            }
            let mut b = Vec::with_capacity(3);
            push_gint3(&mut b, v);
            b
        }
        _ => {
            let v: u8 = value.parse().map_err(|_| bad())?;
            if v > GCHAR_MAX {
                return Err(bad());
            }
            vec![gchar(v)]
        }
    };
    Ok((id, raw))
}

fn push_gint3(out: &mut Vec<u8>, v: u32) {
    out.push(gchar(((v >> 14) & 0x7f) as u8));
    out.push(gchar(((v >> 7) & 0x7f) as u8));
    out.push(gchar((v & 0x7f) as u8));
}

/// Id 76, `PLI_RC_PLAYERPROPSSET2`, built from a 72 the way the real RC
/// builds it (both 2026-11-04 captures): GSTRING account, GSTRING world,
/// GCHAR props length and the props in [`SET_PROP_IDS`] order, GSHORT
/// flag count and a GSTRING each, GSHORT chest count (0), GCHAR weapon
/// count and a GSTRING each, then a GCHAR 0.
///
/// Each prop comes from `from`, then `edits` replace theirs. Online
/// seconds (29) go as 0, as the real RC sends them; the fork ignores them.
/// Props 7, 21, 22 and 23, which the fork's 72 leaves out, go as the RC
/// sent them (7 as 1, the rest empty). A 72 with chests, with unread
/// weapons or without one of the other props is refused.
pub fn props_set(from: &PropsReply, edits: &[(u8, Vec<u8>)]) -> Result<Vec<u8>, String> {
    if from.chests != 0 {
        return Err("the props carry opened chests; that layout is not known".into());
    }
    let weapons = from
        .weapons
        .as_ref()
        .ok_or("the weapons in the props were not read")?;
    let mut block = Vec::new();
    for id in SET_PROP_IDS {
        let raw = match edits.iter().rev().find(|(e, _)| *e == id) {
            Some((_, raw)) => raw.clone(),
            None => match id {
                29 => vec![0x20; 3],
                7 | 21..=23 => from
                    .props
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.raw.clone())
                    .unwrap_or_else(|| vec![if id == 7 { 0x21 } else { 0x20 }]),
                _ => from
                    .props
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.raw.clone())
                    .ok_or_else(|| format!("the props have no prop {id} to send back"))?,
            },
        };
        block.push(gchar(id));
        block.extend_from_slice(&raw);
    }
    if block.len() > usize::from(GCHAR_MAX) {
        return Err("the props are longer than one GCHAR length holds".into());
    }
    let account = account_bytes(&from.account).map_err(|e| e.to_string())?;
    let mut payload = Vec::new();
    push_gstring(&mut payload, &account);
    push_gstring(&mut payload, &latin1_bytes(&from.world));
    push_gstring(&mut payload, &block);
    push_gshort(&mut payload, from.flags.len() as u16);
    for f in &from.flags {
        push_gstring(&mut payload, &latin1_bytes(f));
    }
    push_gshort(&mut payload, 0);
    payload.push(gchar(weapons.len() as u8));
    for w in weapons {
        push_gstring(&mut payload, &latin1_bytes(w));
    }
    payload.push(gchar(0));
    Ok(message(pli::RC_PLAYERPROPSSET2, &payload))
}

/// A short name for the player prop ids seen in the 2026-11-04 capture.
pub fn prop_name(id: u8) -> Option<&'static str> {
    Some(match id {
        0 => "nickname",
        1 => "max power",
        2 => "power (half hearts)",
        3 => "gralats",
        4 => "arrows",
        5 => "bombs",
        6 => "glove",
        7 => "bomb power",
        8 => "sword",
        9 => "shield",
        10 => "gani",
        11 => "head",
        13 => "colors",
        15 => "x (half tiles)",
        16 => "y (half tiles)",
        18 => "status",
        20 => "level",
        21 => "horse image",
        22 => "horse bushes",
        23 => "effect colors",
        26 => "magic points",
        27 => "kills",
        28 => "deaths",
        29 => "online seconds",
        30 => "last IP",
        32 => "AP",
        34 => "account",
        35 => "body",
        36 => "rating",
        _ => return None,
    })
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
    fn comments_match_the_capture() {
        // 85 + 0x20 = 'u'; 86 + 0x20 = 'v'.
        assert_eq!(comments_get("test1").unwrap(), b"utest1");
        assert_eq!(
            comments_set("test1", "lab comment one\nline two").unwrap(),
            b"v%test1\"lab comment one\",\"line two\""
        );
        assert_eq!(
            parse_comments(b"%test1\"lab comment one\",\"line two\"").unwrap(),
            Comments {
                account: "test1".into(),
                lines: vec!["lab comment one".into(), "line two".into()],
            }
        );
        assert!(parse_comments(b"%test1").unwrap().lines.is_empty());
        assert_eq!(
            comments_get("a\nb"),
            Err(PlayerError::LineBreak("the account name"))
        );
        assert_eq!(
            comments_get(&"a".repeat(LEVEL_MAX + 1)),
            Err(PlayerError::AccountTooLong)
        );
    }

    #[test]
    fn ban_matches_the_capture() {
        // 87 + 0x20 = 'w'; 88 + 0x20 = 'x'.
        assert_eq!(ban_get("test1").unwrap(), b"wtest1");
        assert_eq!(
            ban_set("test1", true, "lab ban reason").unwrap(),
            b"x%test1!\"lab ban reason\""
        );
        assert_eq!(ban_set("test1", false, "").unwrap(), b"x%test1 ");
        assert_eq!(
            parse_ban(b"%test1 ").unwrap(),
            Ban {
                account: "test1".into(),
                banned: false,
                reason: vec![],
            }
        );
        let on = parse_ban(b"%test1!\"lab ban reason\"").unwrap();
        assert!(on.banned);
        assert_eq!(on.reason, ["lab ban reason"]);
        assert!(parse_ban(b"%test1").is_none());
    }

    #[test]
    fn rights_match_the_capture() {
        // 83 + 0x20 = 's'; 84 + 0x20 = 't'.
        assert_eq!(rights_get("test1").unwrap(), b"stest1");
        let set = Rights {
            account: "test1".into(),
            rights: RIGHT_WARPTO_XY | RIGHT_ADMIN_MESSAGE,
            ip_range: "0.0.0.0".into(),
            folders: vec!["rw levels/*".into()],
        };
        assert_eq!(
            rights_set(&set).unwrap(),
            b"t%test1   $!'0.0.0.0 -\"rw levels/*\""
        );
        // The empty reply before the set.
        assert_eq!(
            parse_rights(b"%test1     '0.0.0.0  ").unwrap(),
            Rights {
                account: "test1".into(),
                rights: 0,
                ip_range: "0.0.0.0".into(),
                folders: vec![],
            }
        );
        // The fork's readback: folder text untokenized, 11 bytes.
        let back = parse_rights(b"%test1   $!'0.0.0.0 +rw levels/*").unwrap();
        assert_eq!(back.rights, 513);
        assert_eq!(back.folders, ["rw levels/*"]);
        // The tokenized form a server should send reads the same.
        assert_eq!(parse_rights(&set_payload(&set)).unwrap(), set);
        // A length past the end is refused.
        assert!(parse_rights(b"%test1   $!'0.0.0.0 /rw").is_none());
        let mut bad = set.clone();
        bad.folders = vec!["a\nb".into()];
        assert_eq!(
            rights_set(&bad),
            Err(PlayerError::LineBreak("a folder line"))
        );
    }

    fn set_payload(r: &Rights) -> Vec<u8> {
        rights_set(r).unwrap()[1..].to_vec()
    }

    /// The id 72 the fork sent the real RC in the 2026-11-04 capture.
    const CAPTURED_72: &str = "2023257465737431246d61696eb020284c616232323272632123222623202020242a25252621283f2a73776f7264312e706e67292b2b736869656c64312e706e672a2469646c652b8d68656164302e706e672d22202a24322f5c305d323434336f6e6c696e6573746172746c6f63616c2e6e773a203b2020203c2020203d20237b3e202820209f4054422574657374314328626f64792e706e67444e927e202020202324626f6d6223626f772c2d67725f6d6f76656d656e74";

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn props_reply_matches_the_capture() {
        // 73 + 0x20 = 'i'; player 3.
        assert_eq!(props_get(3).unwrap(), b"i #");
        let r = parse_props(&hex(CAPTURED_72)).unwrap();
        assert_eq!(r.id, 3);
        assert_eq!(r.account, "test1");
        assert_eq!(r.world, "main");
        assert!(r.props_complete, "{:?}", r.props);
        let ids: Vec<u8> = r.props.iter().map(|p| p.id).collect();
        assert_eq!(
            ids,
            [
                0, 1, 2, 3, 4, 5, 6, 8, 9, 10, 11, 13, 15, 16, 18, 20, 26, 27, 28, 29, 30, 32, 34,
                35, 36
            ]
        );
        assert_eq!(r.props[0].raw, b"(Lab222rc");
        // Online seconds, GINT3 475.
        assert_eq!(r.props[19].raw, [0x20, 0x23, 0x7b]);
        assert!(r.flags.is_empty());
        assert_eq!(r.chests, 0);
        assert_eq!(
            r.weapons.unwrap(),
            ["bomb".to_string(), "bow".into(), "-gr_movement".into()]
        );
        // A chest count that is not 0: the weapons are not guessed.
        let mut odd = hex(CAPTURED_72);
        let at = odd.len() - 27;
        assert_eq!(&odd[at..at + 4], b"    ");
        odd[at + 3] = 0x21;
        assert_eq!(parse_props(&odd).unwrap().weapons, None);
        assert!(parse_props(&hex(&CAPTURED_72[..40])).is_none());
    }

    /// Capture 2, 2026-11-04: the fork's 72 for offline `test1` (asked
    /// for with 74) after the set below, and the real RC's 76 that set
    /// gralats 3 and the flag `labflag=1` from the same dialog.
    const CAPTURED2_72: &str = "2020257465737431246d61696eaf2027756e6b6e6f776e2123222623202023242a25252621283f2a73776f7264312e706e67292b2b736869656c64312e706e672a2469646c652b8d68656164302e706e672d22202a24322f5d3038323434336f6e6c696e6573746172746c6f63616c2e6e773a203b2020203c2020203d20203e3e20202020204052422574657374314328626f64792e706e67444e927e202020202324626f6d6223626f772c2d67725f6d6f76656d656e74";
    const CAPTURED2_76: &str = "257465737431246d61696e9d2123222623202023242a252526212721283f2a73776f7264312e706e67292b2b736869656c64312e706e672a2469646c652b8d68656164302e706e672d22202a24322f5d3038323434336f6e6c696e6573746172746c6f63616c2e6e773520362037203a203b2020203c2020203d20202040524328626f64792e706e672021296c6162666c61673d3120202324626f6d6223626f772c2d67725f6d6f76656d656e7420";

    #[test]
    fn props_by_account_matches_capture_2() {
        // 74 + 0x20 = 'j'.
        assert_eq!(props_get_account("test1").unwrap(), b"j%test1 ");
        let r = parse_props(&hex(CAPTURED2_72)).unwrap();
        assert_eq!(r.id, 0);
        assert_eq!(r.account, "test1");
        assert!(r.props_complete);
        assert!(r.flags.is_empty());
        assert_eq!(r.weapons.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn props_set_rebuilds_the_real_rcs_76() {
        let mut r = parse_props(&hex(CAPTURED2_72)).unwrap();
        r.flags = vec!["labflag=1".into()];
        let msg = props_set(&r, &[]).unwrap();
        assert_eq!(msg[0], b'l'); // 76 + 0x20
        assert_eq!(msg[1..], hex(CAPTURED2_76));
        // The same tail in a 72 reads back as one flag.
        let mut reply = vec![0x20, 0x20];
        reply.extend_from_slice(&msg[1..msg.len() - 1]);
        let back = parse_props(&reply).unwrap();
        assert_eq!(back.flags, ["labflag=1"]);
        assert_eq!(back.weapons.unwrap().len(), 3);
    }

    #[test]
    fn props_set_applies_edits() {
        let r = parse_props(&hex(CAPTURED2_72)).unwrap();
        let edits = [
            parse_prop_edit("gralats=200").unwrap(),
            parse_prop_edit("hearts=2.5").unwrap(),
            parse_prop_edit("max-hearts=4").unwrap(),
            parse_prop_edit("ap=60").unwrap(),
        ];
        let msg = props_set(&r, &edits).unwrap();
        let mut i = 1;
        read_gstring(&msg, &mut i).unwrap();
        read_gstring(&msg, &mut i).unwrap();
        let block = read_gstring(&msg, &mut i).unwrap();
        let props = seihai_core::protocol::parse_player_props(&block);
        let ids: Vec<u8> = props.iter().map(|p| p.id).collect();
        assert_eq!(ids, SET_PROP_IDS);
        let raw = |id: u8| props.iter().find(|p| p.id == id).unwrap().raw.clone();
        assert_eq!(raw(3), [0x20, 0x21, 0x20 + 72]); // 200 = 1 * 128 + 72
        assert_eq!(raw(2), [0x20 + 5]);
        assert_eq!(raw(1), [0x20 + 4]);
        assert_eq!(raw(32), [0x20 + 60]);
        assert_eq!(raw(29), [0x20; 3]);
    }

    #[test]
    fn prop_edits_refuse_what_does_not_fit() {
        for bad in [
            "gralats",
            "nick=bob",
            "gralats=-1",
            "gralats=2097152",
            "hearts=2.25",
            "max-hearts=2.5",
            "hearts=21",
            "bombs=224",
            "ap=x",
        ] {
            assert!(parse_prop_edit(bad).is_err(), "{bad}");
        }
        assert_eq!(parse_prop_edit("gralats=2097151").unwrap().1, [0x9f; 3]);
        let mut r = parse_props(&hex(CAPTURED2_72)).unwrap();
        r.chests = 1;
        assert!(props_set(&r, &[]).is_err());
        let mut r = parse_props(&hex(CAPTURED2_72)).unwrap();
        r.props.retain(|p| p.id != 35);
        assert!(props_set(&r, &[]).unwrap_err().contains("35"));
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
            b"< \" #!\"\"hi\"".to_vec()
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
        assert_eq!(&m[5..], b"\"caf\xe9 ?\"");
        let quoted = private_message(&[1], "say \"hi\"").unwrap();
        assert_eq!(&quoted[5..], b"\"say \"\"hi\"\"\"");
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
