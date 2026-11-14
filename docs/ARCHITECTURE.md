# Architecture

How `seihai-rc` is put together, and where new work goes. Read `AGENTS.md`
(the rules) and `docs/PROTOCOL.md` (the wire) first.

## Layers

```
main.rs        command line: parse options, run one command, print, exit code
   │
rc.rs          RC session with a gserver: login, Event, PlayerList, message builders
files.rs       file link on that session: list, get, put
listserver.rs  RC list from the listserver: handshake, account, server picking
   │
link.rs        one TCP link: frame codec (Plain / Gen5), send, recv with a deadline
wire.rs        GCHAR / GSTRING encoders, Latin-1 text
   │
seihai-core    (squirelworks/seihai-core, git tag) gen-5 cipher, zlib/bz2, message
               splitting, GSHORT, list parsing
```

Rules for the layers:

- `link` knows frames, never message ids.
- `rc` and `listserver` know message ids and payloads, never the terminal.
- `main` is the only place that prints or reads the environment.
- Encoding and decoding are pure functions (`login_body`, `parse_event`,
  `parse_addplayer`, ...) so they can be unit-tested with exact bytes.
- Where `seihai-core` already has a decoder, use it. When a new shared piece is
  needed by both the client and the RC (for example a prop table), add it to
  the separate `squirelworks/seihai-core` repo, release a tag through its owner,
  then update the dependency here. `Cargo.toml` records the current tag.

## Session model

`Session` is blocking and single-threaded: `next_event(deadline)` reads frames
until an event is ready or the deadline passes. Commands are short-lived
(log in, do one thing, exit), which keeps the command-line tools simple and
scriptable.

The desktop window (egui, `src/ui.rs`) needs a live session.
`live::LiveSession` runs the `Session` on its own thread, sends `Update`s
(events, sent notes, errors, props, close) to the UI over a channel and takes
`Command`s (chat, PM, admin messages, kick, warp, props, accounts, files,
server settings, NPC-server discovery, quit) from another. `live_nc::LiveNc` does the same
for the NPC-Control socket, with `NcView` as its state.
`live::RcView` is the state the window draws, built only from `Update`s, so
it is tested without egui. No async runtime is needed for one link; do not
add tokio unless a feature needs many links.

## Where new features go

| Feature (PLAN.md) | Code | Command line |
|---|---|---|
| 2. Players: PM, message all, warp, disconnect, props, rights, ban | `src/players.rs` (builders + parsers), events in `rc::Event` | `seihai-rc players pm/warp/kick/props/rights/ban` |
| 3. Accounts | `src/accounts.rs` | `seihai-rc accounts list/get/set/add/delete/reset` |
| 4. Server options, folder config, flags, file browser | `src/server.rs`, `src/files.rs` | `seihai-rc server options/folders/flags [--set-file PATH]`, `seihai-rc files ls/get/put/mv/rename/rm/rmdir` |
| 5. NPC Control | `src/nc.rs` for the type-3 socket; discovery (ids 94 and 79) in `src/rc.rs` | `seihai-rc nc discover`, `nc npcs`, `nc npc`, `nc classes`, `nc class`, `nc weapons`, `nc weapon` |
| Window | `src/ui.rs` behind the `gui` cargo feature, so the CLI builds without egui; RC and NPC Control workers/view state in `src/live.rs` and `src/live_nc.rs` | `seihai-rc gui` (Players, Accounts, Files, NPC Control and Server tabs) |

Each new message:

1. Add the id as a `const` next to the others (ids are facts and may come from
   any source; layouts must come from our notes, captures or lab runs).
2. Write the builder or parser as a pure function with a byte-exact unit test.
3. Add a case to the fake gserver in `tests/fake_servers.rs`.
4. Run it against the fork in the lab (`docs/lab/HOWTO.md`) and record the run
   in `docs/lab/`.
5. Document the payload in `docs/PROTOCOL.md`.

## Command line

The parser in `main.rs` is hand-written to keep the dependency list short.
When the subcommand tree grows past stage 2, switching to `clap` (MIT/Apache)
is fine; keep the option names already documented in the README.

Passwords only come from an environment variable. Keep it that way: command
lines end up in shell history and process lists.

## Testing

- Unit tests next to the code: exact bytes for every builder and parser.
- `tests/fake_servers.rs`: small fake gserver and listserver on ephemeral
  `127.0.0.1` ports. They must stay independent of the fork's code.
- `tests/real_listserver.rs`: starts the real `seihai-listserver` dev-dependency
  on loopback and checks RC lists and password refusal. Its pin is in `Cargo.toml`.
- `tests/live_session.rs` and `tests/window_tabs.rs`: exercise GUI workers and
  view state against fake peers; they do not drive the window's selection and
  confirmation controls or establish real-fork GUI acceptance.
- The fork's dated CLI/protocol lab receipts live in `docs/lab/`. The #49
  window smoke test covers login and the player list, not all four #50 tabs.
  Track four-tab runtime acceptance and macOS validation separately in
  [roadmap #27](https://github.com/squirelworks/seihai-rc/issues/27).
