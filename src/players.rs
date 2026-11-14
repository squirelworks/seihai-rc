//! Stage 2: player actions. Private messages, the admin message to
//! everyone and to one player, disconnect, warp, the comments, account ban
//! and rights of an account, and reading and setting props.
//!
//! The layouts follow the 2026-11-04 loopback captures of a real RC
//! (`GSERV024`) against the gserver-v2 fork, and `seihai-rc` runs against
//! it: `docs/lab/2026-11-04-rc-player-admin.md`,
//! `docs/lab/2026-11-04-live-fork-run.md` and
//! `docs/lab/2026-11-04-rc-props-set.md`.
//!
//! Plan and message table: `docs/NEXT.md`. How to add a message:
//! `docs/ARCHITECTURE.md`, "Where new features go". Each builder has a
//! byte-exact unit test and a case in `tests/fake_servers.rs`.

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
    /// The opened chests: a GSHORT count, then each chest as a GSTRING whose
    /// body is GCHAR x, GCHAR y and the level name (lab 2026-11-13, an
    /// account with two `CHEST` lines; the real 2004 RC sent them back
    /// unchanged in 76).
    pub chests: Vec<Chest>,
    /// The weapon names.
    pub weapons: Vec<String>,
}

/// One opened chest in 72 and 76.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chest {
    /// The chest's tile, as one GCHAR each.
    pub x: u8,
    pub y: u8,
    pub level: String,
}

impl Chest {
    /// The GSTRING body: GCHAR x, GCHAR y, then the level name.
    fn body(&self) -> Vec<u8> {
        let mut b = vec![gchar(self.x), gchar(self.y)];
        b.extend_from_slice(&latin1_bytes(&self.level));
        b
    }

    fn parse(body: &[u8]) -> Option<Chest> {
        let x = body.first()?.checked_sub(0x20)?;
        let y = body.get(1)?.checked_sub(0x20)?;
        Some(Chest {
            x,
            y,
            level: text(&body[2..]),
        })
    }
}

/// Read id 72: GSHORT player id, GSTRING account, GSTRING world, GCHAR
/// props length N, N bytes of props, GSHORT flag count and a GSTRING per
/// flag, GSHORT chest count and a GSTRING per chest ([`Chest`]), GCHAR
/// weapon count, then a GSTRING per weapon.
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
    let nchests = read_gshort(payload.get(i..)?)?;
    i += 2;
    let mut chests = Vec::with_capacity(usize::from(nchests));
    for _ in 0..nchests {
        chests.push(Chest::parse(&read_gstring(payload, &mut i)?)?);
    }
    let n = payload.get(i)?.checked_sub(0x20)?;
    i += 1;
    let mut weapons = Vec::with_capacity(usize::from(n));
    for _ in 0..n {
        weapons.push(text(&read_gstring(payload, &mut i)?));
    }
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

/// What a props set may change, by name, with the value it takes:
/// `max-hearts` (whole hearts), `hearts` (halves allowed), `gralats`,
/// `arrows`, `bombs`, `bomb-power`, `glove`, `sword` and `shield` (the
/// power), `sword-image`, `shield-image`, `gani`, `head` (an image),
/// `body`, `colors` (five numbers joined with `:`), `x` and `y` (tiles,
/// halves allowed), `level`, `magic`, `kills`, `deaths` and `ap`.
///
/// The widths are `seihai-core`'s player prop table; only the first seven
/// were changed by a real RC in a capture (2026-11-04). The rest follow the
/// same table that reads the captured 72, so a value read back unchanged
/// is the check.
pub const PROP_EDIT_NAMES: [&str; 22] = [
    "max-hearts",
    "hearts",
    "gralats",
    "arrows",
    "bombs",
    "bomb-power",
    "glove",
    "sword",
    "sword-image",
    "shield",
    "shield-image",
    "gani",
    "head",
    "body",
    "colors",
    "x",
    "y",
    "level",
    "magic",
    "kills",
    "deaths",
    "ap",
];

/// The largest value a GINT3 holds.
const GINT3_MAX: u32 = (1 << 21) - 1;

