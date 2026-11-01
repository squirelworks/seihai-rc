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

## NPC Control (stage 5, login only)

A second socket, not the RC game-server link. Layouts follow the behaviour
in Graal_analysis `protocol/npc-control.md` (link only; that note is not
copied here). `seihai-rc nc login` speaks this. Finding the address from the
game server (ids 94 and 79) and the NPC / class / weapon commands are not
in this slice.

| | |
|---|---|
| Login type | 3 |
| Token | `NCL11012` (eight bytes, no key byte) |
| Body | `GCHAR 3` + token + `GSTRING` account + `GSTRING` password + `\n` |
| Frames | `u16be` length + zlib body, both ways, including the login and every frame after it. No compression-type byte and no cipher. |

Nothing is sent after the password. A loopback capture of a real NPC-server
may still show an extra byte; until that run, this client does not add one.

The staff client treats the socket as connected once the login frame is
written. This client does the same when the link stays open. Id 16
(`DISCMESSAGE`, text) during that wait is a refusal and the command fails.
Later id 16 lines are printed as `[disconnected]`. Every other id is printed
as `[nc] id N (L bytes)` and is not parsed yet. Large raw uploads (id 50
out, id 100 in) are not handled.
