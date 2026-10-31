# Architecture

How `seihai-rc` is put together, and where new work goes. Read `AGENTS.md`
(the rules) and `docs/PROTOCOL.md` (the wire) first.

## Layers

```
main.rs        command line: parse options, run one command, print, exit code
   │
rc.rs          RC session with a gserver: login, Event, PlayerList, message builders
listserver.rs  RC list from the listserver: handshake, account, server picking
   │
link.rs        one TCP link: frame codec (Plain / Gen5), send, recv with a deadline
wire.rs        GCHAR / GSTRING encoders, Latin-1 text
   │
seihai-core    (squirelworks/seihai, git tag) gen-5 cipher, zlib/bz2, message
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
  `seihai-core` in the `seihai` repo, tag a release there, then bump the tag here.

## Session model

`Session` is blocking and single-threaded: `next_event(deadline)` reads frames
until an event is ready or the deadline passes. Commands are short-lived
(log in, do one thing, exit), which keeps the command-line tools simple and
scriptable.

The desktop window (later, egui) needs a live session. Plan: run the
`Session` on its own thread, send `Event`s to the UI over a channel and take
commands (`Command` enum: chat, PM, warp, ...) from another. No async runtime is
needed for one link; do not add tokio unless a feature needs many links.

## Where new features go

| Feature (PLAN.md) | Code | Command line |
|---|---|---|
| 2. Players: PM, message all, warp, disconnect, props, rights, ban | `src/players.rs` (builders + parsers), events in `rc::Event` | `seihai-rc players pm/warp/kick/props/rights/ban` |
| 3. Accounts | `src/accounts.rs` | `seihai-rc accounts list/add/edit/del` |
| 4. Server options, folder config, flags, file browser | `src/server.rs`, `src/files.rs` | `seihai-rc options get/set`, `seihai-rc files ls/get/put` |
| 5. NPC Control | separate login to the NPC-server (NC); `src/nc.rs` | `seihai-rc nc ...` |
| Window | `src/ui/` behind a `gui` cargo feature, so the CLI builds without egui | `seihai-rc gui` |

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
- Later: CI against `seihai-gserver` (the Rust gserver) once its RC endpoint
  (its stage 4) exists. Until then the fork is checked by hand in the lab.
