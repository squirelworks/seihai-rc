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
2. **Mostly done (command line, 2026-11-04).** Players: list, private message,
   message all, warp, disconnect, edit props and rights, local ban. This is
   v0.1.0. List RCs moved to v0.2.0; it comes from the player list (an RC
   is a player with an empty level), since id 65 is never sent. More prop
   names are still open.
3. **Mostly done (command line, 2026-11-04).** Accounts: list with a filter,
   add, edit, delete. The byte before the ban reason in 73 is still open.
4. Server options, folder config, server flags, file browser with upload and
   download. The file link (list, get, put) is in as a library API; the rest
   is not started.
5. **Mostly done (command line, 2026-11-01).** NPC Control: NPC list, edit
   script, add/delete NPC, classes, weapons, levels. Login, discovery and
   NPC, class and weapon list / get / set / delete are in. Levels are not.

Each feature: a command-line form first (`seihai-rc players list --server ...`),
tested in CI against `seihai-gserver` and in the lab against the fork, then the
window.

## Out of scope: Graal3D (February 2000)

No work. That side project has its own admin packets on `GR3DV002`, not the
RC/NC path. Seihai covers its client admin as **G3D-5** and its host as
**G3D-9** in `documentation/plannedmilestones.md`. Do not add RC features
for it here.

Status and the next steps in order: [NEXT.md](NEXT.md). Code layout:
[ARCHITECTURE.md](ARCHITECTURE.md).