/// Sword power at or under this goes with no image; over it, the GCHAR is
/// the power plus [`SWORD_IMAGE_OFFSET`] and the image follows.
const SWORD_PLAIN_MAX: u8 = 4;
const SWORD_IMAGE_OFFSET: u8 = 30;
const SHIELD_PLAIN_MAX: u8 = 3;
const SHIELD_IMAGE_OFFSET: u8 = 10;
/// Head (11) as an image is GCHAR (length + 100) and the name.
const HEAD_IMAGE_OFFSET: u8 = 100;

/// One change for a props set, from `NAME=VALUE` ([`parse_prop_edit`]).
/// Turned into prop bytes against the props just read with
/// [`resolve_prop_edits`], because sword and shield keep their image when
/// only the power changes, and the other way round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropEdit {
    /// Replace prop `id` with these value bytes.
    Raw(u8, Vec<u8>),
    SwordPower(u8),
    SwordImage(String),
    ShieldPower(u8),
    ShieldImage(String),
}

/// A GSTRING value: latin-1, one line, at most `max` bytes.
fn gstring_value(name: &str, value: &str, max: usize) -> Result<Vec<u8>, String> {
    if value.chars().any(|c| c == '\n' || c == '\r') {
        return Err(format!("{name}: the value contains a line break"));
    }
    let b = latin1_bytes(value);
    if b.len() > max {
        return Err(format!("{name}: longer than {max} bytes"));
    }
    Ok(b)
}

/// Tiles with halves allowed, as one GCHAR of half tiles.
fn half_tile_value(v: &str) -> Option<u8> {
    let v: f32 = v.parse().ok()?;
    let halves = v * 2.0;
    if !halves.is_finite()
        || halves.fract() != 0.0
        || !(0.0..=f32::from(GCHAR_MAX)).contains(&halves)
    {
        return None;
    }
    Some(halves as u8)
}

