# Next steps

Stage 1 and most of stages 2, 3 and 5 are merged; the boxes below say what is left. Work in this order,
one branch and pull request each. Ids below are packet ids (facts). Payload
layouts marked **check** must be confirmed in the lab against the fork before
they are relied on; write what you see into `docs/PROTOCOL.md`.

## Small follow-ups to stage 1

- [x] `login`: print `[update]` rather than `[online]` when `ADDPLAYER` comes for
      an id already listed (the fork resends it when props change). The stored
      entry is replaced; after `DELPLAYER` the next one is `[online]` again.
      Evidence: fake gserver in `tests/fake_servers.rs`. Fork run
      2026-11-12 (`docs/lab/2026-11-12-fork-lab-pass.md`): the fork resends `ADDPLAYER` for an RC that
      joins (`[update] 5 labrc2 (RC)`), but sent none for a player whose
      props an RC set. The resend was also seen on 2026-10-31
      (`docs/lab/2026-10-31-stage1.md`).
- [x] RC keepalive: an empty id 162 every 60 s on the game-server RC link
      (not on the NPC-Control socket), and id 124 printed as raw hex, layout
      unknown. Evidence: `tests/keepalive.rs`. No real client was run
      (PR #21, issue #19).
- [x] Read `PLO_OTHERPLPROPS` (8) for RCs and update `PlayerList`: a listed
      id gets the nickname, level, status or community it carries, shown as
      `[update]`; an unlisted id is not added. The fork sends 8 to an RC
      for the props it broadcasts (a head change, RC nicknames), not for
      gralats or gani. Evidence: unit test, fake gserver, and the fork run
      of 2026-11-14 (`docs/lab/2026-11-14-fork-lab-pass.md`).
- [x] `--nick` default: with no `--nick` the CLI sends `*account`. Real RCs
      always send a nickname, and `*account` is what the fork shows for an RC
      that sends none, so the screen is the same and the wire matches a real
      RC. `--no-nick` sends nothing. Evidence: fake gserver.
- [x] `players list --json` (and `rcs list --json`) for scripts: one array,
      the `--tsv` fields.
- [x] Optional identity string after the password in the type-6 login:
      `--identity TEXT`, sent as the rest of the login line, off by default
      and refused with `--rc classic`. The fork takes any text: it sets
      the platform to it and forwards it to its listserver. Evidence: unit
      test, fake gserver and the fork run of 2026-11-14 (`docs/lab/2026-11-14-fork-lab-pass.md`).
- [x] Upstream gs2lib packet names: `PLI_PONG` (162, was `PLI_PING`) and
      `PLO_RC_UNKNOWN124` (124, was `PLO_RC_PLAYERFLAGS`). Names only.
- [x] A test against the real seihai-listserver, not only the fake one:
      `tests/real_listserver.rs` starts it (dev-dependency at tag `v0.1.0`)
      on free loopback ports, registers a hidden (HQ level 0) and a gold
      gserver, and checks `fetch_rc_list` and `servers` list both, the
      hidden one tagged `U`, and that a wrong password is refused
      (seihai-listserver issue #19, item 2).
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
| List RCs | from the player list (65 is never sent) | |

Command line: `seihai-rc players pm ID TEXT`, `players say TEXT` (message all),
`players admin-pm ID TEXT` (private admin message),
`players kick ID`, `players warp ID LEVEL X Y`, `players props ID`,
`players rights ACCOUNT [--set ...]`, `players ban ACCOUNT [--set ...]`,
`rcs list`.

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
- [x] Opened chests in 72 and 76: a GSHORT count, then each chest as a
      GSTRING of GCHAR x, GCHAR y and the level. A props set sends them
      back unchanged, flags sorted and status without the paused bit, as
      the 2004 RC did; the builder rebuilds that RC's 76 byte for byte.
      Evidence: `docs/lab/2026-11-13-archived-rc-captures.md` and a unit test.
- [x] Props set: more names than max-hearts, hearts, gralats, arrows,
      bombs, glove and ap; flags and weapons as options. Now also
      bomb-power, sword and shield (power and image apart, each keeping
      the other half), gani, head (an image), body, colors, x, y, level,
      magic, kills and deaths, and `--add-flag`, `--remove-flag`,
      `--add-weapon`, `--remove-weapon`. Widths from `seihai-core`'s
      prop table, which reads the captured 72s; a set reads back every
      prop, flag and weapon it changed and fails if one differs.
      Evidence: unit tests in `src/players.rs` and
      `tests/fake_servers.rs`. Fork run 2026-11-14 (`docs/lab/2026-11-14-fork-lab-pass.md`):
      sword, shield, magic, kills, deaths, x, y, head, body, gani, colors,
      flags and weapons stored and read back. Bomb power and level (on an
      account not online) are not kept by the fork (seihai-gserver-v2 #83);
      seihai-rc reports both.
- [x] Fork: send the stored flags in 72, so a props set keeps them. Done in
      seihai-gserver-v2 (flags kept on load; `CHANGES-SEIHAI.md`, checked
      with a real 2.22 RC, and with the 2004 RC on 2026-11-13).
- [x] List RCs, for v0.2.0 (owner's call, 2026-11-12): `seihai-rc rcs list`.
      No RC client from 1.21 to 2.05 sends id 65 and no server answers it:
      RCs are listed through the normal player list, where an RC is a player
      with an empty level. So it is built from `PlayerList` and `is_rc()`,
      with no new packet ([Graal_analysis RC write-up](https://github.com/squirelworks/Graal_analysis/issues/410),
      2026-11-12). Tested against the fake gserver and in the fork run of
      2026-11-14 (`docs/lab/2026-11-14-fork-lab-pass.md`).

## Stage 3: accounts

- [x] List (72 → 70), get (77 → 73), set (78), add (70) and delete (71):
      `seihai-rc accounts list/get/set/add/delete`. Evidence: a real RC
      against the fork, one field per apply, and `seihai-rc` live against
      the fork (`docs/lab/2026-11-04-rc-accounts.md`); each builder has a
      byte test from the capture and a case in `tests/fake_servers.rs`.
- [x] The byte before the ban reason in 73 is a GSTRING ban length
      (`3600` with `BANLENGTH 3600`; the RC never sends it back).
      `accounts get` prints it. Evidence: `docs/lab/2026-11-13-archived-rc-captures.md`.
- [x] Account reset (75): `accounts reset ACCOUNT --yes`, the account as
      the rest of the line, as the 2004 RC sends it from its account menu.
      Evidence: `docs/lab/2026-11-13-archived-rc-captures.md` (second pass) and a unit test.
- [x] Fork: keep load only, admin level and admin worlds from 78 and 70.
      Done in seihai-gserver-v2 (`CHANGES-SEIHAI.md`). Checked with
      `seihai-rc accounts set` against the fork on 2026-11-14 (`docs/lab/2026-11-14-fork-lab-pass.md`).

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
- [x] `files ls|get|put|mv|rename|rm|rmdir` command line. Move (96),
      delete (97), rename (98) and folder delete (160) are sent with the
      layouts in `docs/PROTOCOL.md`. Fork run 2026-11-12 (`docs/lab/2026-11-12-fork-lab-pass.md`): delete
      and rename work as sent; move was corrected to `GSTRING` folder, then
      the name, the order the fork reads; folder delete was not run. No real
      RC was captured doing them. Each command lists the folder first (the fork names a file
      by the folder from the last list), prints the server's id 67 lines,
      and lists again to see whether the change happened. `rm` and `rmdir`
      need `--yes`. Evidence: `tests/files_cli.rs` (fake gserver) and unit
      tests. No real client.
- [x] Capture 96, 97, 98 and 160 from a real RC: move and delete from the
      2004 RC, rename and folder delete from RC3. All matched; 160 carries
      a trailing `/`, which `files rmdir` now adds. Evidence: `docs/lab/2026-11-13-archived-rc-captures.md`.
- [x] `server options|folders|flags [--set-file PATH]`: get 51/53/68,
      replies 76/77/61, set 52/54/69. Options and folder config as
      tokenized text, flags as a count and one `GSTRING` each, all
      **check** (no real RC captured). A set reads the value back, prints
      the line counts before and after, and fails if the server kept
      something else. Evidence: `tests/server_cli.rs` (fake gserver) and
      unit tests. No real client.
- [x] Capture 51 to 54 and 68/69 from a real RC: the 2004 RC, get and
      set. The layouts matched; the fork's unquoted 76/77 broke the real
      RC's set (seihai-gserver-v2 #63). Evidence: `docs/lab/2026-11-13-archived-rc-captures.md`.
- [x] Large-file packets: put as 155, put blocks of 32000 bytes, 156;
      get joins 68, 84, the id 102 pieces and 69. Layouts **check**, from
      the fork's behaviour. Evidence: `tests/file_link.rs` (fake gserver)
      and unit tests. No real client.
- [x] Capture 155/156 from a real RC putting a file over 60 KiB (RC3,
      81200 bytes), and a get of one over 32000 bytes (2004 RC, 50000
      bytes). The layouts matched; the fork stored each put piece one byte
      long (#63). Evidence: `docs/lab/2026-11-13-archived-rc-captures.md`.

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
      exactly one line parses as an address. The fork sends `host,port`
      with a comma and no spaces (run 2026-11-12, `docs/lab/2026-11-12-fork-lab-pass.md`); both shapes
      are accepted and any other shape, including a colon, does not open
      the socket. `seihai-rc nc discover --server 127.0.0.1:14802`.
      Evidence: fake gserver in `tests/nc_commands.rs` and the fork run.
- [x] NPC, class and weapon list / get / set / delete. Class delete (119
      out, 188 in) carries the bare class name, and weapon get's 33 has a
      GCHAR 0 after the name and a GCHAR 1 after the image: both corrected
      to what the fork does in the run of 2026-11-12 (`docs/lab/2026-11-12-fork-lab-pass.md`), where NC
      login, NPC list/get/script, class list/set/get/delete and weapon
      list/get/set/delete all worked against the fork. See
      `docs/PROTOCOL.md`. Evidence: `tests/nc_commands.rs` and the fork run.
- [x] Real-RC NC capture (2004 RC with `NCL11012`, RC3 with `NCL21075`,
      2026-11-13): scripts and flags go tokenized, so a script may hold
      line breaks; weapon scripts go with `0xA7` line breaks; 33's length
      high byte is not masked (21212 = `c5 7c`); 192 is read as RC3's
      weapon reply; 157/160/161/162 are read back into lines. New: `nc npc
      add`, `reset`, `warp` and `flags [--set-file]` (111, 105/29, 107,
      108/161, 110). Evidence: `docs/lab/2026-11-13-archived-rc-captures.md`, unit tests and
      `tests/nc_commands.rs`.
- [x] NPC, class and weapon script sets read that same target back (106,
      112, 116) and succeed only when the reply matches the text this
      client can represent. RC chat, a class name, a weapon list and
      another target's script do not count. A match is that read, not a
      restart. The window ends a script read at the login timeout. A save
      then reports no matching reply while the link is still up, and the
      next same-target script stays with that old read so it cannot settle
      a newer open or save. Serialization with the eventual #57 repair
      must recheck expected-versus-reply normalization with independent
      producer fixtures. Evidence: `tests/nc_commands.rs` and
      `tests/window_tabs.rs`
      (`nc_save_expires_while_the_peer_stays_up_and_a_late_reply_is_not_the_next_save`)
      against loopback fakes. No real client.

## Window

- [x] egui window behind the `gui` feature: `seihai-rc gui`. Login form,
      players and RCs, RC chat, server messages, and per-player PM, admin
      message, warp, props read and disconnect; admin message to all.
      The session runs on a worker thread (`src/live.rs`: `LiveSession`
      takes `Command`s and sends `Update`s; `RcView` is the state the
      window draws). Evidence: unit tests in `src/live.rs`,
      `tests/live_session.rs` (fake gserver, both login types: every
      command's bytes, props, a closed link and a refused login), and a
      screenshot under Xvfb against a loopback stand-in. CI builds and
      lints it on Linux and Windows. Logged in to the fork and showed its
      players on 2026-11-14 (`docs/lab/2026-11-14-fork-lab-pass.md`).
      Needs rustc 1.95 (eframe 0.37); the command line builds on older.
- [x] Accounts, Files and NPC Control tabs in the window. Accounts:
      list with a pattern, read, edit (e-mail, banned, load only, admin
      level, admin worlds, ban reason; saved with 78 and read back) and
      delete (two clicks). Files: list a folder, download to a local
      file, upload, rename and delete (two clicks), each listing the
      folder first as the command line does. NPC Control: find the
      NPC-server through the game server (94 → 79) or type its address,
      then a second worker thread (`src/live_nc.rs`) logs in on the
      type-3 socket; NPCs (attributes, script edit, reset, delete),
      classes (open, new, save, delete) and weapons (open, new with an
      image, save, delete). Evidence: unit tests in `src/live.rs` and
      `src/live_nc.rs`, `tests/window_tabs.rs` (fake gserver and fake
      NPC-server: every command's ids on the wire), and screenshots of
      each tab under Xvfb against loopback stand-ins.
- [x] Server tab: server options (51 → 76, set 52), folder config
      (53 → 77, set 54) and flags (68 → 61, set 69). Opens with a read;
      Save needs a second click, then reads back and logs whether the
      server kept what was sent. Evidence:
      `tests/window_tabs.rs::server_tab_reads_and_saves_options_and_flags`.
- [x] NPC Control editor selection (#53): reopening an unchanged script
      reloads it; only the selected target's reply fills the editor.
      Existing scripts cannot be edited/saved while their read is pending.
      Every selection/new target disarms Delete. Evidence: GUI regressions
      in `src/ui.rs`; no real-fork or macOS runtime run for this fix.
- [x] Starting another login clears the previous server's account and file
      forms, pending download, NPC-Control connection and editor, server
      read cache, and armed actions. Each new server is read afresh.
      Evidence: GUI regression test in `src/ui.rs`; this does not replace
      the outstanding real-fork run of the four tabs.
- [ ] Later: the macOS polish (owner's polishing phase).

## NPC Control readback follow-up

- [x] Retain trailing empty fields (#57, partial #56).
      Decoder and NPC/class reply regressions cover quotes,
      backslashes and internal/trailing empty lines. Empty input still has no
      fields; saving still omits one final line feed, separately from decoding.
- [ ] Escaped-backslash replies (#56): the current Rust server preserves
      literal backslashes, while an escaping producer doubles them. Identical
      payloads are ambiguous; generic decoding preserves literals. Supporting
      escaped producers requires an explicit producer contract, not guessing
      from bytes. #57 does not resolve this part of #56.
- [ ] Real-fork GUI acceptance remains outstanding (#27), including reopening,
      editing and saving scripts after reconnect or target switches. This parser
      repair does not add correlated save acceptance: transport success and
      chat/list notifications do not establish accepted content. A same-target
      GET observes content at that instant, not persistence across restart.
