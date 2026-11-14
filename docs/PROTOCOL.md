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
(GSTRING). With none, the fork shows `*account`. Real RCs from 2000 on send
one, so `seihai-rc` sends `*account` when no `--nick` is given (`--no-nick`
sends none). The 2004 RC, the 2002/05/02 RC and RC3 also send prop 53 (list
status) as `21`; the 2001 and 2002/01/04 RCs send `*account - away` with no
prop 53; rc133 (`GSERV014`) sends no nickname (lab 2026-11-13).

A type-6 login may carry more text after the password GSTRING, up to the line
feed: an identity string. The 2004 RC sends none. RC3 (`GSERV025`) sends it
tokenized: `win,<32 hex>,"",<32 hex>` (platform, a machine id, an empty item,
a second machine id), and the fork accepted it (lab 2026-11-13). The fork
takes any text there as the platform and forwards it to its listserver
(`--identity test`, lab 2026-11-14). `seihai-rc` sends it only with
`--identity TEXT`.

Older RCs log in with type 1 and plain zlib both ways: `GSERV019` to
`GSERV023` (2000/8/19 to 2002/05/02) and `GSERV014` (rc133). The fork let
each of them in. The 2000 and 2001 builds also send their start dialog's
timers right after login, one message each when changed: 55 respawn, 56
horse lifetime, 57 alignment increment, 58 baddy respawn, each a GINT4
(301 is `20 20 22 4d`). The fork ignores all four.

The fork at `a3b6ac23` sent a type-6 RC one plain zlib frame (id 42, no
compression byte, no gen 5) before the login reply (seihai-gserver-v2 #63
fixes that).

`PLO_OTHERPLPROPS` (8) is GSHORT id, then props as in `ADDPLAYER`. If it comes
for a listed id, seihai-rc applies the nickname, level, list status and
community name it carries and shows `[update]`; reading stops at any other
prop. The fork sends 8 to an RC for the props it broadcasts: a head change
arrived as `28 2023 2b 8d head5.png`, and RC nicknames the same way; a gralats
or gani set sent none (lab 2026-11-14). RC chat lines go out as
`PLI_RC_CHAT` (79) + text.

While the session is open, seihai-rc sends id 162 with an empty body every
60 s. That is the RC keepalive (the 2010 RC and RC3; RC3 sent it about
every 30 s in the 2026-11-13 run). Game clients use the
same id to answer a server 108, which this client does not send. The
NPC-Control socket does not send 162: on that socket it is the server's
class script. If the server sends 124, seihai-rc prints the raw bytes in
hex and does not parse them. The layout of 124 is unknown.

## RC list from the listserver

1. Plain zlib frame: id 5 + GCHAR key + the list name `rc` (two bytes,
   not the gserver token). The 2004 RC sent `25 4d 72 63` (lab
   2026-11-13).
2. Gen-5 frame: id 1 + GSTRING account + GSTRING password.

With `--rc classic` the handshake is the 2002/05/02 RC's: plain zlib both
ways, id 0 + `rc` and the same id 1 in **one** frame, as rc141 sends them
(`0017 78da53284ae65254cd494c2a4a069105e55c0031c2058f`, inflated
` rc\n!%labrc%labpw\n`). Sent as two frames, the listserver answers the lone
id 0 as a login for `player` and then answers the real login too, so the
list arrives twice (seihai-rc live run, 2026-11-13). Its list comes back without type
tags. RC3 instead sends a game client's id 7 with `G3D30123` and the list
name `rc2`; `seihai-rc` does not send that form.

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
| Props get | 73 `RC_PLAYERPROPSGET2`: GSHORT player id | 72: GSHORT player id, GSTRING account, GSTRING world, GCHAR props length N, N bytes of props, GSHORT flag count and a GSTRING per flag, GSHORT opened-chest count and a GSTRING per chest (GCHAR x, GCHAR y, the level name), GCHAR weapon count, a GSTRING per weapon |
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
second opened chests. A third capture (2026-11-13,
`docs/lab/2026-11-13-archived-rc-captures.md`), of an account with two
`CHEST` lines, showed each chest as a GSTRING whose body is GCHAR x,
GCHAR y and the level name: `35 3e 34 onlinestartlocal.nw` is x 30, y 20.
The 2004 RC sent both chests back unchanged.