/// One change, from `NAME=VALUE`. Hearts, `x` and `y` take halves (`2.5`);
/// every other number is whole. See [`PROP_EDIT_NAMES`].
pub fn parse_prop_edit(item: &str) -> Result<PropEdit, String> {
    let (name, value) = item
        .split_once('=')
        .ok_or_else(|| format!("{item}: give NAME=VALUE"))?;
    let name = name.trim();
    let value = value.trim();
    if !PROP_EDIT_NAMES.contains(&name) {
        return Err(format!(
            "{name}: not a prop this client sets ({})",
            PROP_EDIT_NAMES.join(", ")
        ));
    }
    let bad = || format!("{name}={value}: not a value this prop holds");
    let small = |max: u8| -> Result<u8, String> {
        value
            .parse::<u8>()
            .ok()
            .filter(|v| *v <= max)
            .ok_or_else(bad)
    };
    let gint3 = || -> Result<Vec<u8>, String> {
        let v: u32 = value.parse().map_err(|_| bad())?;
        if v > GINT3_MAX {
            return Err(bad());
        }
        let mut b = Vec::with_capacity(3);
        push_gint3(&mut b, v);
        Ok(b)
    };
    let string = |id: u8| -> Result<PropEdit, String> {
        let b = gstring_value(name, value, usize::from(GCHAR_MAX))?;
        let mut raw = Vec::with_capacity(1 + b.len());
        push_gstring(&mut raw, &b);
        Ok(PropEdit::Raw(id, raw))
    };
    Ok(match name {
        "max-hearts" | "hearts" => {
            let v: f32 = value.parse().map_err(|_| bad())?;
            let halves = v * 2.0;
            if !halves.is_finite() || halves.fract() != 0.0 || !(0.0..=40.0).contains(&halves) {
                return Err(bad());
            }
            if name == "max-hearts" {
                // Max power is whole hearts.
                if halves.rem_euclid(2.0) != 0.0 {
                    return Err(bad());
                }
                PropEdit::Raw(1, vec![gchar((halves / 2.0) as u8)])
            } else {
                // Power is half hearts.
                PropEdit::Raw(2, vec![gchar(halves as u8)])
            }
        }
        "gralats" => PropEdit::Raw(3, gint3()?),
        "arrows" => PropEdit::Raw(4, vec![gchar(small(GCHAR_MAX)?)]),
        "bombs" => PropEdit::Raw(5, vec![gchar(small(GCHAR_MAX)?)]),
        "glove" => PropEdit::Raw(6, vec![gchar(small(GCHAR_MAX)?)]),
        "bomb-power" => PropEdit::Raw(7, vec![gchar(small(GCHAR_MAX)?)]),
        "sword" => PropEdit::SwordPower(small(GCHAR_MAX - SWORD_IMAGE_OFFSET)?),
        "shield" => PropEdit::ShieldPower(small(GCHAR_MAX - SHIELD_IMAGE_OFFSET)?),
        "sword-image" => {
            PropEdit::SwordImage(text(&gstring_value(name, value, usize::from(GCHAR_MAX))?))
        }
        "shield-image" => {
            PropEdit::ShieldImage(text(&gstring_value(name, value, usize::from(GCHAR_MAX))?))
        }
        "gani" => string(10)?,
        "head" => {
            let b = gstring_value(name, value, usize::from(GCHAR_MAX - HEAD_IMAGE_OFFSET))?;
            if b.is_empty() {
                return Err(bad());
            }
            let mut raw = vec![gchar(HEAD_IMAGE_OFFSET + b.len() as u8)];
            raw.extend_from_slice(&b);
            PropEdit::Raw(11, raw)
        }
        "colors" => {
            let parts: Vec<&str> = value.split(':').collect();
            if parts.len() != 5 {
                return Err(format!("{name}: give five numbers joined with ':'"));
            }
            let mut raw = Vec::with_capacity(5);
            for p in parts {
                let v = p
                    .trim()
                    .parse::<u8>()
                    .ok()
                    .filter(|v| *v <= GCHAR_MAX)
                    .ok_or_else(bad)?;
                raw.push(gchar(v));
            }
            PropEdit::Raw(13, raw)
        }
        "x" => PropEdit::Raw(15, vec![gchar(half_tile_value(value).ok_or_else(bad)?)]),
        "y" => PropEdit::Raw(16, vec![gchar(half_tile_value(value).ok_or_else(bad)?)]),
        "level" => {
            if value.is_empty() {
                return Err(bad());
            }
            string(20)?
        }
        "magic" => PropEdit::Raw(26, vec![gchar(small(GCHAR_MAX)?)]),
        "kills" => PropEdit::Raw(27, gint3()?),
        "deaths" => PropEdit::Raw(28, gint3()?),
        "ap" => PropEdit::Raw(32, vec![gchar(small(GCHAR_MAX)?)]),
        "body" => string(35)?,
        _ => unreachable!("every name in PROP_EDIT_NAMES has a case"),
    })
}

/// A sword (8) or shield (9) value read as power and image: GCHAR power
/// with no image at or under `plain_max`, else GCHAR (power + `offset`)
/// and a GSTRING image.
fn split_power_image(raw: &[u8], plain_max: u8, offset: u8) -> Option<(u8, String)> {
    let v = raw.first()?.checked_sub(0x20)?;
    if v <= plain_max {
        return Some((v, String::new()));
    }
    let power = v.checked_sub(offset)?;
    let mut i = 1;
    let image = read_gstring(raw, &mut i)?;
    Some((power, text(&image)))
}

/// Sword (8) or shield (9) as its power and image (empty when it has
/// none); `None` for another id or a value that does not read.
pub fn power_and_image(id: u8, raw: &[u8]) -> Option<(u8, String)> {
    match id {
        8 => split_power_image(raw, SWORD_PLAIN_MAX, SWORD_IMAGE_OFFSET),
        9 => split_power_image(raw, SHIELD_PLAIN_MAX, SHIELD_IMAGE_OFFSET),
        _ => None,
    }
}

/// Head (11) as an image name, when it is one (GCHAR length + 100).
pub fn head_image(raw: &[u8]) -> Option<String> {
    let v = raw.first()?.checked_sub(0x20)?;
    let n = usize::from(v.checked_sub(HEAD_IMAGE_OFFSET)?);
    (raw.len() == 1 + n).then(|| text(&raw[1..]))
}

