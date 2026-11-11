# RC protocol, as seihai-rc speaks it

Written from the Seihai protocol notes (`squirelworks/seihai`, `documentation/`),
the RC lineage notes and lab runs against the `seihai-gserver-v2` fork on
loopback. Packet ids and wire formats only; no server code was copied.

## Wire types

- GCHAR: one byte, value + `0x20`.
- GSHORT: two GCHARs, high 7 bits first.
- GSTRING: GCHAR length, then the bytes (Latin-1).
- A message is GCHAR id + payload + `\n`. A frame body holds one or more messages.

## Framing

| Login type | Who | Frames |
|---|---|---|
| 1 | RCs from 2000 to 2002 (`GSERV019` to `GSERV023`) | `u16be length` + zlib, both ways, whole session |
| 6 | newer RCs (`GSERV024`, `GSERV025`) | login frame plain zlib; after it `u16be length` + compression byte + gen-5 XORed body, both ways |

Gen 5: compression byte `0x02` none, `0x04` zlib, `0x06` bz2. The XOR stream is
the gen-5 stream from `seihai-core` (`Gen5`), keyed with the login key, one per
direction. seihai-rc sends bodies under 48 bytes uncompressed and the rest zlib.

## Login to a gserver

```
GCHAR type   GCHAR key (type 6 only)   token (8 bytes)   GSTRING account   GSTRING password
```

The fork checks the account with its listserver (`VERIACC2`), so a gserver with
no listserver link refuses every RC ("The login server is offline"). It then
needs the account in the `staff` server option and the RC's IP in the account's
`IPRANGE`.

A refused login gets `PLO_DISCMESSAGE` (16) with the reason, and the link closes.

After a good login the server sends, in the lab:

| Id | Name | Payload |
|---:|---|---|
| 25 | SIGNATURE | GCHAR (73) |
| 74 | RC_CHAT | text: the server's RC welcome lines, "New RC: ...", "Currently online: ..." |
| 190, 47, 180, 103, 194 | listserver connected, staff guilds, status list, max upload size, clear weapons | not read in stage 1 |
| 55 | ADDPLAYER | GSHORT id, GSTRING account, then props: 20 level (GSTRING), 53 list status (GCHAR), 0 nickname (GSTRING), 82 community name (GSTRING) |

Then, live: `ADDPLAYER` (55) when a player or RC logs in, `DELPLAYER` (56,
GSHORT id) when one leaves, and RC chat (74). There is no end marker after the
first `ADDPLAYER`s, so `players list` waits until nothing has come for
`--settle` ms.

The fork also sends `ADDPLAYER` again for an id already in the list when that
player's props change (2026-10-31 lab). `seihai-rc login` prints the first one
as `[online]` and a later one for the same id as `[update]`, and replaces the
stored entry. After `DELPLAYER`, the next `ADDPLAYER` for that id is `[online]`
again. `players list` prints whichever entry was stored last.

An RC's own entry is not sent to it. Other RCs are listed with a level of one
space; seihai-rc shows them as kind `rc`.

After the login the RC may send its nickname: `PLI_PLAYERPROPS` (2) + prop 0
(GSTRING). With none, the fork shows `*account`. RC chat lines go out as
`PLI_RC_CHAT` (79) + text.

While the session is open, seihai-rc sends id 162 with an empty body every
60 s. That is the RC keepalive (the 2010 RC and RC3). Game clients use the
same id to answer a server 108, which this client does not send. The
NPC-Control socket does not send 162: on that socket it is the server's
class script. If the server sends 124, seihai-rc prints the raw bytes in
hex and does not parse them. The layout of 124 is unknown.

## RC list from the listserver

1. Plain zlib frame: id 5 + GCHAR key + 8-byte token.
2. Gen-5 frame: id 1 + GSTRING account + GSTRING password.

Answer (gen-5 frames): id 0 server list (count, then per server 8 GSTRINGs:
type tag + name, language, description, URL, version, players, host, port),
then id 2 status (`\r` = new line), id 3 site URL, id 5 upgrade URL. Names carry
a type tag (`P `, `H `, `3 `, `U `; none for classic). A refused login gets id 4
with the reason and the link closes.

### Secure handshake

`--secure-login` with `--server-key` asks for this same list without the
account or password in the clear. The handshake is Seihai's `SEI00001`
(`documentation/secure-login.md` in the seihai repo). What this client
sends:

1. Plain zlib C1: id 5, GCHAR key, `SEI00001`, the RC token (the same eight
   bytes as the classic handshake), the client's X25519 key. No list name.