Both captured 76s carry the same props, in this order: 1 to 11, 13, 15,
16, 18, 20 to 23, 26 to 29, 32 and 35. They leave out 0 (nickname), 30
(last IP), 34 (account) and 36 (rating), send 7 (bomb power) as 1 and
21 to 23 as empty when the 72 had none, and send 29 (online seconds) as
0; the fork keeps its own online seconds. `seihai-rc` builds 76 the same
way from the 72 it just read, with the user's changes, and from the
captured 72 it rebuilds the captured 76 byte for byte. The 2026-11-13
capture adds two rules, both followed: the RC sends the flags sorted (the
72 had `quest.done=yes, labflag=1`, the 76 `labflag=1, quest.done=yes`),
and it sends status (18) without the paused bit (21 went back as 20).

`players props --set` changes more props than the RC captures did:
the widths come from `seihai-core`'s player prop table, the one that
reads the captured 72s, and none of these was captured from a real RC
setting it. Sword (8) is GCHAR power with no image up to 4, else GCHAR
(power + 30) and a GSTRING image; shield (9) the same with 3 and 10;
head (11) as an image is GCHAR (length + 100) and the name; colors (13)
five GCHARs; x and y (15, 16) GCHAR half tiles; gani (10), level (20)
and body (35) GSTRINGs; kills and deaths (27, 28) GINT3; bomb power (7)
and magic (26) GCHAR. `sword=` or `sword-image=` alone keeps the other
half from the 72. `--add-flag`, `--remove-flag`, `--add-weapon` and
`--remove-weapon` change the lists 76 carries; a flag is matched by the
name before `=`.

A 76 replaces the stored flags with the ones it carries. Until
2026-11-04 the fork's 72 always had a flag count of 0, because it dropped
every flag when it loaded an account, so any props set through it, the
real RC's included, dropped the account's flags. The fork now keeps them
and its 72 carries them (seihai-gserver-v2 `CHANGES-SEIHAI.md`, checked
there with a real 2.22 RC, and with the 2004 RC on 2026-11-13: both
`FLAG` and both `CHEST` lines stayed after a set). `players props --set` still warns when a reply has no flags, since
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
| Get | 77 `RC_ACCOUNTGET`: the account as the rest of the line | 73: GSTRING account, GSTRING password (empty), GSTRING e-mail, GCHAR banned, GCHAR load only, GCHAR admin level, GSTRING admin worlds, GSTRING ban length, GSTRING ban reason tokenized |
| Set | 78 `RC_ACCOUNTSET`: as 73 without the ban length; an empty password leaves it | no reply; `accounts set` reads back with 77 |
| Add | 70 `RC_ACCOUNTADD`: as 78 | no reply; `accounts add` reads back with 77 |
| Delete | 71 `RC_ACCOUNTDEL`: the account as the rest of the line | no reply; `accounts delete` lists the name |
| Reset | 75 `RC_PLAYERPROPSRESET`: the account as the rest of the line (2004 RC, after a confirm dialog, 2026-11-13) | an RC chat line `<rc> has reset the attributes of account: <account>` |

The RC's dialog has a ban time, but the RC sent none, even with one
typed in. `seihai-rc` sends none either. The fork sends it in 73 as a
GSTRING: empty in the 2026-11-04 captures, `3600` with `BANLENGTH 3600`
in the account file (2026-11-13), which the 2004 RC showed as the ban
time. `accounts get` prints it as `ban length`. In 1999 to 2002 RCs
(GSERV019 to 023) the list and get are the same 72/70 and 77/73. The fork answers a get or list
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

