# Archived RC builds on the wire against seihai-gserver-v2, 2026-11-13 (#25)

Loopback only. Everything ran in a Debian lab VM, inside a private network
namespace that has only `lo` (no route out). The real RCs ran under Wine
(32-bit prefix, `explorer /desktop=` virtual desktop) on a headless Xvfb display.
The Windows 10 lab VM was not used, because another lab held its desktop.

The full message log, with exact hex, is in
[`2026-11-13-archived-rc-captures-hex.md`](2026-11-13-archived-rc-captures-hex.md).
That file has one bullet per message, grouped under the UI step that caused it.
Messages up to 800 bytes are given in full. Longer ones (scripts, server
options, file pieces) are given as their first 48 bytes, their length and a
SHA-256 prefix; their content is described below so a test can rebuild it.

## Setup

| Part | Commit / build |
|---|---|
| seihai-gserver-v2 | `seihai` at `a3b6ac23`, Linux, `-DPACKET_LOGGING=ON`. A rebuild at `e3da7ace` finished during the run but was not used. The four commits in between touch only weapon CRCs, minify and Graal 3D code, not the RC or NC paths. |
| seihai-listserver | `main` at `ae213fc` |
| seihai-rc | `main` at `98ae031`; not run, only read |
| Relay | A logging TCP relay in front of each port. It decodes the login frame, then `u16be` + zlib (login types 0, 1, 3) or `u16be` + compression byte + gen-5 (types 5, 6, 7), splits the messages, and unwraps raw blocks (C>S 50, S>C 100). Every message is logged as GCHAR id + payload, hex + ASCII, with a UTC time, and UI steps are logged as markers. |

Servers: `default` ("My Server", 14802) and `npcdemo` ("Seihai NPC demo",
14804, `serverside = true`, built-in NPC server). Both use `localhost = true` and
`localhostlist = true` and register with the lab listserver. The gservers and
the listserver listen on `127.0.0.2`; the relay listens on `127.0.0.1` and
connects from `127.0.0.1`, so `IPRANGE 127.0.0.1` still matches.

Accounts:
- `labrc`, `labrc2`: staff, `IPRANGE 127.0.0.1`.
- `labflags`: two `FLAG` lines and two `CHEST` lines (`30:20:onlinestartlocal.nw`, `10:12:onlinestartlocal.nw`).
- `labban`, `labprops`: plain accounts.

Test files and weapons:
- `world/labbig.txt` is 50000 bytes.
- Folders `world/labdir` (empty) and `world/labgone` (one file).
- NPC-server weapons with generated scripts:
  - `lab300`: 9 lines `// lab300 line NN padding text`, 279 bytes.
  - `lab9000`: 200 lines `// lab9000 line NNN padding padding padding`, 8800 bytes.
  - `lab20000`: 400 lines `// lab20000 line NNN padding padding padding padding`, 21200 bytes.
  - `labbreaks`: a GS2 script with line breaks.

Server config files were backed up before the run and restored after each set
(see the fork bugs). The server password is the lab's `labpw`, not a real one.

The VM clock was about 10 minutes slow at first and jumped forward to real time
at 05:44. Times inside one step are consistent.

## Clients