2. Plain zlib S1 from the listserver. The signature is checked. If
   `--server-key` is not that key, the client stops and sends nothing more.
3. Plain zlib C2: `SEI00001` and the sealed id 1 payload (`GSTRING` account,
   `GSTRING` password). The id byte is not inside the seal.

After C2 both sides start gen 5 from the key in C1. The client sends no
gen-5 frame of its own before the reply. The reply is the list above (id 0,
then id 2, id 3 and id 5, or id 4).

The player-list form (id 7, and a list name such as `newmain` after the
client key) is not sent. A listserver that only treats id 7 as `SEI00001`
will not answer this client.

## Player actions (stage 2)

`seihai-rc players pm`, `say`, `admin-pm`, `kick` and `warp`. Each layout
here was confirmed by `seihai-rc` itself against the fork (`seihai` at
`1d7843a`) on 2026-11-04: `docs/lab/2026-11-04-live-fork-run.md`. Each command
logs in, waits `--settle` ms for the player list, and refuses a player id
or account that is not in it before anything is sent.

| Action | Client to server | What the command waits for |
|---|---|---|
| Private message | 28 `PRIVATEMESSAGE`: GSHORT count, that many GSHORT player ids, then the text tokenized (one quoted item, `"` doubled), as the Seihai client sends it | nothing; it reads for `--settle` ms and fails only if the link closes |
| Message all | 63 `RC_ADMINMESSAGE`: the text as the rest of the line | as above |
| Admin message to one player | 64 `RC_PRIVADMINMESSAGE`: GSHORT player id, then the text as the rest of the line | as above |
| Disconnect | 61 `RC_DISCONNECTPLAYER`: GSHORT player id, nothing after it | 56 `DELPLAYER` for that id; without it the command fails |
| Warp | 82 `RC_WARPPLAYER`: GSHORT player id, GCHAR x, GCHAR y (both in half tiles, as the level warp messages carry them), then the level name as the rest of the line | nothing; an `ADDPLAYER` resend for that id is printed as `[update]` |

Text and level names are Latin-1. A line break is refused, as it would end
the message. An empty text or level name is refused. A player id over 14
bits does not fit a GSHORT and is refused. X and Y must be 0 to 111.5 in
steps of 0.5. `warp` refuses an RC entry (level of one space).

What the fork did with them in that run:

- 28: the player got 37 with the text as the last tokenized item.
- 63 and 64: the player got 35, `Admin <account>:`, `0xA7`, then the
  text. 64 goes only to the one player.
- 61: no reason text is needed. The player got 16 with the fork's own
  text, and the RC got 56 for that id.
- 82: the fork read x and y as GCHAR half tiles and the level after them,
  and logged the warp. The player, already in that level, got no message.
- The RC got nothing back after 28, 63, 64 or 82.

Still needs a lab check: whether a `0xA7` in our text is read as a line
break by the player, and what the player gets on a warp to another level.

### Comments, account ban, rights and props

Seen on the wire from a real RC (`GSERV024`) against the fork on
2026-11-04: `docs/lab/2026-11-04-rc-player-admin.md`. `seihai-rc` then
ran each get and set below (not 76) live against the fork the same day:
`docs/lab/2026-11-04-live-fork-run.md`. Tokenized text is
one item per line, each quoted with `"` doubled, joined with commas;
`seihai-rc` reads unquoted items too.

| Action | Client to server | Server to client |
|---|---|---|
| Comments get | 85 `RC_PLAYERCOMMENTSGET`: the account as the rest of the line | 63: GSTRING account, then the comments tokenized |
| Comments set | 86 `RC_PLAYERCOMMENTSSET`: GSTRING account, then the comments tokenized | no reply; `players comments --set` reads them back with 85 |
| Ban get | 87 `RC_PLAYERBANGET`: the account as the rest of the line | 64: GSTRING account, GCHAR banned (0 or 1), then the reason |
| Ban set | 88 `RC_PLAYERBANSET`: GSTRING account, GCHAR banned, then the reason tokenized | no reply; `players ban --set` reads it back with 87 |
| Rights get | 83 `RC_PLAYERRIGHTSGET`: the account as the rest of the line | 62: GSTRING account, GINT5 rights, GSTRING IP range, GSHORT length, then that many bytes of folder rights tokenized |
| Rights set | 84 `RC_PLAYERRIGHTSSET`: same as 62 | no reply; `players rights --set` reads them back with 83 |
| Props get | 73 `RC_PLAYERPROPSGET2`: GSHORT player id | 72: GSHORT player id, GSTRING account, GSTRING world, GCHAR props length N, N bytes of props, GSHORT flag count and a GSTRING per flag, GSHORT opened-chest count, GCHAR weapon count, a GSTRING per weapon |
| Props get by account | 74 `RC_PLAYERPROPSGET3`: GSTRING account, then an empty GSTRING | 72, as above, with player id 0 |
| Props set | 76 `RC_PLAYERPROPSSET2`: GSTRING account (no player id), GSTRING world, GCHAR length N, N bytes of props, the same tail as 72, then a GCHAR 0 | no reply; `players props --set` reads them back with 73 or 74 |