The notes call that text an address. Against seihai-gserver-v2 (lab run
2026-11-12, `docs/lab/2026-11-12-fork-lab-pass.md`) it is `host,port` with a
comma and no spaces: `127.0.0.1,14804`. This client accepts either
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
then the bytes. A line feed ends a message, so no script carries one: the
2004 RC and RC3 (lab 2026-11-13, `docs/lab/2026-11-13-archived-rc-captures.md`)
send NPC and class scripts and NPC flags tokenized, one item per line, and
weapon scripts with each line break as `0xA7`. Tokenized here is the RCs'
own form: an item is quoted only when it starts with a quote or holds a
space, a byte outside 33..126, a comma or a slash, so `}` and
`labnpcflag=1` stay bare. The server's tokenized replies (157, 160, 161,
162) are read back into lines. Every set in that run, an 8800-byte and a
21232-byte weapon included, went as one ordinary message: no raw-data
path.

There is no NPC-list or class-list request in the notes. After login this
client waits `--settle` ms and reads what arrived.

| Action | Client to server | Server to client |
|---|---|---|
| NPC list | (none) | 158 `NC_NPCADD`: GINT3 id, then any subset of GCHAR 50 + GSTRING name, GCHAR 51 + GSTRING type, GCHAR 52 + GSTRING level, in any order. 159 `NC_NPCDELETE`: GINT3 id, and only when those three bytes are the whole payload |
| NPC get | 103 `NC_NPCGET`: GINT3 id | 157 `NC_NPCATTRIBUTES`: the variables dump tokenized, one item per line, no id |
| NPC script | 106 `NC_NPCSCRIPTGET`: GINT3 id | 160 `NC_NPCSCRIPT`: GINT3 id, then the script tokenized |
| NPC set | 109 `NC_NPCSCRIPTSET`: GINT3 id, then the script tokenized | an RC chat line `NPC script of <name> updated by <rc>` |
| NPC add | 111: `name,id,type,scripter,level,x,y` tokenized | 158 for the new NPC, to every NC, and an RC chat line. A level that does not exist gives an empty 52 |
| NPC reset | 105: GINT3 id | 29: GINT3 id, and an RC chat line |
| NPC warp | 107: GINT3 id, GCHAR x × 2, GCHAR y × 2, then the level as the rest of the line | none |
| NPC flags | 108: GINT3 id | 161: GINT3 id, then the flags tokenized |
| NPC flags set | 110: GINT3 id, then the flags tokenized | an RC chat line per flag |
| NPC delete | 104 `NC_NPCDELETE`: GINT3 id | 159 for that id. The fork at `a3b6ac23` sent it twice (seihai-gserver-v2 #63 fixes that) |
| Update level | 62 on the **RC** socket: GSHORT count, a GSTRING per level | 159 on the NC socket for each NPC in the level |
| Class list | (none) | 163 `NC_CLASSADD`: the rest of the line is one class name |
| Class get | 112 `NC_CLASSEDIT`: the class name as the rest of the line | 162 `NC_CLASSGET`: GSTRING name, then the script tokenized |
| Class set | 113 `NC_CLASSADD`: GSTRING name, then the script tokenized | 163 with the bare name, and an RC chat line |
| Class delete | 119 `NC_CLASSDELETE`: the class name as the rest of the line | 188: the class name as the rest of the line, sent only when the class existed |
| Weapon list | 115 `NC_WEAPONLISTGET`: no payload | 167: repeated GSTRING names, always the whole list. A set or delete sends it again; the fork at `a3b6ac23` sent that only to the NC that made the change (#63 fixes that) |
| Weapon get | 116 `NC_WEAPONGET`: the weapon name as the rest of the line | To `NCL11012`, 33: GSTRING name, GCHAR 0, GSTRING image, GCHAR 1, a two-byte length, then the script joined by `0xA7`. The length is a GSHORT, high 7 bits first, but above 16383 the fork's high byte runs past 7 bits: 21212 went as `c5 7c`, and the 2004 RC read it, so the high byte is not masked. To `NCL21075` (RC3), 192: GSTRING name, GSTRING image, then the script joined by `0xA7` as the rest of the line, no length |
| Weapon set | 117: GSTRING name, GSTRING image, then the script joined by `0xA7`, not tokenized, no length | an RC chat line; a new weapon also gets 167 |
| Weapon delete | 118: the weapon name as the rest of the line | an RC chat line and 167 |

A script set is not kept just because the server accepted the bytes or
answered with RC chat (id 74), a class name (163) or a weapon list (167).
`nc npc set`, `nc class set` and `nc weapon set`, and the same buttons in
the window, send the get for that same NPC, class or weapon (106, 112 or
116) immediately after the set. The command succeeds only when id 160, 162,
33 or 192 for that target carries the text this client can represent for
what it sent. For an NPC or class, carriage returns fold into line feeds and
one final line feed is omitted. For a weapon, line breaks are `0xA7`, a
trailing line break stays, and a section sign in the text is a line break.
A match is the script observed on that read. It does not show that the
script is still there after a restart. A different script, an `error:` chat
line, a disconnect (id 16) and no same-target reply are different failures.
Another target's script is not the reply. A chat line such as `NPC script
of <name> updated by <rc>` is not the script.

The window's NPC Control worker waits for that read for the same timeout
it used to log in. When the timeout passes and the link is still up, a
save is the no-matching-reply failure, and another open or save for that
target may be sent. A script reply has no request number. Reads for one
target are paired in the order they were sent, including a read whose
wait already ended: the next same-target script is that old read's, it is
not shown as a newer read's script, and it is not that newer read's
result. The script after that one is. A peer that sends only one script
for several gets of that target retires the oldest timed-out read and
leaves the newer read still waiting.

`representable_npc_class_script` calls the shared decoder. A trailing
empty field in that normalized text has to come back on the same read.
An independent reply that drops it, or adds another, is not a match.
Issue #56's escaped-producer contract is still open. A match is the
script observed on that read. It does not show that the script is still
there after a restart.

Ids above 114 the fork sent to an NC in that run: 157 to 163, 167, 188 and
192. Below it: 25, 29, 33 and 74. RC3 also sends an empty 103 every ~30 s
on the NC socket as a keepalive; the fork does not answer it.

Not seen, and so not turned into fields:

- Id 158 with a property other than 50, 51 or 52. The whole payload stays raw. No UI path in the 2004 RC or RC3 produced one. This client leaves a stored value when a property is missing.
- Id 159 with bytes after the GINT3.

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
| Put | 50 raw, then 93 `RC_FILEBROWSER_UP` | `GSTRING` name, then the file bytes. No size field. As for id 100, the raw length counts one line feed after the message (32015 for a 32014-byte 93 from the 2004 RC), and the block ends with it. The fork with #63 drops that byte; a count without it cost the file's own last byte in the seihai-rc live run of 2026-11-13. |
| Stored | 67 `RC_FILEBROWSER_MESSAGE` | `Uploaded file ` and the name. Any other id 67 is a refusal. |

| Move | 96 `RC_FILEBROWSER_MOVE` | `GSTRING` target folder, then the file name in the folder from the last list as the rest of the line. The fork answers id 67 `Moved file <from> to <to>`. |
| Delete | 97 `RC_FILEBROWSER_DELETE` | The file name in the folder from the last list, the rest of the line, with no confirm dialog in the 2004 RC. The fork answers id 67 `Deleted file <name>`. |
| Rename | 98 `RC_FILEBROWSER_RENAME` | `GSTRING` old name, `GSTRING` new name, both in the folder from the last list. The fork answers id 67 `Renamed file <old> to <new>`. |
| Folder delete | 160 `RC_FOLDERDELETE` | The folder with a trailing `/`, the rest of the line (`c0 world/labgone/` from RC3). `seihai-rc` adds the slash when it is missing. The fork at `a3b6ac23` sent 16 and dropped the link both for a folder that was not empty and after removing an empty one (#63 fixes that). With #63 it answers 67 `Folder world/labdir/ has been removed.`, then 65 and 66 `/`. A later 90 of the deleted folder still answers `Folder changed to` and an empty 66 when the account's rights cover it, so `seihai-rc files rmdir` reads the 67 line instead of listing again. |

All four match real RCs: move and delete from the 2004 RC, rename and
folder delete from RC3 (lab 2026-11-13). After each, the real RC sends 90
for the current folder again. What the server
answers to mv, rename and rm is not relied on: `seihai-rc files mv|rename|rm`
prints any id 67 lines and lists the folder again to see the result.
`rmdir` goes by the 67 line (see the table), and lists again only when the
server sends no 67 at all.

Id 65 `RC_FILEBROWSER_DIRLIST` is the reply to id 89 `START`, which this
client does not send. Not sent, layout unknown: 91 `END`.

### Large files

One frame's length is a `u16`, so a file over 60 KiB (`MAX_FILE_BYTES`)
travels in pieces. RC3 put an 81200-byte file and the 2004 RC got a
50000-byte one this way (lab 2026-11-13).

| Direction | Ids | Layout |
|---|---|---|
| Put | 155 `RC_LARGEFILESTART` | The name, the rest of the line. |
| | 93 inside 50, repeated | Ordinary put blocks, `GSTRING` name (the same name), then at most 32000 bytes of the file. The fork keeps them for that name and does not answer each one. |
| | 156 `RC_LARGEFILEEND` | The name, the rest of the line. The fork stores the joined file in the folder from the last list and answers id 67 `Uploaded large file ` and the name. |
| Get | 68 `PLO_LARGEFILESTART` | The name, after a 92 for it, the rest of the line. |
| | 84 `PLO_LARGEFILESIZE` | `GINT5` total size (`74 20 20 23 26 70` = 50000). 2.3 and later servers; an older one leaves it out. |
| | 102 inside 100, repeated | The file block above, at most 32000 bytes each. |
| | 69 `PLO_LARGEFILEEND` | The name, the rest of the line. |

The fork sends any file over 32000 bytes as a large file, so a get of a
file between 32000 bytes and 60 KiB comes in pieces too. `seihai-rc` puts a
file up to 60 KiB in one block, as before, and a bigger one as a large
file. A get joins the pieces and fails if they come to more or fewer bytes
than id 84 said. A file over 64 MiB is refused either way, before anything
is sent or kept.

Paths are relative, `/`-separated. `..`, a leading `/`, a backslash and a
colon are refused. A trailing `/` is kept. A path longer than 223 bytes is
refused: a put name is one `GCHAR`, which cannot say more.

## Server options, folder config, server flags (stage 4)

Captured from the 2004 RC, get and set (lab 2026-11-13). Ids 51 to 54
meant account actions in RC 1.33: rc133 (`GSERV014`) still sends 53 for
an account add, with GSTRING account, GSTRING password and a props block.
These are the meanings from the 2002/05/02 RC on. Each set is answered
with an RC chat line (`<rc> has updated the server options.`, `<rc>
updated the folder config.`, `<rc> has updated the server flags.`).

The fork at `a3b6ac23` sent 76 and 77 with a line quoted only for a comma
or a quote. The real RC splits an unquoted item at its spaces too, so one
open, edit and set from it took `serveroptions.txt` from 572 to 2476 lines
(seihai-gserver-v2 #63 fixes that). `seihai-rc` reads an unquoted item up
to the next comma, so it kept the lines whole either way, and it sends
every line quoted.

| What | Get | Reply | Set | Payload |
|---|---|---|---|---|
| Server options | 51 | 76 | 52 | Tokenized text, one item per line, as for the comments in id 63. The get has no payload. |
| Folder config | 53 | 77 | 54 | The same. |
| Server flags | 68 | 61 | 69 | `GSHORT` count, then each flag (`name=value`) as a `GSTRING`. |

`seihai-rc server options|folders|flags --set-file PATH` reads the value,
sends the set, reads it again, and fails if what came back differs from
what was sent (blank lines and trailing spaces aside; flags in any order).
