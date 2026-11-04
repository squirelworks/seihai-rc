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

/// Id 72, `PLO_RC_PLAYERPROPSGET`, as far as the capture shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropsReply {
    pub id: u16,
    pub account: String,
    pub world: String,
    /// The props block, decoded with `seihai-core`'s prop table. Decoding
    /// stops at the first id that table does not know.
    pub props: Vec<seihai_core::protocol::PlayerProp>,
    /// True when the props block decoded to its last byte.
    pub props_complete: bool,
    /// The two GSHORTs after the props. Both were 0 in the capture, so
    /// which one counts flags and which opened chests is not known.
    pub tail_counts: (u16, u16),
    /// The weapon names. `None` when a tail count is not 0: the entries
    /// after a count have not been seen, so the weapons cannot be found.
    pub weapons: Option<Vec<String>>,
}

/// Read id 72: GSHORT player id, GSTRING account, GSTRING world, GCHAR
/// props length N, N bytes of props, GSHORT, GSHORT, GCHAR weapon count,
/// then a GSTRING per weapon.
pub fn parse_props(payload: &[u8]) -> Option<PropsReply> {
    use seihai_core::protocol::{parse_player_props, read_gshort};
    let id = read_gshort(payload)?;
    let mut i = 2;
    let account = text(&read_gstring(payload, &mut i)?);
    let world = text(&read_gstring(payload, &mut i)?);
    let block = read_gstring(payload, &mut i)?;
    let props = parse_player_props(&block);
    let used: usize = props.iter().map(|p| 1 + p.raw.len()).sum();
    let first = read_gshort(payload.get(i..)?)?;
    let second = read_gshort(payload.get(i + 2..)?)?;
    i += 4;
    let weapons = if first == 0 && second == 0 {
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
        tail_counts: (first, second),
        weapons,
    })
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
        8 => "sword",
        9 => "shield",
        10 => "gani",
        11 => "head",
        13 => "colors",
        15 => "x (half tiles)",
        16 => "y (half tiles)",
        18 => "status",
        20 => "level",
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
        assert_eq!(r.tail_counts, (0, 0));
        assert_eq!(
            r.weapons.unwrap(),
            ["bomb".to_string(), "bow".into(), "-gr_movement".into()]
        );
        // A non-zero tail count: the weapons are not guessed.
        let mut odd = hex(CAPTURED_72);
        let at = odd.len() - 27;
        assert_eq!(&odd[at..at + 4], b"    ");
        odd[at + 1] = 0x21;
        assert_eq!(parse_props(&odd).unwrap().weapons, None);
        assert!(parse_props(&hex(&CAPTURED_72[..40])).is_none());
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
