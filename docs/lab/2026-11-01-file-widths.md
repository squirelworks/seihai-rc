# Lab note: file-link field widths, 2026-11-01

Client: `seihai-rc` on this branch (`Session::list_dir`, `get_file`,
`put_file`), type 6, token `GSERV025`, account `labrc`. No original RC
binary was run.

Server: the gserver-v2 fork (`gs2emu` from the lab tree), loopback, an
ephemeral port. Not the lab port in `docs/lab/HOWTO.md`. Its list check
went to seihai-listserver on another ephemeral loopback port, open
accounts, so the fork would accept the RC. The MIT gserver has no file
link, so it was not the peer. Nothing was listening on the usual lab port
before this run, and that port was not bound.

What was done: after login, list `""` and `world`, get
`world/onlinestartlocal.nw`, put `rc-probe.txt` with the six bytes
`probe` and a line feed. A byte log of the link was decoded with this
crate's gen-5 codec.

Before: those three calls had only been run against the fake gserver in
`tests/file_link.rs`. The widths were unchecked against a running server.

After: the login succeeded (`New RC`, token `GSERV025`). The bytes do not
match.

- List, id 90. Empty path was one byte `0x20`. `world` was one length byte
  then the five letters. The fork sent nothing after either.
- Get, id 92. The payload was one length byte (`0x39`, the character `9`)
  then `world/onlinestartlocal.nw`. The fork's RC log says it downloaded
  `9world/onlinestartlocal.nw`. The reply was not id 100 with id 66 inside.
  It was id 102. The payload was 6 bytes (`26 77 3a 62 7e 33`), then the
  name `onlinestartlocal.nw` with no length byte, then 9353 level bytes,
  the same length as that level file. No GINT3 size. The client reported
  no file reply.
- Put, id 50 raw block, inner id 93. The name length was one byte and the
  fork's log names `rc-probe.txt`, so that length was consumed. The stored
  file is 9 bytes: the three GINT3 size bytes (`20 20 26`) and then
  `probe` and a line feed. The fork then sent id 67, `Uploaded file
  rc-probe.txt`. `put_file` does not wait for that.

At the time of this run the client still sent a length byte on list and
get, and three size bytes on put.

Follow-up: the client now sends the folder and the get path as the rest of
the line, reads id 66 as the directory and id 102 as the file, and puts the
file bytes straight after the name. The six bytes before
`onlinestartlocal.nw` are a GINT5 modification time and the name length
(`0x33` is 19, the length of that name). See `docs/PROTOCOL.md`. A later
loopback run of that client is `docs/lab/2026-11-07-file-wire.md`.

Logs: the probe's stdout, kept outside this repo, and the fork's RC log
on the lab copy. No pcap. The probe program was not committed.
