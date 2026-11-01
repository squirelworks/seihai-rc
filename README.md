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
list, watch the RC chat, and list the players online. `nc login` opens the
separate NPC-server socket (type 3, plain zlib) when you already know its
address. NPC Control can also find the NPC-server from the game server and
then list, get, set and delete NPCs, classes and weapons. A live level
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

# Players online, direct or picked from the RC list (by number or name)
seihai-rc players list --server 127.0.0.1:14802 --account myrc
seihai-rc players list --listserver 127.0.0.1:14922 --pick "My Server" --account myrc --tsv

# Log in and print the RC chat and joins for 30 seconds
seihai-rc login --server 127.0.0.1:14802 --account myrc --nick "My RC" --for 30

# NPC-Control socket, when its address is already known (lab port 14804)
seihai-rc nc login --server 127.0.0.1:14804 --account myrc --for 30

# NPC-server address from the game server, then the lists on that socket
seihai-rc nc discover --server 127.0.0.1:14802 --account myrc
seihai-rc nc npcs --discover 127.0.0.1:14802 --account myrc --tsv
seihai-rc nc npc get 12 --server 127.0.0.1:14804 --account myrc
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