| Build | Token | NC token | Result |
|---|---|---|---|
| 2004 RC (`RemoteControl.exe`, "Graal Remote Control 2004/01/17") | `GSERV024` | `NCL11012` | **Priority 1 done**: options, folders and flags get+set; file delete, move and large get; timed ban; props with chests; all NC. Its rename crashed under Wine (access violation in the RC, nothing sent). It has no upload menu and no folder menu. |
| RC2_2.05 | `GSERV025` | `NCL21075` | **Not run.** Its GTK 2 runtime crashes in `libgdk-win32` when OK is pressed, under this Wine, with and without a virtual desktop. Its text also draws as boxes (pango modules). |
| RemoteControl3 ("Remote Control 2015/10/31") | `GSERV025` | `NCL21075` | Used in place of RC2_2.05: same tokens. Rename, folder delete, large and small put, NC with NCL21075. Text draws as boxes (the same pango issue); I drove it by layout, and every step is confirmed by the wire log. |
| rc141 ("GServer2 Remote Control 2002/05/02") | `GSERV023` | `NCL11012` | Logged in through the listserver; every toolbar window opened once; account list and get. |
| rc201 ("2002/01/04") | `GSERV022` | – | Direct to the gserver port; logged in; every window opened once. |
| rc8212001 ("2001/08/20") | `GSERV022` | – | As rc201. |
| rc62101 ("2001/01/27") | `GSERV021` | – | Logged in; every window opened once; account list and get. |
| rc1025 ("2000/10/25") | `GSERV020` | – | Second pass (below): logged in with one start-dialog server option; every window opened once; account list. |
| rc139rev2 ("2000/8/19") | `GSERV019` | – | Second pass: logged in with all four start-dialog server options; every window opened once; account list. |
| rc133 ("GServer2 Remote Control (C) Stefan Knorr") | `GSERV014` | – | Second pass: logged in; its account add was sent and misread by the fork (below). |

## Logins

| Dir | Id | Layout | Seen (hex) |
|---|---:|---|---|
| C>S listserver, 2004 RC | 5 | Plain zlib frame: GCHAR key, then the two bytes `rc`. **Not** an 8-byte token. Gen-5 after it. | `25 4d 72 63` |
| C>S listserver, rc141 | 0 | Plain zlib frame: GCHAR 0, then `rc`. Plain zlib both ways after it (no gen-5). The list comes back without type tags. | `20 72 63` |
| C>S listserver, RC3 | 7 | Plain zlib frame: GCHAR key, `G3D30123`, then the list name `rc2`. The same shape as a game client's id 7. Gen-5 after it. | `27 91 47 33 44 33 30 31 32 33 72 63 32` |
| C>S listserver, all | 1 | GSTRING account, GSTRING password | `21 25 6c61627263 25 6c61627077` |
| C>S gserver, 2004 RC | 6 | GCHAR key, `GSERV024`, GSTRING account, GSTRING password; nothing after it | `26 4e 4753455256303234 25 …` |
| C>S gserver, RC3 | 6 | As above with `GSERV025`, then the **identity** as the rest of the line: `win,<32 hex>,"",<32 hex>`. It is tokenized: platform, a 32-hex id, an empty item, a 32-hex id. The fork accepted it. (In the hex file both ids are replaced with `0123456789abcdef0123456789abcdef`.) | see hex file, 94 bytes |
| C>S gserver, rc141 / rc201 / rc8212001 / rc62101 | 1 | Type 1, plain zlib both ways: `GSERV023`/`GSERV022`/`GSERV021`, GSTRING account, GSTRING password. The same frame carries the nickname 2 right after it. | `21 47534552563032 33 25 …` |
| C>S nickname | 2 | Prop 0 GSTRING. The 2004 RC, rc141 and RC3 add prop 53 (list status) `21`. rc201, rc8212001 and rc62101 send `*labrc - away` with no prop 53. | `22 20 26 2a6c61627263 55 21` |
| C>S NC, 2004 RC / RC3 | 3 | `NCL11012` / `NCL21075`, GSTRING account, GSTRING password, nothing after it. `u16be` + zlib both ways. | `23 4e434c3131303132 25 … 25 …` |

After an NC login the fork sends 25 and then one 158 per NPC; no other burst.

**Fork timing quirk:** on `npcdemo`, the first frame the fork sent to a type-6
RC was id 42 (world time), in plain `u16be` + zlib framing **with no
compression byte and no gen-5**. It was sent before the login reply, and gen-5
started on the next frame (`789cf3cacd0a69e4020007930201` = `4a 6d 6a 54 81`).
The real RC coped with it. A strict gen-5 reader will not, unless it treats a
frame whose first byte is `0x78` as plain zlib and does not advance the stream.