fn join_power_image(
    what: &str,
    power: u8,
    image: &str,
    plain_max: u8,
    offset: u8,
) -> Result<Vec<u8>, String> {
    if image.is_empty() {
        if power > plain_max {
            return Err(format!(
                "{what} power {power} needs an image: the prop holds a power over \
                 {plain_max} only with one"
            ));
        }
        return Ok(vec![gchar(power)]);
    }
    let v = power
        .checked_add(offset)
        .filter(|v| *v <= GCHAR_MAX)
        .ok_or_else(|| format!("{what} power {power} does not fit with an image"))?;
    let mut raw = vec![gchar(v)];
    push_gstring(&mut raw, &latin1_bytes(image));
    Ok(raw)
}

/// Turn `edits` into the value bytes of each prop they change, against the
/// props in `from`. Later edits of the same prop win. A sword or shield
/// edit keeps the image (or power) that `from` has.
pub fn resolve_prop_edits(
    from: &PropsReply,
    edits: &[PropEdit],
) -> Result<Vec<(u8, Vec<u8>)>, String> {
    let mut out: Vec<(u8, Vec<u8>)> = Vec::new();
    for e in edits {
        let current = |id: u8| {
            out.iter()
                .find(|(d, _)| *d == id)
                .map(|(_, r)| r.clone())
                .or_else(|| {
                    from.props
                        .iter()
                        .find(|p| p.id == id)
                        .map(|p| p.raw.clone())
                })
        };
        let (id, raw) = match e {
            PropEdit::Raw(id, raw) => (*id, raw.clone()),
            PropEdit::SwordPower(_) | PropEdit::SwordImage(_) => {
                let (power, image) = current(8)
                    .and_then(|r| split_power_image(&r, SWORD_PLAIN_MAX, SWORD_IMAGE_OFFSET))
                    .unwrap_or_default();
                let (power, image) = match e {
                    PropEdit::SwordPower(p) => (*p, image),
                    PropEdit::SwordImage(i) => (power, i.clone()),
                    _ => unreachable!(),
                };
                (
                    8,
                    join_power_image("sword", power, &image, SWORD_PLAIN_MAX, SWORD_IMAGE_OFFSET)?,
                )
            }
            PropEdit::ShieldPower(_) | PropEdit::ShieldImage(_) => {
                let (power, image) = current(9)
                    .and_then(|r| split_power_image(&r, SHIELD_PLAIN_MAX, SHIELD_IMAGE_OFFSET))
                    .unwrap_or_default();
                let (power, image) = match e {
                    PropEdit::ShieldPower(p) => (*p, image),
                    PropEdit::ShieldImage(i) => (power, i.clone()),
                    _ => unreachable!(),
                };
                (
                    9,
                    join_power_image(
                        "shield",
                        power,
                        &image,
                        SHIELD_PLAIN_MAX,
                        SHIELD_IMAGE_OFFSET,
                    )?,
                )
            }
        };
        out.retain(|(d, _)| *d != id);
        out.push((id, raw));
    }
    Ok(out)
}

/// The flag list after `add` and `remove`. A flag is `NAME` or
/// `NAME=VALUE`; adding one replaces any flag of the same name, and
/// removing `NAME` drops it whatever its value.
pub fn edit_flags(
    flags: &[String],
    add: &[String],
    remove: &[String],
) -> Result<Vec<String>, String> {
    let name = |f: &str| f.split_once('=').map_or(f, |(n, _)| n).to_string();
    let mut out: Vec<String> = flags.to_vec();
    for r in remove {
        let n = name(r.trim());
        if n.is_empty() {
            return Err("--remove-flag needs a flag name".into());
        }
        out.retain(|f| name(f) != n);
    }
    for a in add {
        let a = a.trim();
        let n = name(a);
        if n.is_empty() {
            return Err("--add-flag needs NAME or NAME=VALUE".into());
        }
        gstring_value("the flag", a, usize::from(GCHAR_MAX))?;
        out.retain(|f| name(f) != n);
        out.push(a.to_string());
    }
    if out.len() > usize::from(GSHORT_MAX) {
        return Err("more flags than a GSHORT count holds".into());
    }
    Ok(out)
}

