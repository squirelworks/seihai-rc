# Next steps

Stage 1 (login, RC list, player list, RC chat) is merged. Work in this order,
one branch and pull request each. Ids below are packet ids (facts). Payload
layouts marked **check** must be confirmed in the lab against the fork before
they are relied on; write what you see into `docs/PROTOCOL.md`.

## Small follow-ups to stage 1

- [ ] `login`: print `[update]` rather than `[online]` when `ADDPLAYER` comes for
      an id already listed (the fork resends it when props change).
- [ ] Read `PLO_OTHERPLPROPS` (8) for RCs if the fork sends it on nickname or
      level changes (**check**), and update `PlayerList`.
- [ ] `--nick` default: real RCs always send a nickname; decide whether to send
      `*account` ourselves.
- [ ] `players list --json` for scripts.
- [ ] Optional identity string after the password in the type-6 login (the fork
      reads it if present; old RCs send none).

## Stage 2: players

| Action | Client → server | Server → client |
|---|---|---|
| Private message | `PLI_PRIVATEMESSAGE` (28): GSHORT count, GSHORT ids, text | `PLO_PRIVATEMESSAGE` (37) |
| Message all (admin message) | `PLI_RC_ADMINMESSAGE` (63): text | `PLO_RC_ADMINMESSAGE` (35) |
| Private admin message | `PLI_RC_PRIVADMINMESSAGE` (64): GSHORT id, text (**check**) | |
| Disconnect player | `PLI_RC_DISCONNECTPLAYER` (61): GSHORT id (**check**) | |
| Warp player | `PLI_RC_WARPPLAYER` (82): GSHORT id, x, y, level (**check**) | |
| Player props | `PLI_RC_PLAYERPROPSGET2` (73, by id) / `GET3` (74, by account); `PLI_RC_PLAYERPROPSSET2` (76) | `PLO_RC_PLAYERPROPSGET` (72) |
| Rights | `PLI_RC_PLAYERRIGHTSGET` (83) / `SET` (84) | `PLO_RC_PLAYERRIGHTSGET` (62) |
| Comments | `PLI_RC_PLAYERCOMMENTSGET` (85) / `SET` (86) | `PLO_RC_PLAYERCOMMENTSGET` (63) |
| Local ban | `PLI_RC_PLAYERBANGET` (87) / `SET` (88) | `PLO_RC_PLAYERBANGET` (64) |
| List RCs | `PLI_RC_LISTRCS` (65) | (**check**) |

Command line: `seihai-rc players pm ID TEXT`, `players say TEXT` (message all),
`players kick ID`, `players warp ID LEVEL X Y`, `players props ID`,
`players rights ACCOUNT [--set ...]`, `players ban ACCOUNT [--set ...]`.

## Stage 3: accounts

`PLI_RC_ACCOUNTLISTGET` (72, with a filter) → `PLO_RC_ACCOUNTLISTGET` (70);
`PLI_RC_ACCOUNTADD` (70), `ACCOUNTDEL` (71), `ACCOUNTGET` (77) →
`PLO_RC_ACCOUNTGET` (73), `ACCOUNTSET` (78). All layouts **check**.

## Stage 4: server

Server options `PLI_RC_SERVEROPTIONSGET/SET` (51/52) → `PLO_RC_SERVEROPTIONSGET` (76);
folder config (53/54) → 77; server flags (68/69) → 61; file browser
(89 to 93, 96 to 98, large files 155/156, folder delete 160) →
`PLO_RC_FILEBROWSER_DIRLIST` (65), `_DIR` (66), `_MESSAGE` (67).
Note: ids 51 to 54 meant account actions in RC 1.33; use the modern meanings
(`remotecontrol-lineage.md`).

## Stage 5: NPC Control

Behaviour: Graal_analysis `protocol/npc-control.md` (link; do not paste it).

- [x] Login and framing. Type 3, token `NCL11012`, plain zlib both ways,
      no key byte (`seihai-rc nc login --server 127.0.0.1:14804`). No bytes
      after the password until a loopback capture shows one. Evidence:
      fake NPC-server in `tests/nc_login.rs`. No real client was checked;
      the GServer-v2 lab pack was not available.
- [ ] Discovery. On the game-server RC link, id 94 (`PLI_NPCSERVERQUERY`)
      then id 79 (`PLO_NPCSERVERADDR`), and only then the NC socket.
- [ ] NPC, class and weapon list / get / set / delete. Payloads the note
      still marks as needing a lab check stay unparsed until then.

## Window

egui window behind a `gui` feature once stages 2 and 3 work on the command
line. See `docs/ARCHITECTURE.md` for the threading plan.
