# seihai-rc (Remote Control)

A new RC client in Rust: a desktop window (egui) and a command-line mode for
scripts and tests. No original RC binary may be shipped or copied; the history and
protocol facts are in the project files `remotecontrol-lineage.md` and
`npc-control.md`.

## Protocol facts we have

- Old RCs (2000 to 2002) used `u16be length` + zlib body, messages `GCHAR id` +
  payload + `\n`, no cipher. Login: id 1, then the version token, account and
  password strings.
- Newer RCs log in through the listserver's RC list (id 5 + key + token, gen 5,
  every server, type-tagged), then connect to the chosen gserver.
- After login the RC sends its nickname as `PLI_PLAYERPROPS` (2).
- Ids 51 to 54 meant account actions in RC 1.33 but server options and folder
  config from 2002. Use the modern meanings.
- GServer-v2 implements the server side (`TPlayerRC.cpp`, `TPlayerNC.cpp` in
  older trees). Use it in the lab to see what the server sends, not as code.

## Features, in order

1. **Done (command line, 2026-10-31).** Connect (listserver RC list or direct
   host and port), log in, show server messages and the RC chat, list players.
2. **Implemented (command line).** Players: list, private message, admin
   messages, warp, disconnect, edit props and rights, comments and local ban.
   More prop names, flags and weapons are implemented in #47; chests in #45.
   `rcs list` uses the player list (an RC has an empty level), not id 65.
3. **Implemented (command line).** Accounts: list with a filter, get, add,
   edit, delete and reset. The byte before the ban reason in 73 is the
   GSTRING ban length, confirmed by the archived-RC captures in #45.
4. **Implemented (command line and library).** Server options, folder config
   and flags (#34); file list/get/put/move/rename/delete/folder-delete (#33),
   including large transfers (#41, with real-RC captures in #45).
5. **Partly implemented (command line).** NPC Control: login, discovery,
   NPC/class/weapon list/get/set/delete, plus NPC add/reset/warp/flags.
   Levels remain outside the implemented command set.

## Window and validation

The window is implemented behind `--features gui`: #48 provides login,
players, chat and player actions; #50 adds Accounts, Files, NPC Control and
Server tabs. See [ARCHITECTURE.md](ARCHITECTURE.md) for the worker model.

Historical CLI/protocol fork checks and the window's login/player-list smoke
test are recorded in [the 2026-11-14 lab pass](lab/2026-11-14-fork-lab-pass.md)
(#49, closing #26). These do not establish real-fork acceptance of the four
new #50 tabs. That acceptance, macOS validation and polish remain outstanding
in [roadmap #27](https://github.com/squirelworks/seihai-rc/issues/27).
Reconnect isolation is tracked separately in #51/#52; NPC Control editor and
delete-confirmation fixes are tracked in #53/#54. Their integration state must
be checked on GitHub before treating them as part of `main`.

New features follow a command-line form first
(`seihai-rc players list --server ...`), with byte tests, loopback integration
tests and recorded fork validation before window acceptance is claimed.

## Out of scope: Graal3D (February 2000)

No work. That side project has its own admin packets on `GR3DV002`, not the
RC/NC path. Seihai covers its client admin as **G3D-5** and its host as
**G3D-9** in `documentation/plannedmilestones.md`. Do not add RC features
for it here.

Status and the next steps in order: [NEXT.md](NEXT.md). Code layout:
[ARCHITECTURE.md](ARCHITECTURE.md).