/// The weapon list after `add` and `remove`, by exact name. A weapon
/// already held is not added twice.
pub fn edit_weapons(
    weapons: &[String],
    add: &[String],
    remove: &[String],
) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = weapons.to_vec();
    for r in remove {
        out.retain(|w| w != r.trim());
    }
    for a in add {
        let a = a.trim();
        if a.is_empty() {
            return Err("--add-weapon needs a weapon name".into());
        }
        gstring_value("the weapon", a, usize::from(GCHAR_MAX))?;
        if !out.iter().any(|w| w == a) {
            out.push(a.to_string());
        }
    }
    if out.len() > usize::from(GCHAR_MAX) {
        return Err("more weapons than a GCHAR count holds".into());
    }
    Ok(out)
}

fn push_gint3(out: &mut Vec<u8>, v: u32) {
    out.push(gchar(((v >> 14) & 0x7f) as u8));
    out.push(gchar(((v >> 7) & 0x7f) as u8));
    out.push(gchar((v & 0x7f) as u8));
}

/// Id 76, `PLI_RC_PLAYERPROPSSET2`, built from a 72 the way the real RC
/// builds it (both 2026-11-04 captures): GSTRING account, GSTRING world,
/// GCHAR props length and the props in [`SET_PROP_IDS`] order, GSHORT
/// flag count and a GSTRING each, GSHORT chest count and a GSTRING each,
/// GCHAR weapon count and a GSTRING each, then a GCHAR 0.
///
/// Each prop comes from `from`, then `edits` replace theirs. Online
/// seconds (29) go as 0, as the real RC sends them; the fork ignores them.
/// Props 7, 21, 22 and 23, which the fork's 72 leaves out, go as the RC
/// sent them (7 as 1, the rest empty). Status (18) goes without its paused
/// bit. Flags go back sorted and chests as they came, as the 2004 RC did
/// (lab 2026-11-13). A 72 without one of the
/// other props is refused.
pub fn props_set(from: &PropsReply, edits: &[(u8, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let weapons = &from.weapons;
    let mut block = Vec::new();
    for id in SET_PROP_IDS {
        let raw = match edits.iter().rev().find(|(e, _)| *e == id) {
            Some((_, raw)) => raw.clone(),
            None => match id {
                29 => vec![0x20; 3],
                // Status: the 2004 RC sent 21 (paused, male, weapons) back as
                // 20, without the paused bit (lab 2026-11-13); 20 stayed 20 in
                // the 2026-11-04 captures. Its dialog has no paused box.
                18 => from
                    .props
                    .iter()
                    .find(|p| p.id == 18)
                    .and_then(|p| p.raw.first())
                    .map(|b| vec![gchar(b.wrapping_sub(0x20) & !1)])
                    .ok_or("the props have no prop 18 to send back")?,
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
    // The RC sends the flags sorted (lab 2026-11-13: the fork's 72 had
    // `quest.done=yes, labflag=1`, the RC's 76 `labflag=1, quest.done=yes`).
    let mut flags: Vec<&String> = from.flags.iter().collect();
    flags.sort();
    push_gshort(&mut payload, flags.len() as u16);
    for f in flags {
        push_gstring(&mut payload, &latin1_bytes(f));
    }
    push_gshort(&mut payload, from.chests.len() as u16);
    for c in &from.chests {
        push_gstring(&mut payload, &c.body());
    }
    payload.push(gchar(weapons.len() as u8));
    for w in weapons {
        push_gstring(&mut payload, &latin1_bytes(w));
    }
    payload.push(gchar(0));
    Ok(message(pli::RC_PLAYERPROPSSET2, &payload))
}

/// A prop value for people: text when the bytes are a GSTRING that fills
/// the value, a number for one byte or a GINT3, `power image`, colors as
/// `a:b:c:d:e`, and escaped bytes otherwise.
pub fn prop_value(id: u8, raw: &[u8]) -> String {
    if let Some((power, image)) = power_and_image(id, raw) {
        return if image.is_empty() {
            power.to_string()
        } else {
            format!("{power} {image}")
        };
    }
    if id == 11 {
        if let Some(image) = head_image(raw) {
            return image;
        }
    }
    if id == 13 && raw.len() == 5 {
        let c: Vec<String> = raw
            .iter()
            .map(|b| b.wrapping_sub(0x20).to_string())
            .collect();
        return c.join(":");
    }
    let n = |b: &u8| u32::from(b.wrapping_sub(0x20) & 0x7F);
    let gstring = raw.len() > 1 && usize::from(raw[0].wrapping_sub(0x20)) == raw.len() - 1;
    match raw {
        [b] => n(b).to_string(),
        [a, b, c] if !gstring || matches!(id, 3 | 27 | 28 | 29 | 36) => {
            ((n(a) << 14) | (n(b) << 7) | n(c)).to_string()
        }
        _ if gstring => crate::wire::text(&raw[1..]),
        _ => raw
            .iter()
            .map(|&b| {
                if (0x20..0x7f).contains(&b) {
                    (b as char).to_string()
                } else {
                    format!("\\x{b:02x}")
                }
            })
            .collect(),
    }
}

/// The `players props` text for a reply, one line each.
pub fn props_lines(r: &PropsReply) -> Vec<String> {
    let mut out = Vec::new();
    if r.id == 0 {
        out.push(format!(
            "account {} world {} (not online)",
            r.account, r.world
        ));
    } else {
        out.push(format!(
            "player {} account {} world {}",
            r.id, r.account, r.world
        ));
    }
    for p in &r.props {
        out.push(format!(
            "{:>4}  {:<20}  {}",
            p.id,
            prop_name(p.id).unwrap_or("?"),
            prop_value(p.id, &p.raw)
        ));
    }
    if !r.props_complete {
        out.push("(the props block holds more than the prop table could read)".into());
    }
    out.push(format!("flags: {}", r.flags.len()));
    for f in &r.flags {
        out.push(format!("  {f}"));
    }
    out.push(format!("opened chests: {}", r.chests.len()));
    for c in &r.chests {
        out.push(format!("  {},{} {}", c.x, c.y, c.level));
    }
    out.push(format!("weapons: {}", r.weapons.join(", ")));
    out
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
        assert!(r.chests.is_empty());
        assert_eq!(
            r.weapons,
            ["bomb".to_string(), "bow".into(), "-gr_movement".into()]
        );
        // A chest count with no chest after it: not read.
        let mut odd = hex(CAPTURED_72);
        let at = odd.len() - 27;
        assert_eq!(&odd[at..at + 4], b"    ");
        odd[at + 3] = 0x21;
        assert!(parse_props(&odd).is_none());
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
        assert_eq!(r.weapons.len(), 3);
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
        assert_eq!(back.weapons.len(), 3);
    }

    /// 2026-11-13, a real 2004 RC: the fork's 72 for `labflags` (two flags,
    /// two opened chests), without the id byte.
    const CAPTURED3_72: &str = "2020286c6162666c616773246d61696eb22027756e6b6e6f776e2123222623202020242a25252621283f2a73776f7264312e706e67292b2b736869656c64312e706e672a2469646c652b8d68656164302e706e672d22202a24322f5c305d323534336f6e6c696e6573746172746c6f63616c2e6e773a203b2020203c2020203d2020203e2020202020405242286c6162666c6167734328626f64792e706e67444e927e20222e71756573742e646f6e653d796573296c6162666c61673d312022353e346f6e6c696e6573746172746c6f63616c2e6e77352a2c6f6e6c696e6573746172746c6f63616c2e6e772324626f6d6223626f772c2d67725f6d6f76656d656e74";
    /// The 76 that RC sent back after gralats 0 -> 7.
    const CAPTURED3_76: &str = "6c286c6162666c616773246d61696e9d2123222623202027242a252526212721283f2a73776f7264312e706e67292b2b736869656c64312e706e672a2469646c652b8d68656164302e706e672d22202a24322f5c305d323434336f6e6c696e6573746172746c6f63616c2e6e773520362037203a203b2020203c2020203d20202040524328626f64792e706e672022296c6162666c61673d312e71756573742e646f6e653d7965732022353e346f6e6c696e6573746172746c6f63616c2e6e77352a2c6f6e6c696e6573746172746c6f63616c2e6e772324626f6d6223626f772c2d67725f6d6f76656d656e7420";

    #[test]
    fn props_with_chests_match_capture_3() {
        let r = parse_props(&hex(CAPTURED3_72)).unwrap();
        assert_eq!(r.account, "labflags");
        assert_eq!(r.flags, ["quest.done=yes", "labflag=1"]);
        assert_eq!(
            r.chests,
            [
                Chest {
                    x: 30,
                    y: 20,
                    level: "onlinestartlocal.nw".into()
                },
                Chest {
                    x: 10,
                    y: 12,
                    level: "onlinestartlocal.nw".into()
                },
            ]
        );
        assert_eq!(r.weapons, ["bomb", "bow", "-gr_movement"]);
        let edits = resolve_prop_edits(&r, &[parse_prop_edit("gralats=7").unwrap()]).unwrap();
        let set = props_set(&r, &edits).unwrap();
        assert_eq!(set, hex(CAPTURED3_76));
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
        let msg = props_set(&r, &resolve_prop_edits(&r, &edits).unwrap()).unwrap();
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
        assert_eq!(
            parse_prop_edit("gralats=2097151").unwrap(),
            PropEdit::Raw(3, vec![0x9f; 3])
        );
        let mut r = parse_props(&hex(CAPTURED2_72)).unwrap();
        r.props.retain(|p| p.id != 35);
        assert!(props_set(&r, &[]).unwrap_err().contains("35"));
    }

    /// The props block of a 76, decoded.
    fn sent_props(msg: &[u8]) -> Vec<seihai_core::protocol::PlayerProp> {
        let mut i = 1;
        read_gstring(msg, &mut i).unwrap();
        read_gstring(msg, &mut i).unwrap();
        seihai_core::protocol::parse_player_props(&read_gstring(msg, &mut i).unwrap())
    }

    #[test]
    fn props_set_takes_the_wider_names() {
        let r = parse_props(&hex(CAPTURED2_72)).unwrap();
        let edits: Vec<PropEdit> = [
            "bomb-power=2",
            "gani=walk",
            "head=head5.png",
            "body=body2.png",
            "colors=1:2:3:4:5",
            "x=30.5",
            "y=12",
            "level=cave.nw",
            "magic=40",
            "kills=300",
            "deaths=2",
        ]
        .iter()
        .map(|e| parse_prop_edit(e).unwrap())
        .collect();
        let msg = props_set(&r, &resolve_prop_edits(&r, &edits).unwrap()).unwrap();
        let props = sent_props(&msg);
        // Every prop still decodes with the same table, in the RC's order.
        let ids: Vec<u8> = props.iter().map(|p| p.id).collect();
        assert_eq!(ids, SET_PROP_IDS);
        let raw = |id: u8| props.iter().find(|p| p.id == id).unwrap().raw.clone();
        assert_eq!(raw(7), [0x22]);
        assert_eq!(raw(10), b"$walk");
        assert_eq!(raw(11), [&[0x20 + 100 + 9][..], b"head5.png"].concat());
        assert_eq!(raw(35), b")body2.png");
        assert_eq!(raw(13), [0x21, 0x22, 0x23, 0x24, 0x25]);
        assert_eq!(raw(15), [0x20 + 61]);
        assert_eq!(raw(16), [0x20 + 24]);
        assert_eq!(raw(20), b"'cave.nw");
        assert_eq!(raw(26), [0x20 + 40]);
        assert_eq!(raw(27), [0x20, 0x22, 0x20 + 44]); // 300 = 2 * 128 + 44
        assert_eq!(raw(28), [0x20, 0x20, 0x22]);
        // What was not edited goes back as the 72 had it.
        let before = |id: u8| r.props.iter().find(|p| p.id == id).unwrap().raw.clone();
        assert_eq!(raw(8), before(8));
        assert_eq!(raw(9), before(9));
    }

    #[test]
    fn sword_and_shield_keep_the_other_half() {
        let r = parse_props(&hex(CAPTURED2_72)).unwrap();
        // The capture holds sword 1 with `sword1.png`, shield 1 with
        // `shield1.png`.
        let one = |e: &str| resolve_prop_edits(&r, &[parse_prop_edit(e).unwrap()]).unwrap();
        assert_eq!(
            one("sword=3"),
            [(8, [&[0x20 + 33, 0x2a][..], b"sword1.png"].concat())]
        );
        assert_eq!(
            one("sword-image=sword2.png"),
            [(8, [&[0x20 + 31, 0x2a][..], b"sword2.png"].concat())]
        );
        assert_eq!(
            one("shield=2"),
            [(9, [&[0x20 + 12, 0x2b][..], b"shield1.png"].concat())]
        );
        // No image: the power alone, which holds up to 4 (sword) or 3 (shield).
        let both = resolve_prop_edits(
            &r,
            &[
                parse_prop_edit("sword-image=").unwrap(),
                parse_prop_edit("sword=4").unwrap(),
            ],
        )
        .unwrap();
        assert_eq!(both, [(8, vec![0x24])]);
        let too_strong = resolve_prop_edits(
            &r,
            &[
                parse_prop_edit("shield-image=").unwrap(),
                parse_prop_edit("shield=4").unwrap(),
            ],
        );
        assert!(too_strong.unwrap_err().contains("needs an image"));
        // A sword image the 72 did not have starts from power 0.
        let mut bare = r.clone();
        bare.props.retain(|p| p.id != 8);
        assert_eq!(
            resolve_prop_edits(&bare, &[parse_prop_edit("sword-image=s.png").unwrap()]).unwrap(),
            [(8, [&[0x20 + 30, 0x25][..], b"s.png"].concat())]
        );
    }

    #[test]
    fn wider_names_refuse_what_does_not_fit() {
        for bad in [
            "colors=1:2:3:4",
            "colors=1:2:3:4:224",
            "x=112",
            "y=1.25",
            "level=",
            "head=",
            "kills=2097152",
            "sword=194",
            "shield=214",
        ] {
            assert!(parse_prop_edit(bad).is_err(), "{bad}");
        }
        let long = format!("head={}", "h".repeat(124));
        assert!(parse_prop_edit(&long).is_err());
        assert!(parse_prop_edit(&format!("head={}", "h".repeat(123))).is_ok());
    }

    #[test]
    fn flag_and_weapon_edits() {
        let flags: Vec<String> = vec!["quest.done=yes".into(), "labflag=1".into()];
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            edit_flags(&flags, &s(&["labflag=2", "new"]), &s(&["quest.done"])).unwrap(),
            s(&["labflag=2", "new"])
        );
        assert!(edit_flags(&flags, &s(&["=x"]), &[]).is_err());
        assert!(edit_flags(
            &flags,
            &s(&["a
b"]),
            &[]
        )
        .is_err());
        let weapons = s(&["bomb", "bow"]);
        assert_eq!(
            edit_weapons(&weapons, &s(&["bow", "-gr_movement"]), &s(&["bomb"])).unwrap(),
            s(&["bow", "-gr_movement"])
        );
        // A 76 carries the edited lists: flags sorted, weapons in order.
        let mut r = parse_props(&hex(CAPTURED2_72)).unwrap();
        r.flags = edit_flags(&r.flags, &s(&["zeta", "alpha=1"]), &[]).unwrap();
        r.weapons = edit_weapons(&r.weapons, &[], &s(&["bow"])).unwrap();
        let msg = props_set(&r, &[]).unwrap();
        let mut i = 1;
        for _ in 0..3 {
            read_gstring(&msg, &mut i).unwrap();
        }
        assert_eq!(&msg[i..i + 2], [0x20, 0x22]);
        i += 2;
        assert_eq!(read_gstring(&msg, &mut i).unwrap(), b"alpha=1");
        assert_eq!(read_gstring(&msg, &mut i).unwrap(), b"zeta");
        i += 2; // no chests
        assert_eq!(msg[i], 0x22);
        i += 1;
        assert_eq!(read_gstring(&msg, &mut i).unwrap(), b"bomb");
        assert_eq!(read_gstring(&msg, &mut i).unwrap(), b"-gr_movement");
        assert_eq!(&msg[i..], [0x20]);
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
