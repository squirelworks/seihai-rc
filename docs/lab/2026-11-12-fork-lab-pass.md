# Lab pass against seihai-gserver-v2, 2026-11-12 (#26)

Loopback only, inside a Windows 10 lab VM with no network adapter. Everything
ran on `127.0.0.1`.

| Part | Commit | Build |
|---|---|---|
| seihai-gserver-v2 | `seihai` at `1548ee3` (after the beta4 `25396a1` merge) | MinGW-w64, Release, `-DPACKET_LOGGING=ON -DTESTS=OFF -DUPNP=OFF`; its `bin/servers/` lab world |
| seihai-listserver | `main` at `350a56e` | `cargo build --release` |
| seihai-rc | `main` at `afc2107`, then this branch for the fixed rows | `cargo build --release` |
| Real client | Graal 2.22 (`Graal_222.exe`) | as shipped |

Servers: `default` ("My Server", 14802) and `npcdemo` (14804, `serverside =
true`), both with `localhostlist = true` so they register with the lab
listserver (gserver port moved to 14901, because 14900 was in use for a 6.x
login server in the same session). RC accounts `labrc` and `labrc2` were made
from `YOURACCOUNT.txt` with `IPRANGE 127.0.0.1` and added to `staff`, as
`HOWTO.md` says.

## What was run and what was seen

| Command | Ids | Result |
|---|---|---|
| `servers --listserver 127.0.0.1:14922` | 5 / list | 6 servers. The one with HQ level 0 (Lab Classic) is listed as `[U]`, the others `[H]`. |
| `login --for 5 -v` | 6 login | Welcome lines, then 194, 190, 47, 180, 103, 42. |
| `players list`, `rcs list` | 55/56 | Empty, then 1 player with a real 2.22 online. |
| `accounts list`, `add`, `get`, `set` | 72/70, 70, 77/73, 78 | As expected. **Fork #16 confirmed:** `accounts set lab26 --email ...` on an account with load only on, admin level 2 and admin worlds `labworld` kept all three. |
| `players comments` set and get | 85/63, 86 | Kept: `COMMENTS "lab comment one"` in the account file. |
| `players ban --set on --reason` / `off` | 87/64, 88 | Banned with the reason, read back by `accounts get`, then unbanned. |
| `players rights --set 1 --ip-range 127.0.0.1 --set-file` (`rw levels/*`) | 83/62, 84 | Kept: `LOCALRIGHTS 1`, `IPRANGE 127.0.0.1`, `FOLDERRIGHT rw levels/*`. |
| `players props ACCOUNT` (offline) and `--set gralats=7,arrows=11` | 74/72, 76 | Kept in the account file. |
| `players props lab222 --set gralats=13` (2.22 online) | 73/72, 76 | Kept: read back 13 (and 12 arrows from a set before it). The account had no flags, so the "stored flags kept" half of fork #15 could not be shown. |
| `server options`, `server folders`, `server flags` | 51/76, 53/77, 68/61 | Read back the server's files, one item per line. Not set. |
| `files ls`, `put`, `rename`, `rm` | 90/66, 93, 98, 97 | Work. The fork answers 67 `Renamed file ...` and `Deleted file ...`. |
| `files mv` | 96 | **Fixed here.** With the name first the fork answered `Error moving file: No such file or directory`. The fork reads `GSTRING` folder, then the name; with that order: `Moved file world/labfile2.txt to world/heads/labfile2.txt`. |
| `nc discover --server 127.0.0.1:14804` | 94/79 | The address is `127.0.0.1,14804`: a comma, no spaces. |
| `nc npcs` | 158 | One NPC, id 10000 (the Control-NPC). |
| `nc npc get 10000`, `--script` | 103/157, 106/160 | 157 is a variables dump with its lines joined by commas; 160 is the script. |
| `nc classes`, `nc class set`, `nc class get` | 163, 113, 112/162 | Work. |
| `nc class delete` | 119/188 | **Fixed here.** A `GSTRING` name got the RC chat line `error: <length byte>labclass does not exist on this server!` and no 188. The fork reads the bare name and answers 188 with the bare name; with that: `deleted labclass`. |
| `nc weapons` | 115/167 | `Demo Wand`, `-gr_movement`; a set weapon shows up, and is gone after delete. |
| `nc weapon get "Demo Wand"` | 116/33 | **Fixed here.** The fork's 33 has a GCHAR 0 after the name and a GCHAR 1 after the image (the `PLO_NPCWEAPONADD` shape), which the parser did not expect. With them: the script and `image: wbomb1.png`. The script's lines are joined by `0xA7`, which is printed as is. |
| `nc weapon set`, `nc weapon delete` | 117, 118 | Work (seen in the list). |
| `login` while a second RC logs in and sets a player's props | 55 | `[update] 5 labrc2 (RC)`: the fork resends `ADDPLAYER` for the RC that just joined. No `[update]` came for the player whose props were set. |

## Not done in this run

- The byte before the ban reason in 73 with a timed ban from a real RC.
- `files rmdir` (160), `nc npc set` and `nc npc delete`, and the server
  options, folder config and flags **sets**.
- Fork #15's flags: needs an account with stored flags.
