# Lab note: RC player admin packets, 2026-11-04

Client: a real RC, `RemoteControl.exe` from the Graal Reborn 2.22.0 pack
(login token `GSERV024`), under Wine. `seihai-rc` was not the client in
this run; its bytes are checked against these in `tests/fake_servers.rs`.

Server: the `seihai-gserver-v2` fork (`seihai` at `f585ca6`) with
`seihai-listserver` (`main`), all on loopback. The target was a real 2.22
player (`test1`, player id 3) in the game. The capture (pcap) is private
and kept out of this repo; it holds only lab passwords.

Each layout below was seen once on the wire, with the values shown. Wire
types are as in `docs/PROTOCOL.md`; GINT5 is five GCHARs, high 7 bits
first. "Tokenized" means one item per line, each quoted, joined with
commas (`"lab comment one","line two"`).

What the RC did, in order: opened the player list; Edit Attributes, set
Gralats 0 to 7; Edit Rights, ticked warp-to-xy and admin message and added
the folder line `rw levels/*`; Edit Comments with two lines; Edit Ban,
cancel; reopened rights, comments and attributes to read them back; Edit
Ban with banned on and the reason `lab ban reason`; reopened the ban twice.

## Comments

| Dir | Id | Payload | Seen |
|---|---|---|---|
| RC to server | 85 | the account as the rest of the line, no length byte | `test1` |
| server to RC | 63 | GSTRING account, then the comments tokenized | before: `%test1`; after: `%test1"lab comment one","line two"` |
| RC to server | 86 | same as 63 | `%test1"lab comment one","line two"` |

Each line of the text box is one item. The readback matched.

## Account ban

| Dir | Id | Payload | Seen |
|---|---|---|---|
| RC to server | 87 | the account as the rest of the line | `test1` |
| server to RC | 64 | GSTRING account, GCHAR banned (0 or 1), then the reason | `%test1 ` (not banned, no reason) |
| RC to server | 88 | GSTRING account, GCHAR banned, then the reason tokenized | `%test1!"lab ban reason"` |

The fork saved the ban to the account file. Then, with the player online,
it went to full CPU and sent nothing more to anyone, and both later id 87
messages went unanswered. Seen once. Ban off and a ban of an offline
account were not run, so a reply with a reason in it was not seen.

## Rights

| Dir | Id | Payload | Seen |
|---|---|---|---|
| RC to server | 83 | the account as the rest of the line | `test1` |
| server to RC | 62 | GSTRING account, GINT5 rights, GSTRING IP range, GSHORT length, then the folder-rights text | rights 0, `0.0.0.0`, empty folders |
| RC to server | 84 | same as 62 | rights 513, `0.0.0.0`, length 13, `"rw levels/*"` |

Rights bits follow the dialog's checkbox order from bit 0: warp-to-xy is
bit 0, admin message bit 9 (together 513). The RC sends the folder lines
tokenized. The fork returned them untokenized (`rw levels/*`, 11 bytes),
and the real RC then showed two lines, `rw` and `levels/*`. That is a fork
bug: a server should return the text tokenized, the way it came in.

## Player props

| Dir | Id | Payload |
|---|---|---|
| RC to server | 73 | GSHORT player id |
| server to RC | 72 | GSHORT player id, GSTRING account, GSTRING world (`main`), GCHAR props length N, N bytes of props, then the tail |
| RC to server | 76 | GSTRING account (no player id), GSTRING world, GCHAR props length N, N bytes of props, the tail, then one extra 0 byte |

Tail, both ways: GSHORT 0, GSHORT 0, GCHAR weapon count, then a GSTRING
per weapon (`bomb`, `bow`, `-gr_movement`). Still to check: which GSHORT
counts flags and which opened chests (both were 0), and the extra byte on
76. The server sent props 0 to 36 (144 bytes); the RC sent back the same
ids without 0, 30, 34 and 36 and with 21, 22 and 23 (one byte each, 0).
Changing Gralats changed only prop 3, and the readback showed 7.
