# seihai-rc against the fork: the last checks for #26, 2026-11-14

Loopback only, on the lab host GwenTea2. It has no Linux lab VM, so every
program ran inside one `bwrap` network namespace that has only `lo`. No VM
was booted, nothing ran on the desktop and no public server was contacted.
The window ran under Xvfb. The setup is the one in
[`2026-11-13-archived-rc-captures.md`](2026-11-13-archived-rc-captures.md):
the same accounts (`labrc`, `labrc2` staff with `IPRANGE 127.0.0.1`,
`labflags`, `labprops`, `labban`) and servers (`default` on 14802, `npcdemo`
with its NPC server on 14804). The run's files stay in the lab folder on that
host; they are not committed.

| Part | Commit / build |
|---|---|
| seihai-rc | `main` at `bc2d226` (PR #48, the window). CLI on rustc 1.93.1; `--features gui` on the official 1.95.0 toolchain, unpacked into the lab folder only |
| seihai-gserver-v2 | `seihai` at `c851b4e`, Linux, `-DPACKET_LOGGING=ON`, built against the antlr 4.10.1 jar to match the system antlr4-runtime 4.10 |
| seihai-listserver | `main` at `50ea009`, on `127.0.0.2` |

Lab setup notes: with the fork's default `listip` of `127.0.0.1`, RC logins
were refused with "login server offline"; `listip = 127.0.0.2` (where the
lab listserver listens) fixed it. The window needs rustc 1.95 because
eframe/egui 0.37 do; the command line still builds on 1.93.

## Results

| # | Check | Result | Evidence |
|---|---|---|---|
| 1 | Fork #16: an account edit keeps load only, admin level and admin worlds | **pass** | The account file had `LOADONLY`, `ADMINLEVEL` and `ADMINWORLDS` set; `accounts set labprops --email lab@example.invalid` left all three in the file and `accounts get` read them back. `accounts set labprops --load-only on --admin-level 2 --admin-worlds labworld` was stored and read back. `accounts add labnew`, `get` and `delete labnew --yes` pass. |
| 2 | Does the fork send `PLO_OTHERPLPROPS` (8) to an RC? | **yes, for the props it broadcasts** | With `login -v --for 40` open, another RC set `head=head5.png` on the online `labban`: the RC got 8 = `28 2023 2b 8d head5.png` (GSHORT id 3, prop 11) and seihai-rc printed `[update] 3 labban`. RC nicknames arrived the same way. `gralats=5` and `gani=walk` sent no 8 to anyone. A player nickname change was not tried: prop 0 is not in the real RC's 76 prop list, so `players props --set` has no nickname. |
| 3 | `rcs list` (with `labrc2` logged in) and `players list --json` | **pass** | `labrc2` listed as an RC; the JSON array has the `--tsv` fields. An RC's level is one space (`" "`), as the fork sends it. |
| 4 | New props names on the offline `labflags` | **10 of 12 pass**, plus colors, flags and weapons | sword, shield, magic, kills, deaths, x, y, head, body and gani were stored and read back; colors too; `--add-flag labnew=1 --remove-flag labflag` and `--remove-weapon bow` pass. **bomb-power** and **level** fail, and seihai-rc reported both as not kept (exit 1): the fork neither stores nor sends back prop 7, and ignores level for an account that is not online. `--add-weapon lantern` was dropped because `default` has no such weapon; adding `bow` back works. |
| 5 | `--identity test` on the type-6 login | **accepted** | The fork reads the rest of the login line as the identity: it sets the platform to `test` and passes the text on to the listserver in its account check (the listserver logged `identity 4 bytes: ok`). |
| 6 | The window: `seihai-rc gui --server 127.0.0.1:14802 --account labrc` under Xvfb | **pass** | It logged in at once; the player list showed `labban` and the log pane the welcome lines. |
| 7 | Smoke: `files ls world/`, `server flags`, `nc discover`, `nc npcs --discover` | **pass** | Files and flags pass on 14802. NC discovery against 14802 finds nothing, as `default` has no NPC server; against 14804 (`npcdemo`) both `nc discover` and `nc npcs` work. |

## What changes

- `docs/PROTOCOL.md`: the fork does send 8 to an RC, for the props it
  broadcasts (head, nickname), not for gralats or gani.
- The identity text is free: the fork takes it as the platform and forwards
  it to its listserver.
- Two fork-side gaps, not seihai-rc faults: prop 7 (bomb power) is not stored
  or sent back, and a level set on an account that is not online is ignored.
  Filed as seihai-gserver-v2 #83.

No Rust code was changed for this run.
