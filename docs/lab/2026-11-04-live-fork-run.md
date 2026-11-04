# Lab note: seihai-rc live against the fork, 2026-11-04

Client: `seihai-rc` itself (`main` at `8d280c8`), logged in as the lab
staff account `labrc` (IP range `127.0.0.1`).

Server: the `seihai-gserver-v2` fork (`seihai` at `1d7843a`, which has the
fixes for the ban hang and for folder rights quoting) with
`seihai-listserver` (`main`, open mode), all on loopback. The target was
`test1`, a plain account (rights 0, no folder rights), logged in as player
id 3 by a scratch 2.22 player client that prints every message it gets.
Only lab accounts and lab passwords were used. Nothing was captured to a
file.

Every `players` command below exited 0 unless the note says otherwise.

## RC list

`seihai-rc list` against the listserver printed
`[H] My Server 127.0.0.1:14802`.

## Comments (85/86 → 63)

`players comments test1 --set-file` with three lines: `lab comment one`,
`line two, with a comma` and `says "hi"`. The readback printed the same
three lines. The account file stored them tokenized:
`COMMENTS "lab comment one","line two, with a comma","says ""hi"""`.

## Rights (83/84 → 62)

`players rights test1 --set 513 --ip-range 127.0.0.1 --set-file` with two
folder lines, `rw levels/*` and `r world/images/*`. The readback printed rights
513 (warp-to-xy, admin message), IP range `127.0.0.1` and both folder
lines whole. With the fork fix the folder text now comes back tokenized,
so a line with a space is no longer split. Rights were then set back to
0, IP range `*.*.*.*` and no folders.

## Account ban (87/88 → 64)

- Offline: `--set on --reason ...` then `--set off`. Both readbacks
  matched.
- Online, with `test1` in the game: `--set on --reason "lab ban, online"`.
  The player got id 16, `labrc has banned you.  Reason: lab ban, online`,
  and the link closed. The server went on answering: the readback came
  back, and so did every later command. Before the fork fix it stopped
  answering here. The ban was then set off.

## Props (73 → 72)

`players props test1` decoded 25 props. Both tail GSHORTs were 0 and the
weapons were `bomb`, `bow` and `-gr_movement`.

## Player actions

| Command | Id sent | What the player got | Server log |
|---|---|---|---|
| `players pm test1 'hello, "test1"'` | 28 | id 37: `"","Private message:","hello, ""test1"""` | |
| `players say "Restart in 5"` | 63 | id 35: `Admin labrc:`, `0xA7`, `Restart in 5` | |
| `players admin-pm test1 "Please stop"` | 64 | id 35: `Admin labrc:`, `0xA7`, `Please stop` | |
| `players warp test1 onlinestartlocal.nw 30.5 12` | 82 | nothing | `labrc has warped test1 to onlinestartlocal.nw (30.50, 12.00)` |
| `players kick test1` | 61 | id 16: `One of the server administrators, labrc, has disconnected you.` | `labrc disconnected test1.` |

`kick` printed `disconnected 3 test1`: the 56 `DELPLAYER` for id 3 came
back to the RC. 61 needs no reason text. For 82 the server read x 30.5 and
y 12 from the two GCHARs in half tiles and the level after them, as sent.
The player client got no warp message from the fork after it; it was
already in that level, so whether the fork sends one on a move to another
level was not checked.

The RC got nothing back after 28, 63, 64 or 82 within the settle time.