GINT5 is five GCHARs, high 7 bits first. Rights bits follow the RC
dialog's checkbox order from bit 0; the capture confirms bit 0
(warp-to-xy) and bit 9 (admin message). The fork at `f585ca6` returned
the folder rights untokenized (`rw levels/*`), and the real RC split that
at the space; the fork at `1d7843a` returns them tokenized. `seihai-rc`
reads an unquoted item up to the next comma, so it keeps the line whole
from either, and it always sends the lines tokenized.

Props are read with `seihai-core`'s player prop table. The tail after
the props was settled by a second capture (2026-11-04,
`docs/lab/2026-11-04-rc-props-set.md`): the real RC sent a flag as GSHORT
1 then the GSTRING `labflag=1`, so the first GSHORT counts flags and the
second opened chests. Only a chest count of 0 has been seen, so
`players props` does not read the weapons after a non-zero one, and a set
from such a reply is refused.

Both captured 76s carry the same props, in this order: 1 to 11, 13, 15,
16, 18, 20 to 23, 26 to 29, 32 and 35. They leave out 0 (nickname), 30
(last IP), 34 (account) and 36 (rating), send 7 (bomb power) as 1 and
21 to 23 as empty when the 72 had none, and send 29 (online seconds) as
0; the fork keeps its own online seconds. `seihai-rc` builds 76 the same
way from the 72 it just read, with the user's changes, and from the
captured 72 it rebuilds the captured 76 byte for byte.

A 76 replaces the stored flags with the ones it carries. Until
2026-11-04 the fork's 72 always had a flag count of 0, because it dropped
every flag when it loaded an account, so any props set through it, the
real RC's included, dropped the account's flags. The fork now keeps them
and its 72 carries them (seihai-gserver-v2 `CHANGES-SEIHAI.md`, checked
there with a real 2.22 RC); `seihai-rc` has not been re-run against that
fork. `players props --set` still warns when a reply has no flags, since
any server that stores flags it does not send would drop them.

The reason in 64 is read as tokenized text. The fork at `f585ca6` stopped
answering after a ban set on an online player (seen once); at `1d7843a` it
disconnects the player with 16 and goes on answering. `players ban` still
fails with "no reply" rather than waiting forever.

## Accounts (stage 3)

`seihai-rc accounts list`, `get`, `set`, `add` and `delete`. Seen from a
real RC (`GSERV024`) against the fork, one field per apply, then run
with `seihai-rc`: `docs/lab/2026-11-04-rc-accounts.md`.

| Action | Client to server | Server to client |
|---|---|---|
| List | 72 `RC_ACCOUNTLISTGET`: GSTRING name pattern (`%` matches any), GSTRING conditions (`adminlevel>0`, may be empty) | 70: a GSTRING per account to the end; empty when none match |
| Get | 77 `RC_ACCOUNTGET`: the account as the rest of the line | 73: GSTRING account, GSTRING password (empty), GSTRING e-mail, GCHAR banned, GCHAR load only, GCHAR admin level, GSTRING admin worlds, one byte (0x20 in every capture), GSTRING ban reason tokenized |
| Set | 78 `RC_ACCOUNTSET`: as 73 without the byte before the reason; an empty password leaves it | no reply; `accounts set` reads back with 77 |
| Add | 70 `RC_ACCOUNTADD`: as 78 | no reply; `accounts add` reads back with 77 |
| Delete | 71 `RC_ACCOUNTDEL`: the account as the rest of the line | no reply; `accounts delete` lists the name |

The RC's dialog has a ban time, but the RC sent none, even with one
typed in. `seihai-rc` sends none either. The fork answers a get or list
sent right after an add or delete with what it had before, so those two
commands wait `--settle` ms before reading back. The fork used to drop
load only, admin level and admin worlds from 78 and 70; it now keeps them
(seihai-gserver-v2 `CHANGES-SEIHAI.md`). `seihai-rc` has not been re-run
against that fork.

