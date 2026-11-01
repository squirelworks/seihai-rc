//! seihai-rc: a Graal Remote Control (RC) client for server staff.
//!
//! Stage 1 logs in (directly, or through the listserver's RC list) and reads
//! the player list. NPC Control logs in on its own socket, finds that socket
//! from the game server, and lists NPCs, classes and weapons. The plan is in
//! `docs/PLAN.md`; the rules are in `AGENTS.md`; the protocol as this crate
//! speaks it is in `docs/PROTOCOL.md`.

pub mod link;
pub mod listserver;
pub mod nc;
pub mod players;
pub mod rc;
pub mod wire;
