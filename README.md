# seihai-rc

A Graal Remote Control (RC) client for server staff: players, accounts, options, files and NPC Control.

Part of the Seihai server stack:
[seihai-gserver](https://github.com/squirelworks/seihai-gserver) ·
[seihai-npcserver](https://github.com/squirelworks/seihai-npcserver) ·
[seihai-listserver](https://github.com/squirelworks/seihai-listserver) ·
[seihai-rc](https://github.com/squirelworks/seihai-rc) ·
[seihai-gserver-v2](https://github.com/squirelworks/seihai-gserver-v2) (GPL fork of GServer-v2, ships first) ·
client: [seihai](https://github.com/squirelworks/seihai).

**Status: command line.** Log in directly or through the listserver's RC
list, watch the RC chat, list the players online, and send a private
message, an admin message to everyone or to one player, a disconnect or
a warp (`players pm/say/admin-pm/kick/warp`),
and read or set an account's comments, ban and rights (`players comments/ban/rights`), and read or set a player's or an account's props (`players props`).
List, read, change, add and delete accounts (`accounts list/get/set/add/delete`). `nc login` opens the
separate NPC-server socket (type 3, plain zlib) when you already know its
address. NPC Control can also find the NPC-server from the game server and
then list, get, set and delete NPCs, classes and weapons, including class
delete. `servers --secure-login` asks the listserver for the RC list with
the account sealed. A live level
editor calls `Session::list_dir`, `get_file` and `put_file` on the same RC
session; there is no file window in this slice. The plan is in
[docs/PLAN.md](docs/PLAN.md), the protocol as this client speaks it in
[docs/PROTOCOL.md](docs/PROTOCOL.md), the code layout in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), the ordered to-do list in
[docs/NEXT.md](docs/NEXT.md), and the rules every contributor and coding
agent follows in [AGENTS.md](AGENTS.md).

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

# Player actions: WHO is an id or an account from the player list
seihai-rc players pm 2,bob "Meet at the shop" --server 127.0.0.1:14802 --account myrc
seihai-rc players say "Restart in 5 minutes" --server 127.0.0.1:14802 --account myrc
seihai-rc players admin-pm bob "Please stop spamming" --server 127.0.0.1:14802 --account myrc
seihai-rc players comments test1 --server 127.0.0.1:14802 --account myrc
seihai-rc players ban test1 --set on --reason "Cheating" --server 127.0.0.1:14802 --account myrc
seihai-rc players rights test1 --set 513 --server 127.0.0.1:14802 --account myrc
seihai-rc players props bob --server 127.0.0.1:14802 --account myrc
seihai-rc players props test1 --set gralats=100,hearts=2.5 --server 127.0.0.1:14802 --account myrc
seihai-rc accounts list "lab%" --server 127.0.0.1:14802 --account myrc
seihai-rc accounts set test1 --email test1@example.invalid --server 127.0.0.1:14802 --account myrc
seihai-rc players kick bob --server 127.0.0.1:14802 --account myrc
seihai-rc players warp alice cave.nw 30.5 12 --server 127.0.0.1:14802 --account myrc

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

## Build

```
cargo build --release
cargo test
```

The tests run the client against small fake servers on loopback. Lab runs
against the gserver-v2 fork are recorded in [docs/lab](docs/lab).

Releases for Linux, Windows and macOS are built by GitHub Actions on each `v*` tag.

## Licence

`MIT OR Apache-2.0`, at your option (see `LICENSE-MIT` and `LICENSE-APACHE`).
This repo contains no code from GServer-v2, gs2lib or gs2-parser (GPL) and must not.