RC3 also sends, right after login: 154 `GraalEngine,irc,login,-`, 152
`GraalEngine,lister,bantypes`, 152 `GraalEngine,pmguilds,""`, 152
`GraalEngine,pmservers,""` and 154 `GraalEngine,lister,options,…`. The fork
answers with 55 for an `irc:#graal` entry and 82
(`GraalEngine,lister,bantypes,…`).

Keepalives seen: RC3 sends **C>S 162 with an empty payload** on the RC socket
and **C>S 103 with an empty payload** on the NC socket, roughly every 30 s. On
the NC socket 103 is also the NPC-get id (with a GINT3). The fork sent nothing
back to the empty 103.

## Server options, folder config, server flags

| Dir | Id | Layout | Seen |
|---|---:|---|---|
| C>S | 68 | no payload | `64` |
| S>C | 61 | GSHORT count, then a GSTRING `name=value` per flag | `5d 20 22 2f labserverflag=1 36 server.lab=hello world` |
| C>S | 69 | the same shape as 61 | `65 20 23 2f labserverflag=2 36 server.lab=hello world 2a labnew=a b` |
| S>C | 74 | reply to 69: `labrc has updated the server flags.` | |
| C>S | 53 | no payload | `55` |
| S>C | 77 | the file's lines, tokenized, rest of the line | 752 bytes |
| C>S | 54 | the edited lines, tokenized (items with spaces quoted) | 682 bytes |
| S>C | 74 | reply to 54: `labrc updated the folder config.` | |
| C>S | 51 | no payload | `53` |
| S>C | 76 | `serveroptions.txt`, tokenized | 24410 bytes |
| C>S | 52 | the edited options, tokenized | 24251 bytes; the RC took about 8 s to send it |
| S>C | 74 | reply to 52: `labrc has updated the server options.` | |

The flags set kept every flag and added the new one. The order of the reply is
the fork's own.

**Fork bug (folder config and server options are destroyed by a round trip):**
the fork tokenizes 77 and 76 quoting a line only when it has a comma or a quote.
`head    heads/*` goes out unquoted. Graal's tokenizer splits an unquoted item at
spaces too, so the real RC (GSERV022 to 025) shows `head` and `heads/*` on two
lines, and its 54/52 sends them back as two items. After one open, edit and
send with the 2004 RC:
- `foldersconfig.txt` had one word per line.
- `serveroptions.txt` grew from 572 to 2476 lines (`name`, `=`, `My`, `Server` each on a line).

The server keeps running on the broken files until the next load. Lines the
RC itself added (with spaces) came back quoted and were stored whole. The same
missing quote breaks the NC text replies: 157, 160, 161 and 162 (below).

## File browser

| Dir | Id | Layout | Seen (real RC) |
|---|---:|---|---|
| C>S | 89 | no payload, when the browser opens | `79` |
| S>C | 65 | the folder rights, tokenized | |
| C>S | 96 move | GSTRING target folder, then the file name as the rest of the line (2004 RC, by drag and drop) | `80 2a documents/ labmove.txt` → 67 `Moved file world/labmove.txt to documents/labmove.txt`. **Confirms** PROTOCOL.md. |
| C>S | 97 delete | the file name as the rest of the line, no confirm dialog (2004 RC) | `81 labdel.txt` → 67 `Deleted file labdel.txt` |
| C>S | 98 rename | GSTRING old name, GSTRING new name (RC3) | `82 2a labren.txt 2b labren2.txt` → 67 `Renamed file labren.txt to labren2.txt`. **Confirms** PROTOCOL.md. |
| C>S | 160 folder delete | the folder path with a trailing `/` as the rest of the line (RC3, folder menu) | `c0 world/labgone/`. **Confirms** the shape. |
| C>S | 92 get | the name as the rest of the line | `7c labbig.txt` |
| S>C | 68 | the name, rest of the line | `64 labbig.txt` |
| S>C | 84 | GINT5 total size | `74 20 20 23 26 70` = 50000 |
| S>C | 100 raw → 102 | 100 with GINT3 count 32018, then 102: GINT5 mtime, GCHAR name length, the name, 32000 bytes. Then 100 with count 18018 and 102 with the last 18000 bytes. | |
| S>C | 69 | the name, rest of the line | `65 labbig.txt` |
| C>S | 155 | the name, rest of the line (RC3) | `bb labup70k.txt` |
| C>S | 50 raw → 93 | 50 with GINT3 count 32015, then 93: GSTRING name, then at most 32000 bytes. 81200 bytes went as 32000 + 32000 + 17200. | `52 21 9a 2f` / `7d 2c labup70k.txt …` |
| C>S | 156 | the name, rest of the line | `bc labup70k.txt` |
| S>C | 67 | `Uploaded large file labup70k.txt` | |
| C>S | 50 → 93 small put | 50 count 57; 93 GSTRING name + 42 bytes | → 67 `Uploaded file labsmall.txt` |