## NPC Control

A second socket from the RC game-server link. Layouts follow the behaviour
in Graal_analysis `protocol/npc-control.md` (link only; that note is not
copied here). Fields that note still leaves for a lab check are not given
a byte layout below, and this client does not invent one.

### Login

| | |
|---|---|
| Login type | 3 |
| Token | `NCL11012` (eight bytes, no key byte) |
| Body | `GCHAR 3` + token + `GSTRING` account + `GSTRING` password + `\n` |
| Frames | `u16be` length + zlib body, both ways, including the login and every frame after it. No compression-type byte and no cipher. |

Nothing is sent after the password. A loopback capture of a real NPC-server
may still show an extra byte; until that run, this client does not add one.

The staff client treats the socket as connected once the login frame is
written. `nc login` does the same when the link stays open. Id 16
(`DISCMESSAGE`, text) during that wait is a refusal and the command fails.
Later id 16 lines are printed as `[disconnected]`. An id this client does not
parse is printed as `[nc] id N (L bytes)`. Large raw uploads (id 50
out, id 100 in) are not handled on the NPC-Control socket. The file link
below uses those two ids on the RC socket, because a file may contain line
feeds and an ordinary message may not.

### Finding the NPC-server

On the game-server RC link, after the player list is in:

1. A player whose nickname contains `(Server)` is an NPC-server player.
2. This client sends id 94 (`PLI_NPCSERVERQUERY`): GSHORT player id, then the text `location`.
3. Id 79 (`PLO_NPCSERVERADDR`) is a GSHORT player id, then the rest of the line as text.

The notes call that text an address. Whether the host and port are separated
by a comma or by spaces still needs a lab check. This client accepts either
`host,port` or `host port` (spaces may sit around the comma) and no other
shape. A colon is not a separator. Text that matches neither shape is printed
unchanged, and the NPC-Control socket is not opened.

`seihai-rc nc discover --server 127.0.0.1:14802` logs into the game server,
sends id 94, and prints each id 79. It does not open the NPC-Control socket.
The other `nc` commands take `--discover 127.0.0.1:14802` when the address
should be found first: the socket is opened only after exactly one id 79
line parsed as an address. `--server` on those commands is the NPC-server
itself. Give one of the two, not both.

### NPC, class and weapon commands

GINT3 is three GCHARs, high 7 bits first. A GSTRING is a GCHAR length and
then the bytes. A line feed ends a message, so a script that contains one
is not sent. The raw-data path for a large set (ids 109, 113 and 117) still
needs a lab check.

There is no NPC-list or class-list request in the notes. After login this
client waits `--settle` ms and reads what arrived.

| Action | Client to server | Server to client |
|---|---|---|
| NPC list | (none) | 158 `NC_NPCADD`: GINT3 id, then any subset of GCHAR 50 + GSTRING name, GCHAR 51 + GSTRING type, GCHAR 52 + GSTRING level, in any order. 159 `NC_NPCDELETE`: GINT3 id, and only when those three bytes are the whole payload |
| NPC get | 103 `NC_NPCGET`: GINT3 id | 157 `NC_NPCATTRIBUTES`: the rest of the line. The notes show 157 as the attribute text; that it is the reply to 103 still needs a lab check. If 157 does not arrive the command fails and names what did |
| NPC script | 106 `NC_NPCSCRIPTGET`: GINT3 id | 160 `NC_NPCSCRIPT`: GINT3 id, then the script |
| NPC set | 109 `NC_NPCSCRIPTSET`: GINT3 id, then the script | no reply id is named. The command finishes when the link stays up |
| NPC delete | 104 `NC_NPCDELETE`: GINT3 id | waits for 159 for that id |
| Class list | (none) | 163 `NC_CLASSADD`: the rest of the line is one class name |
| Class get | 112 `NC_CLASSEDIT`: the class name as the rest of the line | 162 `NC_CLASSGET`: GSTRING name, then the script |
| Class set | 113 `NC_CLASSADD`: GSTRING name, then the script | no reply id is named |
| Class delete | 119 `NC_CLASSDELETE`: GSTRING name, the same string form as class set | 188: GSTRING name, and only when that string is the whole payload |
| Weapon list | 115 `NC_WEAPONLISTGET`: no payload | 167: repeated GSTRING names |
| Weapon get | 116 `NC_WEAPONGET`: the weapon name as the rest of the line | 33: GSTRING name, GSTRING image, GSHORT length, then that many script bytes |
| Weapon set | 117: GSTRING name, GSTRING image, then the script | no reply id is named |
| Weapon delete | 118: the weapon name as the rest of the line | no reply id is named |

