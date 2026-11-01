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

## RC list from the listserver

1. Plain zlib frame: id 5 + GCHAR key + 8-byte token.
2. Gen-5 frame: id 1 + GSTRING account + GSTRING password.

Answer (gen-5 frames): id 0 server list (count, then per server 8 GSTRINGs:
type tag + name, language, description, URL, version, players, host, port),
then id 2 status (`\r` = new line), id 3 site URL, id 5 upgrade URL. Names carry
a type tag (`P `, `H `, `3 `, `U `; none for classic). A refused login gets id 4
with the reason and the link closes.

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
| Class delete | 119 is not sent | 188 is not parsed |
| Weapon list | 115 `NC_WEAPONLISTGET`: no payload | 167: repeated GSTRING names |
| Weapon get | 116 `NC_WEAPONGET`: the weapon name as the rest of the line | 33: GSTRING name, GSTRING image, GSHORT length, then that many script bytes |
| Weapon set | 117: GSTRING name, GSTRING image, then the script | no reply id is named |
| Weapon delete | 118: the weapon name as the rest of the line | no reply id is named |

Still needs a lab check, and so is not turned into fields:

- Id 158 with a property other than 50, 51 or 52. The whole payload stays raw. Whether a missing property clears a value already stored also still needs a lab check; this client leaves the stored value.
- Id 159 with bytes after the GINT3.
- Id 188, and client id 119. The notes still disagree on a bare class name versus a length-prefixed string. `nc class delete` prints that and sends nothing.
- Ids 112, 116 and 118. The notes write `str` where a length byte was seen, and they do not write it for these three, so this client sends the name as the rest of the line. A lab check should confirm there is no length byte.
- Id 33 when the GSHORT length runs past the message. A GINT3 length for a large script still needs a lab check and is not tried. A script byte that is a line feed still needs a lab check: the frame reader stops at a line feed.
- Each id 167 replaces the stored weapon list. Whether a second one is a full list or an addition still needs a lab check. The notes describe the packet as the list, so this client treats each one as the whole list.
- Attribute text, class names and scripts are the rest of the one message that carried them. A line feed inside that text still needs a lab check.

## File link (list, get, put)

Ids are the file-browser ids named under stage 4 in `docs/NEXT.md`. This
repo does not describe their payloads. The layout below is the smallest one
that fits the codec already in this crate: `GSTRING` for a path or a name,
`GSHORT` for a count, `GINT3` for a byte count, and a raw block so the file
bytes may contain `\n`.

A raw block is the shape `split_messages` already uses for id 100: `GCHAR`
id, `GINT3` byte count, `\n`, then that many bytes (the next message, id
included, line feeds allowed). Inbound files use id 100, so `Link::recv`
keeps them intact. Outbound files use id 50, the outbound raw id named in
the NPC-Control section above. `split_messages` does not treat id 50 as
raw; the sender builds the block itself.

Every payload field below still needs a lab check against the fork. Until
that run, do not treat this layout as what a gserver sends.

| | Id | Payload, lab-unconfirmed |
|---|---:|---|
| List | 90 `RC_FILEBROWSER_CD` | `GSTRING` path. `""` is the world root. Id 89 `START` is not sent. |
| Names | 65 `RC_FILEBROWSER_DIRLIST` | `GSHORT` count, then that many `GSTRING` names. No size, no directory flag. |
| Get | 92 `RC_FILEBROWSER_DOWN` | `GSTRING` path. |
| File | 100 raw, then 66 `RC_FILEBROWSER_DIR` | `GSTRING` name, `GINT3` size, then that many bytes. |
| Put | 50 raw, then 93 `RC_FILEBROWSER_UP` | same name, `GINT3` size, and bytes as the file row. |
| Refusal | 67 `RC_FILEBROWSER_MESSAGE` | the text. |

Not sent, layouts unknown: 89 `START`, 91 `END`, 96 move, 97 delete, 98
rename, 155/156 large file, 160 folder delete. A file longer than 60 KiB
is refused. One frame's length is a `u16`, and those large-file ids are
how a bigger file would have to travel once a lab run shows their layout.

Paths are relative, `/`-separated. `..`, a leading `/`, a backslash and a
colon are refused. A `GSTRING` holds at most 223 bytes, so a longer path
is refused rather than cut.
