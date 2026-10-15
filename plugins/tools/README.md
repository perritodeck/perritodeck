# Tools

Test tools for the plugins in this folder. This isn't a plugin: there's no
`perritodeck-plugin.json` here, so `build_catalog.py` leaves the folder out of the online
catalogue (it zips every file of a plugin's folder, which is why these don't live in
`twitch/`). `test_all.py` runs `test_tools.py` along with the plugins' tests. MIT, like
everything under `plugins/`; standard library only.

## Twitch

These are for the real-world test plan in [docs/testing/twitch.md](../../docs/testing/twitch.md).

| File | What |
|---|---|
| `twitch_live_test.py` | The stdio harness. It runs the plan's section 3 cases in the plan's order through the plugin's own protocol and writes a JSON report |
| `twitch_trace.py` | Runs the plugin with every HTTP call traced (tokens redacted), summarises a trace, and turns one into contract fixtures |
| `twitch_cleanup.py` | Deletes the channel points rewards your app made: a dry run unless you pass `--yes` |
| `fake_twitch.py` | The fake Twitch from `twitch/test_plugin.py`, started for dry runs and tests |
| `twitch_common.py` | Shared code: redaction, a small Helix client, the test-folder check |
| `test_tools.py` | Tests of the harness's safety rules and of the tools, against the fakes |

### Against the fakes first

```
py -3 plugins/tools/twitch_live_test.py --dry-run-fake
py -3 plugins/tools/twitch_trace.py summary <the trace it names at the end>
py -3 plugins/tools/twitch_trace.py fixtures <that trace> fixtures.json
py -3 plugins/tools/twitch_cleanup.py --dry-run-fake
```

The dry run runs every case the fakes can answer, answering "yes" by itself. A live run
never does that.

### A live run

You need what the plan's section 2 lists: your own app (Client Type Public) and the accounts
T, V and R. In PowerShell:

```
$env:PERRITODECK_TWITCH_CLIENT_ID = "<your app's client ID>"
py -3 plugins/tools/twitch_live_test.py --broadcaster <T> --viewer <V> --raid-target <R> --main-channel <M>
```

- The harness signs in with the device code flow. It shows the code and waits for you to
  approve it in the browser profile signed in as T. It never types anything into Twitch.
  If another account approves, or nobody does within `--sign-in-minutes` (10), it presses
  Sign out at once: the plugin cancels the attempt and revokes the token.
- Give `--main-channel` every time: it's the backstop that keeps moderation and raids off
  M whatever else you pass. With `--no-skip-affiliate` the harness won't start without it.
- `--only A,B` (or `--only A3,B1`) picks sections or cases. Once you're signed in,
  `--auto-only` runs unattended: only cases that need no person and have no public effect.
- `--check-by-eye` also asks the optional "does it match twitch.tv?" questions, and runs
  B1's second half (T live).
- The long cases are A8 (`--validate-minutes`, 75 by default), M1 (`--soak-minutes`, 120)
  and L10 (`--l10-minutes`, 10). Set one to 0 to skip it. A8 and M1 run at the end of
  stage 11 (see below).