After each change the real RC sends 90 for the current folder, so the fork
answers with 67 `Folder changed to …` and a new 66.

The 2004 RC's download landed whole: the downloaded `labbig.txt` matched the
server's file (50000 bytes, mtime kept).

**Fork bug (puts gain a line feed):** the fork stores the line feed that ends
each raw block as part of the file. The 81200-byte large put was stored as
81203 bytes (a `\n` after each piece, at 32000, 64001 and the end). The 42-byte
small put was stored as 43 bytes.

**Fork bug (folder delete disconnects the RC):**
- On the non-empty `world/labgone/`, the fork sent 16 `filesystem error: cannot remove: Directory not empty [world/labgone/]` and closed the RC link.
- On the empty `world/labdir/`, it deleted the folder, then sent 16 `filesystem error: directory iterator cannot open directory: No such file or directory [world/labdir/]` and closed the link.

The other RCs got 56 for it. A refusal or a result should be a 67 line.

The fork pushes a fresh 65 to an online RC when another RC changes its folder
rights with 84.

## Accounts and ban

| Dir | Id | Layout | Seen |
|---|---:|---|---|
| C>S | 72 | GSTRING pattern, GSTRING conditions | `68 24 lab% 20` (2004 RC, rc141, rc62101) |
| S>C | 70 | a GSTRING per account | |
| C>S | 77 | the account as the rest of the line | `6d labban` |
| S>C | 73 | GSTRING account, GSTRING password (empty), GSTRING e-mail, GCHAR banned, GCHAR load only, GCHAR admin level, GSTRING admin worlds, **GSTRING ban length**, GSTRING ban reason (tokenized) | with no length: `… 24 main 20 2f "lab timed ban"`; with `BANLENGTH 3600`: `… 24 main 24 3600 2f "lab timed ban"` |
| C>S | 78 | as 73 **without** the ban length | `6e 26 labban 20 20 21 20 20 24 main 2f "lab timed ban"` |
| S>C | 74 | reply to 78: `labrc has modified the account: labban` | |

**Resolved:** the "one byte, 0x20 in every capture" before the reason in 73 is
a **GSTRING ban length**, empty in those captures. With `BANLENGTH 3600` in the
account file, the fork sent `24 33 36 30 30` and the 2004 RC showed 3600 in Ban
time. The 2004 RC's 78 never carries the ban time, even with 3600 shown in the
dialog. The fork keeps an existing `BANLENGTH` on a 78. The fork also stores the
tokenized reason with its quotes (`BANREASON "lab timed ban"`) and sends it back
the same way; minor.

rc141 and rc62101 (GSERV021, 2001) use the same 72/70 and 77/73.

## Props with flags and opened chests

