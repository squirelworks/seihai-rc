# Next steps

Stage 1 (login, RC list, player list, RC chat) is merged. Work in this order,
one branch and pull request each. Ids below are packet ids (facts). Payload
layouts marked **check** must be confirmed in the lab against the fork before
they are relied on; write what you see into `docs/PROTOCOL.md`.

## Small follow-ups to stage 1

- [x] `login`: print `[update]` rather than `[online]` when `ADDPLAYER` comes for
      an id already listed (the fork resends it when props change). The stored
      entry is replaced; after `DELPLAYER` the next one is `[online]` again.
      Evidence: fake gserver in `tests/fake_servers.rs`. No real client. The
      fork was not re-run (lab pack not available). The resend was seen on
      2026-10-31 (`docs/lab/2026-10-31-stage1.md`).
- [ ] Read `PLO_OTHERPLPROPS` (8) for RCs if the fork sends it on nickname or
      level changes (**check**), and update `PlayerList`.
- [ ] `--nick` default: real RCs always send a nickname; decide whether to send
      `*account` ourselves.
- [ ] `players list --json` for scripts.
- [ ] Optional identity string after the password in the type-6 login (the fork
      reads it if present; old RCs send none).
- [x] RC list over the secure handshake (`--secure-login --server-key`).
      C1 is id 5 plus `SEI00001` and the RC token, C2 seals the id 1
      payload, and the reply is the same id 5 list. The player-list form
      (id 7 and a list name) is not sent. Evidence: fake listserver in
      `tests/fake_servers.rs`. No real client. The seihai-listserver was
      not the peer (it answers `SEI00001` on id 7).

## Stage 2: players

| Action | Client → server | Server → client |
|---|---|---|
| Private message | `PLI_PRIVATEMESSAGE` (28): GSHORT count, GSHORT ids, text | `PLO_PRIVATEMESSAGE` (37) |
| Message all (admin message) | `PLI_RC_ADMINMESSAGE` (63): text | `PLO_RC_ADMINMESSAGE` (35) |
| Private admin message | `PLI_RC_PRIVADMINMESSAGE` (64): GSHORT id, text | |
| Disconnect player | `PLI_RC_DISCONNECTPLAYER` (61): GSHORT id | |
| Warp player | `PLI_RC_WARPPLAYER` (82): GSHORT id, GCHAR x, GCHAR y, level | |
| Player props | `PLI_RC_PLAYERPROPSGET2` (73, by id) / `GET3` (74, by account); `PLI_RC_PLAYERPROPSSET2` (76) | `PLO_RC_PLAYERPROPSGET` (72) |
| Rights | `PLI_RC_PLAYERRIGHTSGET` (83) / `SET` (84) | `PLO_RC_PLAYERRIGHTSGET` (62) |
| Comments | `PLI_RC_PLAYERCOMMENTSGET` (85) / `SET` (86) | `PLO_RC_PLAYERCOMMENTSGET` (63) |
| Local ban | `PLI_RC_PLAYERBANGET` (87) / `SET` (88) | `PLO_RC_PLAYERBANGET` (64) |
| List RCs | `PLI_RC_LISTRCS` (65) | (**check**) |

Command line: `seihai-rc players pm ID TEXT`, `players say TEXT` (message all),
`players admin-pm ID TEXT` (private admin message),
`players kick ID`, `players warp ID LEVEL X Y`, `players props ID`,
`players rights ACCOUNT [--set ...]`, `players ban ACCOUNT [--set ...]`.

- [x] First slice: `players pm`, `players say`, `players kick`,
      `players warp` (ids 28, 63, 61, 82). Who may be an id or an account
      from the player list. Every layout is **check**; see the player
      actions section of `docs/PROTOCOL.md`. Evidence: unit tests in
      `src/players.rs` and the fake gserver in `tests/fake_servers.rs`;
      `seihai-rc` live against the fork (`1d7843a`), 2026-11-04
      (`docs/lab/2026-11-04-live-fork-run.md`).
- [x] Private admin message: `players admin-pm WHO TEXT` (id 64,
      GSHORT id then the text). Evidence: unit test in `src/players.rs`,
      the fake gserver in `tests/fake_servers.rs`, and the same live run.
- [x] Comments (85/86 → 63) and account ban (87/88 → 64):
      `players comments ACCOUNT [--set ...]`, `players ban ACCOUNT [--set on|off]`.
      PM text (28) is now tokenized, as the Seihai client sends it.
      Evidence: a real RC (`GSERV024`) against the fork, 2026-11-04
      (`docs/lab/2026-11-04-rc-player-admin.md`); `seihai-rc` itself is
      checked against those bytes in `tests/fake_servers.rs`, and ran
      live against the fork (`docs/lab/2026-11-04-live-fork-run.md`).