- Sections I, J and K report "needs an Affiliate channel" unless you pass
  `--no-skip-affiliate`. The current pass uses test accounts only and doesn't pass it: the
  Affiliate cases are deferred to a later pass (`docs/DECISIONS.md`, 2026-10-15), which
  gives `--no-skip-affiliate` with an Affiliate or Partner channel as `--broadcaster`.
  That channel is usually M, so with `--no-skip-affiliate` every other case with a public
  effect is refused unless `--only` names it by its ID (`--only C1`, not `--only C`).
  Run it as `--no-skip-affiliate --only I,J,K`. With `--viewer`, V must follow that
  channel too (J5, K2 and K4 need V; without `--viewer` they're skipped).
- The test folder is `%TEMP%\perritodeck-twitch-live-test` unless you pass `--data-dir`.
  It holds the sign-in (`plugin-data/dev.perritodeck.twitch/tokens.json`), the plugin's
  stderr, the trace and the report. The sign-in is kept for the next run. Pass
  `--sign-out-at-end` to revoke it, and delete the folder when you're done. The harness
  refuses PerritoDeck's real data folders and any folder inside the repository, for the
  test folder and for `--trace` and `--report`. On macOS and Linux the folder is made
  mode 700, and one that another user owns, or that others can write to, is refused.
- `--no-trace` runs `twitch/plugin.py` itself, exactly as the host does. By default the
  plugin runs under `twitch_trace.py run`, the same code over the same protocol.

### What the harness refuses

- A case with a public effect runs only after you type `yes`. That covers chat,
  announcements, title, category, tags and labels, markers, clips, raids, shoutouts,
  moderation, ads, polls, predictions and rewards. The prompt lists every action the case
  runs, including those inside its custom steps, and the channel each one acts in.
- Nothing risky happens unless the signed-in account is T: both `tokens.json` and Twitch
  (`GET /users`) must say so.
- Before the first risky case it looks V and R up on Twitch: both must exist, and V must
  follow T (`GET /channels/followers`). It shows their IDs and creation dates and goes on
  only after a typed `yes` that these are your test accounts. Any other answer refuses
  every risky case for the rest of the run.
- Moderation happens only in T's channel and only to V. G4 is the one exception: it also
  bans T itself (Twitch refuses) and a made-up name.
- Raids go only to R; shoutouts to R (or T itself, H5). Chat and clips go only to T's or
  R's channel.
- When T is M, or the sign-in is M's, moderation, chat modes, Shield Mode, AutoMod,
  clearing chat, warnings and raids are refused (`--main-channel`), including the ones a
  person would press by hand in F2, G9 and H3.
- A made-up name (B1, G4) is used only after `GET /users` says no account has it.
- Every action is checked against the effect its case declared, so a case can't post or
  moderate more than it says.
- A case that stops early (Ctrl+C, a refusal, a step it can't do) puts back what it
  changed, newest first: it unbans V, removes mod and VIP, turns chat modes and Shield
  Mode off, puts AutoMod back and cancels the raid. Each of those goes through the same
  checks. What couldn't be put back is printed and listed in the report (`not_undone`).
- Tokens are never printed or written, and the report leaves out the device code. The
  harness itself calls Twitch only for `GET /users` and `GET /channels/followers` (the
  checks above) and `GET /chat/settings` (F1); M5 repeats `GET /users` to use up the rate
  limit.

F2 and G9 act in R's channel, where the harness never moderates: they report "manual",
and with a person there the harness asks for the outcome of doing them by hand.

### Where the harness differs from the plan

| Case | The plan | The harness |
|---|---|---|
| A3, A4, A6 | How: Manual | Semi: the harness presses Sign in (A6: with the Confidential app's client ID) and waits while the person approves, denies or tries the code |
| A4 | Signed out beforehand: no `tokens.json` afterwards | Stage 11 comes after the sign-in, so A4 runs signed in as T and checks the earlier sign-in is kept: a refused sign-in mustn't touch the saved one |
| A9 | Press Add marker after the restart | Shows `live:me` instead: Add marker is a public action, and the refresh happens at start either way |
| A8 | In stage 11's list, before A9 | At the end of stage 11, before M1: it takes two and a half hours |
| M1 | In the background from stage 8 | At the end of stage 11: only one plugin process may hold a sign-in |
| B1 | T offline, then live | The live half runs with `--check-by-eye` (a person goes live) |
| H5 | "while offline" | Last, after the person stops streaming; H1 asks to go live again |

`test_tools.py` checks that every case's How matches the plan's How column unless it's in
this table, and that the cases run in the plan's order but for A8 and M1.

### The report

The report is a JSON file (`--report`, or `report-<time>.json` in the test folder),
rewritten after every case. It has a summary count, then each case's id, stage, how, effect
and status, plus a reason and the steps, with the plugin's answer to each action. A status
is one of: pass, fail, recorded (the plan says to note what Twitch did), skipped, refused,
manual (a person runs it) or needs-affiliate. A case that stopped early has its undo steps
("undo: run ...") after its own.

### Found so far

The tools' probe of `tokens.json` found that on Windows a save failed ("Access is denied")
while another thread was reading the file, which could lose Twitch's new single-use refresh
token. That's fixed in plugin 1.1.3 (`replace_file` retries the replace); A10 re-checks it
on real Twitch (the plan's section 1).

### Traces and fixtures

`twitch_trace.py`'s docstring describes the trace records and the fixture format. In
short, `fixtures` writes one recorded request and answer per distinct call and error, plus
EventSub event payloads, with account names and IDs replaced by stand-ins. A fake server can
replay them with `find_fixture(fixtures, method, path, query)`.

### Removing test rewards

```
py -3 plugins/tools/twitch_cleanup.py --match "PerritoDeck test"
py -3 plugins/tools/twitch_cleanup.py --broadcaster <T> --match "PerritoDeck test" --yes
py -3 plugins/tools/twitch_cleanup.py --sign-in --broadcaster <M> --match "PerritoDeck test" --yes
```

By default it uses the harness's sign-in. It never refreshes that token, because the
plugin keeps using it. With `--sign-in` it signs in on its own (asking only for
`channel:manage:redemptions`) and revokes that sign-in at the end. `--yes` needs
`--broadcaster`: nothing is deleted unless the sign-in belongs to that account. It lists
every reward the app made, so use `--match` if the app also manages real rewards on that
channel.