| Dir | Id | Layout | Seen |
|---|---:|---|---|
| C>S | 74 | GSTRING account, then an empty GSTRING | `6a 28 labflags 20` |
| S>C | 72 | … props …, GSHORT flag count, a GSTRING per flag, **GSHORT chest count, then each chest as a GSTRING whose body is GCHAR x, GCHAR y, then the level name**, GCHAR weapon count, a GSTRING per weapon | flags `20 22` + `2e quest.done=yes` + `29 labflag=1`; chests `20 22` + `35 3e 34 onlinestartlocal.nw` (x 30, y 20) + `35 2a 2c onlinestartlocal.nw` (x 10, y 12); weapons `23` + `24 bomb` `23 bow` `2c -gr_movement` |
| C>S | 76 | GSTRING account, GSTRING world, props, the same tail as 72, then GCHAR 0 | 238 bytes, gralats 0 → 7; the chest entries were sent back unchanged |
| S>C | 74 | reply to 76: `labrc set the attributes of player labflags` | |

After the 76 the account file still had both `FLAG` and both `CHEST` lines, plus
`GRALATS 7`. That is fork #15's "flags kept" half, now shown with a real RC.

## NPC Control

The 2004 RC (NCL11012) and RC3 (NCL21075) sent the same ids, except for the
weapon-get reply.

| Dir | Id | Layout | Seen |
|---|---:|---|---|
| S>C | 158 | GINT3 id, then 50 GSTRING name, 51 GSTRING type, 52 GSTRING level | `be 20 6e 30 52 2b Control-NPC 53 27 CONTROL 54 20` (id 10000, empty level); `be 20 27 89 52 26 labnpc 53 26 OBJECT 54 30 npcdemo_start.nw` (id 1001) |
| C>S | 111 NPC add | tokenized `name,id,type,scripter,level,x,y` | `8f labnpc,1001,OBJECT,labrc,npcdemo_start.nw,20,25` → 158 + 74 `NPC labnpc added by labrc` |
| C>S | 103 attributes | GINT3 id | `87 20 27 89` → 157 (no id; the variables dump, lines joined by commas) |
| C>S | 106 script get | GINT3 id | → 160 GINT3 id + the script |
| C>S | 109 script set | GINT3 id, then **the script tokenized**: one item per line, quoted when needed, the trailing empty line dropped | `8d 20 27 89 "// lab npc script","if (created) {","  this.lab = 1;",}` → 74 `NPC script of labnpc updated by labrc` |
| C>S | 108 flags get | GINT3 id | → 161 GINT3 id + flags |
| C>S | 110 flags set | GINT3 id, then the flags tokenized | `8e 20 27 89 labnpcflag=1,"other.flag=a b"` → 74 `flag added:\t…` per flag |
| C>S | 105 reset | GINT3 id | → **29** GINT3 id, and 74 `NPC script of labnpc reset by labrc` |
| C>S | 107 warp | GINT3 id, GCHAR x×2, GCHAR y×2, then the level as the rest of the line | `8b 20 27 89 39 3c npcdemo_start.nw` (12.5, 14); no reply |
| C>S RC socket | 62 update level | GSHORT count, a GSTRING per level | `5e 20 21 30 npcdemo_start.nw` → NC 159 for each NPC id in that level |
| C>S | 104 delete | GINT3 id | → 159 GINT3 id **twice**, and 74 `NPC labnpc deleted by labrc` |
| C>S | 113 class set | GSTRING name, then the script tokenized (quotes doubled) | `91 28 labclass "// lab class","function onCreated() {","  echo(""lab, class"");",}` → 163 bare name + 74 |
| C>S | 112 class get | the name as the rest of the line | `90 labclass` → 162 GSTRING name + the script |
| C>S | 119 class delete | the name as the rest of the line | `97 labclass` → 188 `dc labclass` + 74; no confirm dialog |
| C>S | 115 weapon list | no payload | → 167 repeated GSTRINGs |
| C>S | 116 weapon get | the name as the rest of the line | NCL11012 → **33**; NCL21075 → **192** |
| S>C | 33 (to NCL11012) | GSTRING name, GCHAR 0, GSTRING image, GCHAR 1, GSHORT script length, script with lines joined by `0xA7` (trailing `0xA7` included) | lab300 `21 22 37` = 279; lab9000 `21 64 80` = 8800; labbreaks `21 20 7a` = 90; lab20000 (21212 bytes) `21 c5 7c` |
| S>C | 192 (to NCL21075) | GSTRING name, GSTRING image, then the script joined by `0xA7` as the rest of the line, **no length field** | lab20000: `e0 28 lab20000 2a wbomb1.png // lab20000 line 000 …`, 21221 bytes |
| C>S | 117 weapon set | GSTRING name, GSTRING image, then the script joined by `0xA7` (not tokenized, no length) | 293 bytes for 6 lines; 8820 for lab9000 (round trip byte for byte); 21232 for lab20000 from RC3 → 74 `Weapon/GUI-script … added/updated by …`; a new weapon also gets 167 |
| C>S | 118 weapon delete | the name as the rest of the line | `96 labwep` → 74 + 167 |

