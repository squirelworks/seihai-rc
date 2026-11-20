# seihai-rc

A Graal Remote Control (RC) client for server staff: players, accounts, options, files and NPC Control.

Part of the Seihai project: the client [seihai](https://github.com/squirelworks/seihai)
and its library [seihai-core](https://github.com/squirelworks/seihai-core); the Rust
server stack [seihai-gserver](https://github.com/squirelworks/seihai-gserver),
[seihai-listserver](https://github.com/squirelworks/seihai-listserver),
[seihai-npcserver](https://github.com/squirelworks/seihai-npcserver),
[seihai-rc](https://github.com/squirelworks/seihai-rc) and
[seihai-servertools](https://github.com/squirelworks/seihai-servertools); the GPL forks
[seihai-gserver-v2](https://github.com/squirelworks/seihai-gserver-v2) and
[seihai-gs2lib](https://github.com/squirelworks/seihai-gs2lib); and the research notes in
[Graal_analysis](https://github.com/squirelworks/Graal_analysis).

## What it does

seihai-rc is a command-line tool, with an optional window (`seihai-rc gui`,
built with `--features gui`). It can:

- log in directly to a game server, or pick one from the listserver's RC list
  (`servers`, optionally over Seihai's secure login with `--secure-login`);
- show the RC chat and who joins and leaves, keeping a quiet session alive
  with an empty message once a minute;
- list the players and RCs online, send a private message, an admin message
  to one player or everyone, disconnect a player or warp them (`players`);
- read and set a player's or an account's props, comments, ban and rights;
- list, read, change, add and delete accounts (`accounts`);
- browse and edit the world's files: list, download, upload, move, rename and
  delete (`files`). Move, rename and the deletes are confirmed by listing the
  folder again, because their exact layouts have not been captured from a real
  RC yet;
- read and change server options, folder config and server flags (`server`);
- use NPC Control: find the NPC-server through the game server, or connect to
  it directly, then list, read, change and delete NPCs, classes and weapons
  (`nc`).

The same RC session is available as a library (`Session::list_dir`,
`get_file`, `put_file`), which a live level editor uses.

More detail: the plan in [docs/PLAN.md](docs/PLAN.md), the protocol as this
client speaks it in [docs/PROTOCOL.md](docs/PROTOCOL.md), the code layout in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and the to-do list in
[docs/NEXT.md](docs/NEXT.md). Contributors should read [AGENTS.md](AGENTS.md)
for the project rules.

## Getting started

Download the archive for your system from the
[Releases page](https://github.com/squirelworks/seihai-rc/releases), or build from
source (see [Build](#build)). Your account must be staff on the game server
(listed in its staff setting, with an IP range that allows your address). Then
put the password in an environment variable and log in:

```
export SEIHAI_RC_PASSWORD=...
seihai-rc players list --server 127.0.0.1:14802 --account myrc
```

`seihai-rc --help` prints every command and option with a short explanation.

## Use

The password is read from an environment variable, never from the command line.

```
export SEIHAI_RC_PASSWORD=...

# Servers on the listserver's RC list
seihai-rc servers --listserver 127.0.0.1:14922 --account myrc
seihai-rc servers --listserver 127.0.0.1:14922 --account myrc \
  --secure-login --server-key KEY

# Players online, direct or picked from the RC list (by number or name)
seihai-rc players list --server 127.0.0.1:14802 --account myrc
seihai-rc players list --listserver 127.0.0.1:14922 --pick "My Server" --account myrc --tsv
seihai-rc players list --server 127.0.0.1:14802 --account myrc --json

# Other RCs online (from the player list; an RC has an empty level)
seihai-rc rcs list --server 127.0.0.1:14802 --account myrc

# Player actions: WHO is an id or an account from the player list
seihai-rc players pm 2,bob "Meet at the shop" --server 127.0.0.1:14802 --account myrc
seihai-rc players say "Restart in 5 minutes" --server 127.0.0.1:14802 --account myrc
seihai-rc players admin-pm bob "Please stop spamming" --server 127.0.0.1:14802 --account myrc
seihai-rc players comments test1 --server 127.0.0.1:14802 --account myrc
seihai-rc players ban test1 --set on --reason "Cheating" --server 127.0.0.1:14802 --account myrc
seihai-rc players rights test1 --set 513 --server 127.0.0.1:14802 --account myrc
seihai-rc players props bob --server 127.0.0.1:14802 --account myrc
seihai-rc players props test1 --set gralats=100,hearts=2.5 --server 127.0.0.1:14802 --account myrc
seihai-rc players props test1 --set sword=3,level=cave.nw --add-flag quest=2 --add-weapon lantern --server 127.0.0.1:14802 --account myrc
seihai-rc accounts list "lab%" --server 127.0.0.1:14802 --account myrc
seihai-rc accounts set test1 --email test1@example.invalid --server 127.0.0.1:14802 --account myrc
seihai-rc players kick bob --server 127.0.0.1:14802 --account myrc
seihai-rc players warp alice cave.nw 30.5 12 --server 127.0.0.1:14802 --account myrc

# Files: the folder is listed first, then the file is named in it
seihai-rc files ls world/ --server 127.0.0.1:14802 --account myrc
seihai-rc files get world/onlinestartlocal.nw --out start.nw --server 127.0.0.1:14802 --account myrc
seihai-rc files put start.nw world/onlinestartlocal.nw --server 127.0.0.1:14802 --account myrc
seihai-rc files rm world/old.nw --yes --server 127.0.0.1:14802 --account myrc

# Server options, folder config and flags; --set-file replaces and reads back
seihai-rc server options --server 127.0.0.1:14802 --account myrc
seihai-rc server flags --set-file flags.txt --server 127.0.0.1:14802 --account myrc

# Log in and print the RC chat and joins for 30 seconds
seihai-rc login --server 127.0.0.1:14802 --account myrc --nick "My RC" --for 30

# NPC-Control socket, when its address is already known (lab port 14804)
seihai-rc nc login --server 127.0.0.1:14804 --account myrc --for 30

# NPC-server address from the game server, then the lists on that socket
seihai-rc nc discover --server 127.0.0.1:14802 --account myrc
seihai-rc nc npcs --discover 127.0.0.1:14802 --account myrc --tsv
seihai-rc nc npc get 12 --server 127.0.0.1:14804 --account myrc
seihai-rc nc class delete Guard --server 127.0.0.1:14804 --account myrc
seihai-rc nc weapons --server 127.0.0.1:14804 --account myrc
```

`--rc classic` logs in as the 2000 to 2002 RCs did (type 1, plain zlib);
the default is the newer gen-5 login (type 6). `seihai-rc --help` lists every
option. The account must be staff on the gserver and allowed from your IP.

## Command-line options

Every command takes these options where they apply:

| Option | What it does |
| --- | --- |
| `--server HOST:PORT` | The game server to log in to (for `nc` commands, the NPC-server) |
| `--listserver HOST:PORT` with `--pick NAME\|N` | Find the server on the listserver's RC list instead, by name or number |
| `--discover HOST:PORT` | For `nc` commands: ask this game server for the NPC-server's address first |
| `--account NAME` | The RC account |
| `--password-env VAR` | The environment variable with the password (default `SEIHAI_RC_PASSWORD`). Passwords are never taken on the command line |
| `--rc gen5\|classic` | Login type: `gen5` (type 6, the default) or `classic` (type 1, the 2000 to 2002 RCs) |
| `--token TOKEN` | The 8-byte version token (default `GSERV025` for gen5, `GSERV023` for classic) |
| `--nick NAME`, `--no-nick` | The nickname sent after the login (default `*ACCOUNT`), or none |
| `--identity TEXT` | gen5 only: text sent after the password (off by default) |
| `--timeout SECS` | Connect and login timeout (default 10) |
| `--settle MS` | For `players` and `rcs`: wait until no message has come for this long before acting (default 1000) |
| `--secure-login`, `--server-key KEY` | For `servers`: get the RC list over Seihai's secure login, checking the listserver's Ed25519 key |
| `--server-profile P` | How tokenized NC text is written for the server: `seihai-gserver`, `gserver-v2-seihai`, `gserver-v2-beta4` or `seihai-gserver-legacy`; by default it is read from the server's `Server:` line |
| `--tsv`, `--json` | Machine-readable output for the list commands |
| `-v`, `--verbose` | Also show the ids of messages not printed as text |
| `--help`, `-V`, `--version` | Print the usage or the version |

Options that belong to one command, such as `--set`, `--set-file`, `--script`,
`--image`, `--out`, `--yes` and the `accounts` field options (`--email`,
`--banned`, `--admin-level` and so on), are described for each command in
`seihai-rc --help`.

## Build

Install Rust from [rustup.rs](https://rustup.rs/) the first time, then:

```
git clone https://github.com/squirelworks/seihai-rc.git
cd seihai-rc
cargo build --release
cargo test
```

The program is then at `target/release/seihai-rc`.

### The window

```
cargo build --release --features gui
SEIHAI_RC_PASSWORD=... seihai-rc gui --server 127.0.0.1:14802 --account myrc
```

`seihai-rc gui` opens an egui window: a login form (filled from `--server`,
`--account`, `--nick`, `--rc` and the password variable, and logged in at
once when all of them are given), then the players and RCs online on the
left, the RC chat and server messages in the middle, and on the right the
selected player's actions: PM, admin message, warp, read props and
disconnect (click twice). The bar under the chat sends an RC chat line or
an admin message to everyone. Tabs beside Players: **Accounts** (list,
read, edit, delete), **Files** (list a folder, download, upload, rename,
delete) and **NPC Control** (find the NPC-server, then NPCs, classes and
weapons with a script editor) and **Server** (server options, folder
config and flags: read, edit, save and read back). Deletes and server
saves need a second click. The sessions
run on their own threads (`src/live.rs`, `src/live_nc.rs`), so the window
never waits on the network.
The window needs Rust 1.95 or newer to build (egui 0.37); the command line
builds on older toolchains. Linux needs X11 or Wayland and OpenGL at run time.

The tests run the client against small fake servers on loopback. Lab runs
against the gserver-v2 fork are recorded in [docs/lab](docs/lab).

## Releases

Prebuilt Linux, Windows and macOS archives are on the
[Releases page](https://github.com/squirelworks/seihai-rc/releases); the latest
is [v0.1.0](https://github.com/squirelworks/seihai-rc/releases/tag/v0.1.0),
which is command-line only. The window and the `files` and `server` commands
are newer than that release. [VERSIONING.md](VERSIONING.md) explains when a
release is made and what the version numbers mean.

## Licence

`MIT OR Apache-2.0`, at your option (see `LICENSE-MIT` and `LICENSE-APACHE`).
This repo contains no code from GServer-v2, gs2lib or gs2-parser (GPL) and must not.
