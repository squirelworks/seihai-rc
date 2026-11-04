# Lab note: accounts (stage 3), 2026-11-04

Setup: the `seihai-gserver-v2` fork (`seihai` at `1d7843a`) and
`seihai-listserver` (`main`), in a network namespace whose only interface
is loopback. The staff account was `labrc` and the target `test1`, both
lab accounts. Only lab passwords were used. The captures (pcap) are
private and kept out of this repo.

## 1. A real RC, one field per apply

Client: `RemoteControl.exe` from the Graal Reborn 2.22.0 pack (token
`GSERV024`), under Wine. In the accounts window it listed accounts, then
opened Edit Account on `test1` again and again. Each apply changed one
field. It then added an account `labadd` and deleted it.

| Dir | Id | Payload | Seen |
|---|---|---|---|
| RC to server | 72 | GSTRING name pattern, GSTRING conditions | `!% ` (`%`, none); `$lab% ` |
| server to RC | 70 | a GSTRING per account; nothing when none match | `&labadd%labrc` |
| RC to server | 77 | the account as the rest of the line | `test1` |
| server to RC | 73 | GSTRING account, GSTRING password (empty), GSTRING e-mail, GCHAR banned, GCHAR load only, GCHAR admin level, GSTRING admin worlds, one byte, GSTRING ban reason (tokenized) | `%test1 3lab@example.invalid   $main 20 ,"lab reason"` |
| RC to server | 78 | as 73 without the byte before the reason | below |
| RC to server | 70 | as 78 | `&labadd` (password) `3add@example.invalid  !#all ` |
| RC to server | 71 | the account as the rest of the line | `labadd` |

Each 78, after `%test1 ` (empty password) `3lab@example.invalid`:

| Change in the dialog | Bytes |
|---|---|
| none | `20 20 20 $main 20` |
| admin level 2 | `20 20 22 $main 20` |
| load only ticked | `20 21 20 $main 20` |
| banned ticked (load only still ticked) | `21 21 20 $main 20` |
| ban time 3600 | `20 20 20 $main 20` (the time is not sent) |
| ban reason `lab reason` | `20 20 20 $main ,"lab reason"` |
| admin worlds `main,lab` | `20 20 20 (main,lab ,"lab reason"` |

So the three GCHARs are banned, load only and admin level, and the last
GSTRING is the ban reason. The byte in 73 between the worlds and the
reason was always 0x20, and the RC never sent anything in its place. It is
most likely the ban time. `seihai-rc` does not read a 73 where that byte
is not 0x20.

The RC asked "Do you really want to delete the account" before sending
71.

## 2. seihai-rc

`seihai-rc` from this branch, against the same servers:

| Command | Result |
|---|---|
| `accounts list` | the five lab accounts |
| `accounts list test%` | `test1` |
| `accounts list --where "adminlevel>0"` | none |
| `accounts get test1` | the fields above |
| `accounts set test1 --email ...`, `--reason ...`, `--banned on`, `--banned off` | read back as sent |
| `accounts set test1 --admin-level 2`, `--load-only on`, `--admin-worlds main,lab` | sent; the fork read back 0, no and `main`, and the command named the field and exited 1 |
| `accounts add labadd3 --new-password-env VAR --admin-worlds main` | added; read back |
| `accounts delete labadd3` | refused without `--yes` |
| `accounts delete labadd3 --yes` | deleted; a list of `labadd%` came back empty |
| `accounts get nosuch` | no reply; the command fails at `--timeout` |

A get or list sent right after an add or delete got the answer from
before the change: the 77 after an add had no reply, and the list after a
delete still had the account. `accounts add` and `accounts delete` wait
`--settle` ms (1000 by default) before they read back.

## Fork gaps

The fork's 78 and 70 keep the e-mail, banned and the ban reason. They do
not keep load only, admin level or admin worlds. A readback always shows
0, no and `main`. Its 73 never carries a password, and it does not answer
77 for an account that does not exist.