How 33 encodes a script length:
- A GSHORT, high 7 bits first. 8800 is `64 80`.
- Above 16383 the fork writes a high byte past 7 bits: 21212 became `c5 7c`, i.e. (0xc5−32)·128 + (0x7c−32). The 2004 RC still showed the whole script.
- Line breaks are `0xA7`. No line feed is ever sent inside 33, 117 or 192.
- The 2004 RC shows each `0xA7` as a new line.

**Fork bug (NC text replies are not quoted):** 157, 160, 161 and 162 join the
lines with commas without quoting them. The 2004 RC detokenizes them, so a
script line `// lab npc script` shows as three lines, and saving it from the RC
would break the script. The sets (109, 110, 113) are tokenized correctly by the
RC and stored correctly by the fork.

Reply ids above 114 seen from the fork: 157, 158, 159, 160, 161, 162, 163,
167, 188, 192. Ids below it that the fork sends to the NC socket: 25, 29, 33
and 74.

## Second pass (07:09 to 07:19 UTC)

This pass used seihai-gserver-v2 at `e3da7ace` (the `PACKET_LOGGING` build that
finished during the first pass) and the same relay with the type mapping fixed.
Its log is the `run4` section of the hex file.

### The 2000 to 2002 builds (direct to the gserver port)

All three logged in with type 1 and plain zlib, the same as rc62101. The fork
logged `[RC] [GSERV020 (unknown)]` and `[RC] [GSERV019 (unknown)]`: it has no
name for those two tokens but accepts them. It logged `[RC] [GSERV014 (1.30)]`
for rc133. rc133 sends no nickname (no 2) after its login.

The start dialog of rc1025 and rc139rev2 (and rc62101) has four server options
with "don't change" boxes. With a box cleared, the RC sends one message right
after the login frame. Each is a 4-byte 7-bit integer:

| Id | Option | Seen |
|---:|---|---|
| 55 | respawn time (bushes etc.), seconds | `57 20 20 20 35` = 21; rc1025 `57 20 20 20 39` = 25 |
| 56 | horse lifetime, seconds | `58 20 20 20 36` = 22 |
| 57 | alignment increment time, seconds | `59 20 20 22 4d` = 301 |
| 58 | baddy respawn time, seconds | `5a 20 20 20 3f` = 31 |

The fork sent nothing back to any of them and logged nothing. Today 55 to 58
mean other things on an RC link (55 is `ADDPLAYER` from the server). rc1025 and
rc139rev2 also used 72/70 for the account list. rc139rev2's Get List button is
hidden by Wine's form scaling; Alt+G sent it.

**rc133 account add (GSERV014):** rc133's account window is a tree of categories
(Guest, Regular, Graalian Police, Trusted Admins, Administrators); opening it and
clicking the categories sent nothing. Add, with name `lab133`, a password and
"No rights", sent **53** (96 bytes): GSTRING account, GSTRING password, then the
dialog's player properties starting with the byte `69`. They include hearts,
rupees, items, colours and the level `onlinestartlocal.graal`; the field
layout of this block is not worked out. Full hex is in the `run4` section. **Fork bug:**
the fork reads 53 as today's "folder config get". It answered with 77 (the
folder config) and created no account. It handles a GSERV014 RC with the
current id meanings.

### The 2004 RC account menu