- [x] Rights (83/84 → 62): `players rights ACCOUNT [--set N]
      [--ip-range R] [--set-file PATH]`. The folder text is read as
      tokenized lines, so the fork's untokenized `rw levels/*` stays one
      line. Evidence: the same lab note; `tests/fake_servers.rs` checks the
      84 bytes against the capture.
- [x] Props read (73 → 72): `players props WHO`. Evidence: the captured
      72 decodes in a unit test and through the fake gserver.
- [x] Props by account (74) and props set (76): `players props WHO
      [--set NAME=VALUE,...]`. A WHO not online is read with 74. The set
      sends the real RC's prop list (see `docs/PROTOCOL.md`) and reads it
      back. Evidence: a second real-RC capture and `seihai-rc` live against
      the fork, offline and online (`docs/lab/2026-11-04-rc-props-set.md`);
      the builder rebuilds the captured 76 in a unit test.
- [ ] Props set: more names than max-hearts, hearts, gralats, arrows,
      bombs, glove and ap; flags and weapons as options; chests once a
      non-zero count is captured.
- [ ] Fork: send the stored flags in 72, so a props set keeps them.
- [ ] List RCs (65). The 2.22 RC (`GSERV024`) sends it from no window, so
      it needs a newer RC or another source.

## Stage 3: accounts

- [x] List (72 → 70), get (77 → 73), set (78), add (70) and delete (71):
      `seihai-rc accounts list/get/set/add/delete`. Evidence: a real RC
      against the fork, one field per apply, and `seihai-rc` live against
      the fork (`docs/lab/2026-11-04-rc-accounts.md`); each builder has a
      byte test from the capture and a case in `tests/fake_servers.rs`.
- [ ] The byte before the ban reason in 73 (most likely the ban time).
      Only 0x20 has been seen; a 73 with another value is not read.
- [ ] Fork: keep load only, admin level and admin worlds from 78 and 70.

## Stage 4: server

Server options `PLI_RC_SERVEROPTIONSGET/SET` (51/52) → `PLO_RC_SERVEROPTIONSGET` (76);
folder config (53/54) → 77; server flags (68/69) → 61; file browser
(89 to 93, 96 to 98, large files 155/156, folder delete 160) →
`PLO_RC_FILEBROWSER_DIRLIST` (65), `_DIR` (66), `_MESSAGE` (67).
Note: ids 51 to 54 meant account actions in RC 1.33; use the modern meanings
(`remotecontrol-lineage.md`).

- [x] File link: list, get, put (`Session::list_dir`, `get_file`, `put_file`).
      No window and no `files` command line in this slice. List and get send
      the path as the rest of the line. The directory is id 66. A file arrives
      as id 102 (time, name, bytes) inside id 100. A put is a `GSTRING` name
      and the bytes, inside id 50, and waits for id 67 `Uploaded file`.
      Widths from the 2026-11-01 loopback run
      (`docs/lab/2026-11-01-file-widths.md`, `docs/PROTOCOL.md`). Evidence:
      `tests/file_link.rs`, and one loopback run against the lab fork
      (`docs/lab/2026-11-07-file-wire.md`). No real client.
- [ ] Server options, folder config, flags, move / delete / rename, folder
      delete (160), large-file packets (155/156).

## Stage 5: NPC Control

Behaviour: Graal_analysis `protocol/npc-control.md` (link; do not paste it).

- [x] Login and framing. Type 3, token `NCL11012`, plain zlib both ways,
      no key byte (`seihai-rc nc login --server 127.0.0.1:14804`). No bytes
      after the password until a loopback capture shows one. Evidence:
      fake NPC-server in `tests/nc_login.rs`. No real client was checked;
      the GServer-v2 lab pack was not available.
- [x] Discovery. On the game-server RC link, id 94 (`PLI_NPCSERVERQUERY`:
      GSHORT player id, then the text `location`) for each nickname that
      contains `(Server)`, then id 79 (`PLO_NPCSERVERADDR`: GSHORT player id,
      then the rest of the line). The NPC-Control socket is opened only after
      exactly one line parses as an address. The separator (comma or spaces)
      still needs a lab check; both shapes are accepted and any other shape,
      including a colon, does not open the socket.
      `seihai-rc nc discover --server 127.0.0.1:14802`. Evidence: fake
      gserver in `tests/nc_commands.rs`. No real client. The fork was not run.
- [x] NPC, class and weapon list / get / set / delete. Class delete (119
      out, 188 in) is a GSTRING name, the same string form as class set. A
      bare class name is not sent and not read. A script that contains a
      line break is not sent. A weapon script whose GSHORT length does not
      fit the message is not parsed. See `docs/PROTOCOL.md`. Evidence:
      `tests/nc_commands.rs`. No real client. The fork was not run for
      class delete.

## Window

egui window behind a `gui` feature once stages 2 and 3 work on the command
line. See `docs/ARCHITECTURE.md` for the threading plan.
