# seihai-rc live run against the fixed fork, 2026-11-13 (#25)

Loopback only: a Debian lab VM, with every program inside a private network
namespace that has only `lo`. The lab is the one in
[`2026-11-13-archived-rc-captures.md`](2026-11-13-archived-rc-captures.md): the
same servers, accounts and test files, and the same decoding relay in front of
every port. Its decoded log is `run6`; it is not committed.

| Part | Commit / build |
|---|---|
| seihai-rc | `claude/project-thread-rvkilj` at `f592fa3` (PR #45), `cargo build --release` |
| seihai-gserver-v2 | `claude/project-thread-rvkilj` at `a1859ba` (PR #63), Linux, `-DPACKET_LOGGING=ON` |
| seihai-listserver | `main` at `ae213fc` |

Commands used `--server 127.0.0.1:14802 --account labrc` (game server) or
`--discover 127.0.0.1:14804 --account labrc` (NPC Control), with the lab
password in `SEIHAI_RC_PASSWORD`.

## Results

| Command | Result | Evidence |
|---|---|---|
| `servers --listserver 127.0.0.1:14922` (gen5) | **pass** | `[H] My Server  127.0.0.1:14802 …` and `[H] Seihai NPC demo  127.0.0.1:14804 …`, then the status text |
| `servers … --rc classic` | **fail** (each server printed twice) | See below |
| `accounts get labban` (`BANLENGTH 3600` in the file) | **pass** | `banned: yes`, `ban length: 3600`, `ban reason: lab timed ban` |
| `accounts reset labprops --yes` | **pass** | `reset labprops` after the fork's 74 line |
| `players props labflags` | **pass** | `gralats 7`, `flags: 2` (`labflag=1`, `quest.done=yes`), `opened chests: 2` (`30,20 onlinestartlocal.nw`, `10,12 onlinestartlocal.nw`), `weapons: bomb, bow, -gr_movement` |
| `players props labflags --set gralats=9` | **pass** | Read back `gralats 9` with both flags and both chests; the account file still has the two `FLAG` and two `CHEST` lines |
| `server options --set-file` | **pass** | The file was the `server options` output plus `rclive = 1`. The command printed `572 line(s) before, 572 after`; `serveroptions.txt` ends with `rclive = 1` and no line is split. (The file is 572 lines because my capture of the get dropped the header comment; the round trip itself is exact.) |
| `server folders --set-file` | **pass** | The same with `file    rclive/*.txt`; `foldersconfig.txt` ends with that line, whole |
| `files put` of a 75400-byte file | **fail** (stored 75399 bytes) | See below |
| `files rmdir world/labdir/ --yes` (empty folder) | **fail** (folder removed, command exits 1) | See below |
| `nc npc add rcnpc 1010 OBJECT npcdemo_start.nw 20 25` | **pass** | `added 1010` (111 → 158) |
| `nc npc flags 1011 --set-file` (2 lines, one with a space) | **pass** | `flags: 0 before, 2 after` |
| `nc npc set 1011 --script-file` (4 lines, quotes and a comma) | **pass** | `sent NPC script for 1011` |
| `nc npc get 1011 --script` | **pass** | Output is byte-identical to the script file (`cmp`) |
| `nc npc reset` | **pass** | `reset 1010` / `reset 1011` (waited for 29) |
| `nc npc warp … 12.5 14` | **pass** | `sent warp for …` |
| `nc npc delete` | **pass** | `deleted 1010` / `deleted 1011` (one 159 each) |
| `nc weapon set rcweapon --image wbomb1.png --script-file` (340 lines, 18020 bytes) | **pass** | `sent weapon rcweapon`; 117 is 18041 bytes |
| `nc weapon get rcweapon` | **pass** | The fork's 33 carries the length as `ac 84` = (140 << 7) + 100 = 18020; the printed script is byte-identical to the file (`cmp`) |

The first pass of the NC sequence (NPC 1010) ran with relative `--set-file` and
`--script-file` paths, which do not resolve inside the namespace. Add, reset,
warp and delete passed there; the file-based commands were then re-run with
NPC 1011 and absolute paths. That is a lab mistake, not a client fault.

## The three failures

### `servers --rc classic`: the list comes twice

seihai-rc sends the classic handshake and the login as **two** zlib frames in
one write:

```
000c 789c53284ae6020002aa0100   -> " rc\n"        (id 0, "rc")
0013 789c5354cd494c2a4a069105e55c0021260490 -> "!%labrc%labpw\n" (id 1)
```

The real rc141 (GSERV023, 2002/05/02) sends both in **one** frame:
`0017 78da53284ae65254cd494c2a4a069105e55c0031c2058f` = `" rc\n!%labrc%labpw\n"`.

The lab listserver answers a frame that holds only id 0 as a whole login, for
account `player`: it sends 0 (list) and 2 `Welcome to RC Lab, player.`. Then it
answers the id 1 frame again with 0 and 2 `Welcome to RC Lab, labrc.`.
seihai-rc prints both lists, so four servers appear. To match the real RC,
send id 0 and id 1 in one frame. (The listserver's eager reply to a lone id 0
is what 1.41 rev 1 needs, so it should stay.)

### `files put` over 60 KiB: the last byte is lost

seihai-rc's raw block counts exactly the 93 message:

```
50 count 32012 = 93 + GSTRING name (put70k.txt) + 32000 bytes
50 count 32012
50 count 11412 = 93 + name + 11400 bytes
```

The real RC (RC3, same file kind) counts **one more byte**: a line feed after
the message, inside the raw block (`50 count 32015` for a 93 of 32014 bytes).
The fixed fork (PR #63) drops that trailing line feed from each block. With
seihai-rc's count there is no extra line feed, so the fork drops the file's own
last `\n` instead (the file ends in one). Stored: 75399 bytes, the same as the
source except the final `\n`. The first two pieces end mid-line and are stored
whole. A small put of a file ending in `\n` would lose that byte the same way.
To match the real RC, the raw count should be the message length + 1, and the
block should end with the message's `\n`, as PROTOCOL.md already says for
inbound id 100.

### `files rmdir`: removed, but reported as still there

```
C>S 90  world/labdir/             (list first)
S>C 67  Folder changed to world/labdir/
S>C 66  GSTRING world/labdir/, no files
C>S 160 world/labdir/
S>C 67  Folder world/labdir/ has been removed.
S>C 65  folder list (the account still has the right rw world/labdir/*)
S>C 67  Welcome to the File Browser.
S>C 66  GSTRING "/", no files
C>S 90  world/labdir/             (seihai-rc's check)
S>C 67  Folder changed to world/labdir/
S>C 66  GSTRING world/labdir/, no files
```

The folder is gone from disk. The fork still answers a 90 for a folder that no
longer exists, but is covered by a folder right, with `Folder changed to …` and
an empty 66. seihai-rc treats that answer as "still lists". The 67 `… has been
removed.` line, or the 65/66 that follow it, is the reliable check against this
fork. (Whether the fork should refuse a 90 for a missing folder is a separate,
minor point.)

## Not changed

No Rust code was changed for this run.