| Dir | Id | Layout | Seen |
|---|---:|---|---|
| C>S | 85 comments get | the account as the rest of the line | `75 labprops` |
| S>C | 63 | GSTRING account, then the comments tokenized (empty here) | `5f 28 labprops` |
| C>S | 86 comments set | GSTRING account, then the comments tokenized | `76 28 labprops "first comment line","second, with comma"` |
| S>C | 74 | reply to 86: `labrc has set the comments of labprops` | |
| C>S | 87 ban get | the account as the rest of the line | `77 labprops` |
| S>C | 64 | GSTRING account, GCHAR banned, then the reason tokenized | `60 28 labprops 20` (none); after the set `60 28 labprops 21 "local ban reason","line two"` |
| C>S | 88 ban set | GSTRING account, GCHAR banned, then the reason tokenized | `78 28 labprops 21 "local ban reason","line two"`; off: the same with `20` |
| S>C | 74 | reply to 88: `labrc has set the ban of labprops` | |
| C>S | 75 reset | the account as the rest of the line, after a confirm dialog | `6b labprops` |
| S>C | 74 | reply to 75: `labrc has reset the attributes of account: labprops` | |

The comments and the ban reason were stored tokenized, as sent. Turning the ban
off keeps the reason. Reset rewrote the account from `(defaultaccount)`: the
status, the AP counter, the comments and the ban reason were reset, and
`COMMUNITYNAME (defaultaccount)` was written into the account. **Fork bug:**
the community name should stay the account's own name.

### 167 and 158 with two NCs

Two 2004 RCs were on `npcdemo`, each with its NPC-Control socket open. The
second one added a weapon and an NPC.

- 117 for a new weapon: the fork sent **167 (the full list) only to the NC that sent the 117**. The other NC got only the 74 chat line, so its weapon list goes stale until it asks again with 115. No incremental 167 was seen; every 167 is the whole list.
- 111 for a new NPC: the fork sent **158 to every NC**. The new NPC had a level that does not exist (`worldu-15.nw`, the dialog's default), and 158 carried 52 with an empty level: `be 20 27 8a 52 27 labnpc2 53 26 OBJECT 54 20`. The 111 quoted the scripter because it has a space: `labnpc2,1002,OBJECT,"Stefan Knorr",worldu-15.nw,30.5,30`.

No UI path made the fork send 158 with a property other than 50, 51 and 52,
or 159 with extra bytes.

## What was not captured

- RC2_2.05 itself (GTK crash under Wine; RC3 has the same tokens and stood in for it).
- rc141's NPC Control socket (it has NCL11012 like the 2004 RC, so it is expected to match).
- 158 with a property other than 50/51/52; 159 with extra bytes; an incremental 167 (none is sent, see above).
- The 2004 RC's own 98 rename (the RC crashed under Wine) and 155/156 (it has no upload menu); both were captured from RC3.
- rc133's account delete and edit (no account showed in its tree), and the player list and toalls sends of every build: those windows were opened only.

## Fork bugs seen (seihai-gserver-v2 `a3b6ac23`; 7 to 9 at `e3da7ace`)

1. 76/77 (and 157/160/161/162) leave items with spaces unquoted. A real RC's set then splits every line at its spaces: `serveroptions.txt` 572 → 2476 lines, `foldersconfig.txt` one word per line.
2. RC uploads (93, single or large) store the raw block's trailing line feed: +1 byte per piece.
3. 160 folder delete sends 16 and closes the RC link, both when the folder is not empty and after it deleted an empty one.
4. A type-6 RC can get one plain-zlib frame (id 42), with no compression byte, ahead of the gen-5 stream.
5. 104 answers 159 twice for the same id.
6. `BANREASON` is stored with the tokenizer's quotes.
7. rc133's (GSERV014) account add 53 is read as today's folder-config get: the fork answers with 77 and adds no account.
8. 75 reset writes `COMMUNITYNAME (defaultaccount)` into the reset account.
9. A new weapon's 167 goes only to the NC that added it; other NCs keep a stale list.