Still needs a lab check, and so is not turned into fields:

- Id 158 with a property other than 50, 51 or 52. The whole payload stays raw. Whether a missing property clears a value already stored also still needs a lab check; this client leaves the stored value.
- Id 159 with bytes after the GINT3.
- A bare class name (the rest of the line, as id 112 sends) for id 119 and id 188. Id 119 sends a GSTRING, the same string form as id 113's name, and id 188 is read only as that GSTRING. The bare name is not sent and not read.
- Ids 112, 116 and 118. The notes write `str` where a length byte was seen, and they do not write it for these three, so this client sends the name as the rest of the line. A lab check should confirm there is no length byte.
- Id 33 when the GSHORT length runs past the message. A GINT3 length for a large script still needs a lab check and is not tried. A script byte that is a line feed still needs a lab check: the frame reader stops at a line feed.
- Each id 167 replaces the stored weapon list. Whether a second one is a full list or an addition still needs a lab check. The notes describe the packet as the list, so this client treats each one as the whole list.
- Attribute text, class names and scripts are the rest of the one message that carried them. A line feed inside that text still needs a lab check.

## File link (list, get, put)

Ids are the file-browser ids named under stage 4 in `docs/NEXT.md`. The
layout is what the gserver-v2 fork did with list, get and put on the
loopback run of 2026-11-01 (`docs/lab/2026-11-01-file-widths.md`).

A raw block is the shape `split_messages` already uses for id 100: `GCHAR`
id, `GINT3` byte count, `\n`, then that many bytes (the next message, id
included, line feeds allowed). The fork wraps an outbound file the same
way with id 50, so a level's line feeds stay inside the put. `split_messages`
does not treat id 50 as raw; the sender builds the block itself. Inbound,
id 100 is unwrapped before this client sees the message, so a file arrives
as id 102.

On that run, a length byte in front of the folder made id 90 match nothing,
so the fork sent no reply. A length byte in front of the get path was logged
as the first character of the path. The reply was id 102. Its payload began
`26 77 3a 62 7e 33` and then `onlinestartlocal.nw` (19 bytes) and then the
level, the same length as the file. Those six bytes are a `GINT5`
modification time and the name's length: `0x33` is 19. There is no `GINT3`
size. A put whose name was a `GSTRING` was stored with the three size bytes
still in front of the file, and the fork answered id 67, `Uploaded file rc-probe.txt`.

| | Id | Payload |
|---|---:|---|
| List | 90 `RC_FILEBROWSER_CD` | The folder, the rest of the line. No length byte. `""` is an empty payload. The fork answers when the text is a folder the account may open; a right such as `world/*` is stored as `world/`. |
| Directory | 66 `RC_FILEBROWSER_DIR` | `GSTRING` folder, then one record per file: a space, a `GCHAR` length, a `GSTRING` name, a `GSTRING` rights text, a `GINT5` size, a `GINT5` modification time. |
| Folder line | 67 `RC_FILEBROWSER_MESSAGE` | `Folder changed to ` and the folder, before id 66. Any other id 67 is a refusal. |
| Get | 92 `RC_FILEBROWSER_DOWN` | The path, the rest of the line. No length byte. The fork joins it to the folder from the last list. |
| File | 100 raw, then 102 `PLO_FILE` | `GINT5` modification time, `GCHAR` name length, the name, the file bytes. The raw length counts one line feed after the file; that byte is not part of the file. |
| Put | 50 raw, then 93 `RC_FILEBROWSER_UP` | `GSTRING` name, then the file bytes. No size field. |
| Stored | 67 `RC_FILEBROWSER_MESSAGE` | `Uploaded file ` and the name. Any other id 67 is a refusal. |

Id 65 `RC_FILEBROWSER_DIRLIST` is the reply to id 89 `START`, which this
client does not send. Not sent, layouts unknown: 91 `END`, 96 move, 97
delete, 98 rename, 155/156 large file, 160 folder delete. A file longer
than 60 KiB is refused. One frame's length is a `u16`, and those large-file
ids are how a bigger file would travel once a lab run shows their layout.

Paths are relative, `/`-separated. `..`, a leading `/`, a backslash and a
colon are refused. A trailing `/` is kept. A path longer than 223 bytes is
refused: a put name is one `GCHAR`, which cannot say more.
