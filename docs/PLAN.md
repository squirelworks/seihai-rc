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
2. Players: list, private message, message all, warp, disconnect, edit props and
   rights, local ban.
3. Accounts: list with a filter, add, edit, delete.
4. Server options, folder config, server flags, file browser with upload and
   download.
5. NPC Control: NPC list, edit script, add/delete NPC, classes, weapons, levels.
   Login and plain-zlib framing are in (`seihai-rc nc login`). Lists are not.

Each feature: a command-line form first (`seihai-rc players list --server ...`),
tested in CI against `seihai-gserver` and in the lab against the fork, then the
window.

Status and the next steps in order: [NEXT.md](NEXT.md). Code layout:
[ARCHITECTURE.md](ARCHITECTURE.md).
