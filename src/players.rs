//! Stage 2: player actions (private and admin messages, warp, disconnect,
//! props, rights, comments, local ban). Not implemented yet.
//!
//! Plan and message table: `docs/NEXT.md`. How to add a message:
//! `docs/ARCHITECTURE.md`, "Where new features go". Each builder gets a
//! byte-exact unit test and a case in `tests/fake_servers.rs`, and each
//! layout is confirmed against the fork in the lab before it is used.

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

// TODO(stage 2): builders, in docs/NEXT.md order:
// - admin_message(text)            -> pli::RC_ADMINMESSAGE
// - private_message(ids, text)     -> pli::PRIVATEMESSAGE
// - disconnect_player(id)          -> pli::RC_DISCONNECTPLAYER
// - warp_player(id, level, x, y)   -> pli::RC_WARPPLAYER
// - player_props_get(id)           -> pli::RC_PLAYERPROPSGET2, parse plo::RC_PLAYERPROPSGET
// - rights / comments / ban get and set
