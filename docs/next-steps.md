# Next steps (2026-10-13)

Where the work stands, and the open work in priority order. Read `CLAUDE.md` first;
`docs/HANDOFF.md` has the Windows notes and how to test without a person at the keyboard.
Follow CLAUDE.md's rules: commit as `dmang-dev <dev@perritodeck.dev>` and sign off every
commit (`git commit -s`, item 8), no personal names anywhere, no GPL code, and ask before
tagging, pushing tags, force-pushing, setting secrets or changing repo settings. Commit in
small pieces, push in batches, and tick items off here as they land.

Items 1–7 are done. Item 8 (going public) and items 9–12 (real-world OBS and Twitch passes,
signed APKs, the website) come next; items 13–18 collect the rest of the open work by area,
and the list ends with nice-to-haves.

**Where things stand**

| Platform | Status |
|---|---|
| Windows 11 + Fire 7 (KFMUWI, Fire OS 7.3.1.7) | Builds, tests and the manual test plan pass on real hardware (2026-10-12); a few manual steps are still to run (item 14) |
| Linux | Checked on a headless Linux machine on 2026-10-12, after the 109 commits of the Windows bring-up: host fmt, clippy and 34 test suites; app check and 51 Playwright tests; plugins and protocol; the real-app e2e. Not compiled since the 2026-10-13 commits, and never run on a real Linux desktop with a tablet |
| macOS | Host tests passed and a universal `.dmg` was built in CI once (2026-10-11). Stale since then (over 100 host commits later), and never run on a real Mac |
| iOS | Never compiled (its one scheduled CI run never started) |
| Installers (NSIS/MSI) | NSIS built and tested on Windows (2026-10-13). The MSI was built in CI once (2026-10-11) but is stale, and was never installed or tested |
| CI | GitHub Actions hasn't run for the repository since 2026-10-11 (an account matter for the maintainer): nothing since then has been through CI on any OS, and the Release workflow has never run |

Items marked 🖥 need a Windows PC and the test tablet. The rest can be done on any machine.
**(maintainer)** marks a decision or an action only the maintainer can take.

---

## 1. A separate MCP bridge exe, so assistants don't lock the app 🖥 — ✅ done (2026-10-12)

**Landed.** `perritodeck-mcp` (a `[[bin]]` in `host/crates/mcp`, also a second binary of the
app package so Tauri bundles it) is copied to `<data dir>/bin` at start. A copy an assistant is
running is renamed aside and replaced. Settings, `--config` and the guide point at the copy,
and Settings notes assistants still on `perritodeck-app mcp`. Every request carries
`PerritoDeck-Bridge: <version>`, and `tests/old_bridges.rs` replays each version's requests.
Verified on Windows: Claude Code stayed connected through the copy while the app was quit,
both exes were deleted and rebuilt with `npx tauri build --no-bundle`, and the app was
restarted. Its next call worked through the same bridge process. Still to run: the updated
real-app e2e on Linux (CI's weekly job, or by hand on a Linux machine). Also untested: a
macOS universal build with two binaries.

**Problem.** The stdio bridge is the app itself (`perritodeck-app mcp`). While Claude Desktop
or Claude Code has it running, Windows locks `perritodeck-app.exe`, so rebuilds fail, and so
would every installer upgrade.

**Plan.** A copy of a bridge that lives in the install folder would be locked the same way.
So the bridge lives outside the install folder, and its job is kept small enough that old
copies keep working.

- Add a small console binary `perritodeck-mcp` to `host/crates/mcp`, as a `[[bin]]` or its own
  tiny crate. It only does `bridge_stdio_to_http`. It reads the port and token from the same
  settings files as `perritodeck-host mcp`, and takes `--data-dir`. Logging goes to stderr only.
- Ship it with the app (Tauri `externalBin`). On start, the app copies it to
  `<data dir>/bin/perritodeck-mcp(.exe)` when the shipped copy is newer.
  - If the copy fails because the file is locked (an assistant is using it), skip it; the old
    one still works.
  - The bridge's job is a plain stdio ↔ loopback HTTP relay. It is versioned so that an old
    bridge talking to a newer app stays correct; add a test for that.
- The Settings → AI assistants snippets and `claude mcp add` commands point at the copy in the
  data folder.
  - Keep `perritodeck-app mcp` and `perritodeck-host mcp` working for existing configs.
  - Show a one-line note in Settings when an old config still points at the app.
- Update the guide (`website/guide/integrations.html`), DECISIONS and SECURITY. The copied
  exe sits in a user-writable folder, which is the same trust level as the rest of the data
  folder; say so.

**Done when:** with Claude Code connected through the bridge, the app can be quit, rebuilt
and restarted, and `npx tauri build --no-bundle` succeeds. The real-app e2e covers the new
bridge on Linux.

## 2. Firewall: tablets on Wi-Fi silently blocked 🖥 — ✅ done (2026-10-12)

**Decision (maintainer, 2026-10-12): no installer rule, the in-app note only.** The NSIS
installer is per-user, and a rule would need a UAC prompt. Recorded in DECISIONS and
SECURITY.

**Detection ✅ done.**

- Perrito sends `networkFailed` in `hello` over the cable after its Wi-Fi connection timed
  out. Refusals don't count (a restarting PerritoDeck refuses). Because Windows Firewall
  also drops connections while PerritoDeck isn't running, Perrito tries the network once
  more after the cable gets in before reporting. While flagged it re-checks every 30 s,
  and reconnects once when the network answers.
- The host names those tablets. On Windows it also reads Windows Firewall's rules
  (read-only) instead of guessing from the network category: that guess stayed on for good
  on the test PC (Public network, PerritoDeck allowed, tablet always on the cable).
- The Tablets and Pair pages show the steps; the guide has a "Tablet only works with the
  cable" entry.
- Verified on the Fire 7: flagged, banner shown, and cleared by itself 30 s after Wi-Fi
  answered again; no false alarm after a restart.
- Not tried on hardware: a real Windows Firewall block (changing firewall rules was out of
  scope). The query was checked against allowed and unlisted programs, and the banner
  wording by Playwright.

**Problem.** On a network Windows classes as **Public** (the test PC's was), the first-run firewall
prompt's default blocks inbound connections. Each exe path gets its own prompt. Until someone
allows it, tablets only work over the cable, and nothing says why.

**Proposal** (ask before doing the installer part):
- **Detect it.**
  - Perrito already falls back to the cable or to other addresses. Have it report to the host
    when it connected by cable after its Wi-Fi addresses failed: a small optional field in
    `hello`, added to the protocol schema and fixtures.
  - The host also knows the network category (diagnostics already shows it).
  - When a tablet had to fall back, or no tablet has ever connected over Wi-Fi on a Public
    network, show a banner on the Tablets and Pair pages. It explains in plain steps: allow
    PerritoDeck in Windows Security → Firewall, or mark this network Private.
- **Installer** (NSIS hook, which runs elevated): add an inbound allow rule for the installed
  exe, TCP 46710 + mDNS UDP 5353, **Private profile only**; remove it on uninstall.
  - Allowing Public silently is a security choice the maintainer should make, not the
    installer.
  - Record the decision in DECISIONS and SECURITY.
- Guide: a "Tablet only works with the cable" entry in troubleshooting.

## 3. Programs started by Node and Stream Deck plugins die with the plugin — ✅ done (2026-10-12)

**Landed.**
- The plugin protocol has `open` (`target`, or `program` + `args`) answered by `opened`,
  and `hello` lists `open` in `features`. The host opens things with its Open and Launch
  code, outside the plugin's job.
- Node SDK 1.1.0 has `openForUser` and `startForUser`, falling back to starting the
  program itself with older hosts. `plugins/PROTOCOL.md` and the phase-7 notes are
  updated.
- Verified on Windows with a test Node plugin: Character Map started through the host
  survived a plugin restart, while the copy the plugin started itself was ended with it.
- Noticed on the way: a Node plugin doesn't exit by itself on `shutdown`, so the host
  ends it after its grace period (about 2 s per stop). Small SDK follow-up.

Each plugin runs in a Windows Job object with kill-on-close, and breakaway is explicit only.
Python plugins can now use `open_for_user` / `start_for_user` (SDK 1.3.0). Node and Stream
Deck plugins can't, because libuv can't ask for breakaway.

- Add a host-side "open" message to the plugin protocol (`{"event":"open", …}` from plugin to
  host). The host opens a URL, file or program for the user, outside the job, with the same
  rules as the Launch/OpenPath actions. Add it to `sdk-node` (`openForUser`, `startForUser`),
  document it in `plugins/PROTOCOL.md`, and bump the protocol docs.
- For Stream Deck plugins, Elgato's `openUrl` already goes through the host. Programs those
  plugins spawn with `child_process` can't be rescued without dropping the job for them;
  document this in the phase-7 notes rather than weakening the job.

## 4. The last manual check: a UAC prompt during auto-switch 🖥 (1 minute, needs a person) — ✅ done (2026-10-13)

**Result: passes.** With a rule for Character Map, the tablet kept that profile through the
taskbar, a jump list and the UAC secure desktop. It switched to the default profile 0.2 s
after the elevated Command Prompt came to the front. Recorded in
`docs/windows-review-2026-10.md`.

With an auto-switch rule active, trigger a UAC prompt (e.g. open an admin terminal). The
profile should stay put, and should switch correctly after the prompt is answered. Record the
result in `docs/windows-review-2026-10.md`.

## 5. Installers on Windows 🖥 (no signing yet; signing is on hold) — ✅ done (2026-10-13)

**Result.** `npx tauri build --bundles nsis` gives a 7.8 MB per-user installer (no admin) that
carries both `perritodeck-app.exe` and `perritodeck-mcp.exe`. Tauri keeps the NSIS tools in
`%LOCALAPPDATA%\tauri`; they were there from an earlier build, so nothing was downloaded.
Tested on the Windows test PC, started as an ordinary program (see the caveat at the end):

- **Install:** goes into `%LOCALAPPDATA%\PerritoDeck` with a Start menu shortcut, an
  optional desktop shortcut and an uninstall entry. WebView2 was already there.
- **First run** (the installer's "Run PerritoDeck"): Windows Firewall asked, and its
  temporary Block rules were in place until it was answered. With "Allow", the Fire 7
  connected over USB and Wi-Fi with the existing pairing.
- **Autostart:** "Start at login" writes `HKCU\…\Run\PerritoDeck` = `<exe> --hidden`, and
  that command starts the app in the tray only.
- **Upgrade** 0.1.0 → 0.1.1 over the running app, with an assistant connected through the
  bridge:
  - The installer closed the app properly ("stopping plugins before exit") and left none
    of its plugin processes.
  - The running bridge was renamed aside and replaced, and the assistant's next call
    worked through the same bridge process.
  - **Bug, fixed:** the old version's uninstaller, which the upgrade runs, removed the
    login entry, so "Start at login" was silently off after every update. The app now
    remembers the choice (`startAtLogin`) and puts the entry back at start, unless it was
    turned off in Task Manager.
- **Uninstall:** with the app running it asks "PerritoDeck is running! Click OK to kill
  it". It removes the files, shortcuts, uninstall entry and login entry, leaves no
  processes, and keeps the data folder.
- Findings for the release (SmartScreen, the upgrade's uninstall wizard, wording, a
  leftover registry key) are in `docs/launch-checklist.md` §7.
- **Caveat from testing:** a program started from inside an app that is installed as an
  MSIX package (some editors and assistant apps are) runs inside that package, where
  AppData is redirected to `…\Packages\<package>\LocalCache`. PerritoDeck started that way
  keeps its data folder there, and an installer started that way installs there.
  Diagnostics now show "really at …" and the log warns when that happens. The firewall
  check resolves the real exe path.

Build the NSIS installer (`npx tauri build`; download the NSIS/WiX tools when asked), then
check:
- install
- first run
- autostart at login
- upgrade over a running app, with the item-1 bridge connected
- uninstall: no leftover processes; the data folder is kept unless asked

Note SmartScreen's "unknown publisher" for the launch checklist. Do **not** create a release
or tag.

## 6. Small fixes from the Windows review — ✅ done (2026-10-13)

**Landed.**
- A plugin value the manifest declares shows "–" until the plugin reports it (values that take
  an argument match by prefix). Unknown names still show the raw token.
- Editor: plugin titles over an image sit at the bottom of the key and wrap inside it, so
  hello-world's two-line "Hello / Dynamic" fits.
- An image-only Stream Deck key is labelled with its action's name, e.g. "Count up: Stream
  Deck plugin", for screen readers and tooltips.
- USB is offered, and adb reverse set up, only when the host listens where adb arrives: all
  addresses or loopback. With `--listen <LAN address>` the log says why USB is off.
- The program-exit test now has room: the started program holds the pipe for 6 s, the
  timeout is 4.5 s, and the key must finish within 4 s.
- **Wi-Fi latency: not the network.** Profiled with timestamps on both ends (temporary
  logging, since removed), pressing an empty key 12 times on each link:
  - The host's ack reached Perrito a median 11.7 ms after the press went out, over the
    cable and over Wi-Fi alike (cable max 18 ms, Wi-Fi max 40 ms). The host spends under
    0.12 ms on it.
  - Perrito used to time the round trip when its main thread got to the ack. That thread
    also handles touches, draws, and decodes key images. That added 15–90 ms, and up to
    0.5 s after a page key, where the key-up's ack waits behind the new page's images.
    That was the "~105 ms over Wi-Fi". Perrito now times the round trip when the frame
    arrives, and a test covers it.
  - Wi-Fi does cost something for messages the computer starts. Pings from the PC take
    0.3–0.9 s on average to reach the dozing Fire 7, against 2 ms the other way. Heartbeats
    after 2 s idle take ~40 ms, against ~13 ms on the cable. A high-performance Wi-Fi lock
    made no difference on Fire OS, so it wasn't kept. Titles and pages the computer pushes
    unprompted can lag that much on this tablet.

**Left open: page switches hold Perrito's main thread on slow tablets.** Measured with
`dumpsys gfxinfo` over 20 page switches on the Fire 7, cable:

| Build | Frames | Median frame | 99th percentile | Frames over 100 ms, per switch |
|---|---|---|---|---|
| As shipped (images decoded on the main thread) | 53 | 200 ms | 550 ms | ~560 ms |
| Decoding off the main thread, one frame at a time | 295 | 27 ms | 350 ms | ~310 ms |
| Decoding off the main thread, in batches | 90 | 150 ms | 1050 ms | ~640 ms |

Neither variant was kept. One at a time keeps touches responsive, but the page finishes
filling later: the key-up's ack, last in line, was handled a median 0.66 s after the
press over Wi-Fi. Batched is no better than shipped.
Most of the cost seems to be Compose redrawing the grid for new images, so the fix
belongs in the deck screen, e.g. per-key image state so that one new image redraws one
key. Compare against these numbers.

The original list:

- A declared plugin value that hasn't reported yet shows its raw token in small type on the
  key, e.g. `(plugin:…:disk)`. Show "–" for values the plugin's manifest declares. Keep
  showing the raw token for unknown names, so typos stay visible.
- Two-line plugin titles over an image are clipped at the key's bottom edge in the editor
  (hello-world's "Dynamic").
- A key whose plugin draws only an image reads "Untitled" to screen readers. Use the action's
  name.
- With the CLI host on a specific `--listen` address, the USB forwarder still runs and
  Settings offers USB, though adb reverse points at a closed port.
- The test `a_command_that_starts_a_program_finishes_when_it_exits` has a 1.5 s limit that a
  loaded machine overshot once (1.73 s). Give it room.
- Wi-Fi round trip on the Fire 7 is ~105 ms against 6–18 ms on USB. Profile one key press
  end to end (tablet logcat timestamps against host logs) before guessing at a cause.

## 7. The website (perritodeck.dev) — ✅ done (2026-10-13), deploy left to the maintainer

**Landed.**
- **Fonts self-hosted** (`website/fonts/`, Fontsource latin subsets, OFL; the maintainer
  chose the variable Bricolage cut, which keeps the optical-size axis). No page, the plugin
  catalogue included, makes a third-party request; checked in a browser.
- **One menu on every page** (Features, Plugins, Guide, FAQ, Download, GitHub) and footer.
  On phones it wraps under the name instead of disappearing, as it used to.
- **Home** rebuilt around `devices-hero` and the three use-case mockups, with how it works,
  the demo deck and an honest status. The paw-of-keys icon is the site icon.
- **New pages:** Features, Stream Deck and Companion compatibility (from `phase-7.md`;
  names only to describe compatibility), and FAQ.
- **Download:** a tested/not-built status per platform, and a real "Build it yourself"
  section linking CONTRIBUTING.
- **Plugins catalogue:** a search box (shown by script, so the list still works without
  JavaScript) and optional plugin icons (`icon` in the manifest, documented in
  `plugins/PROTOCOL.md`; no plugin has one yet). `plugins/test_build_catalog.py` covers it.
- **Guide:** app screenshots (light and dark), and the fixes from a check of every page
  against the app.
- **Privacy:** covers the website and the iOS app, and matches what Perrito stores and sends.
- Images are WebP from PNG masters, which moved to `docs/assets/site/`; `website/README.md`
  says how. Every page was checked at 375 px and 1440 px in light and dark: no sideways
  scrolling, and about 155 KB at most before images.

**Left for the maintainer:** the Cloudflare Pages setup below. `website/README.md` has the
steps and the build command.

**Decision: keep it in this repo, in `website/`.**
- The guide describes the app's features and changes in the same commits as the code.
- The plugin catalogue is built from `plugins/`.
- `scripts/release-version.py` updates `download.html`.
- CI workflows already filter by path, so website-only commits trigger no builds.
- A separate repo would mean cross-repo syncing for all three, for no gain. Cloudflare Pages
  can serve the site from this repo while it's still private.
- Revisit only if the site grows its own framework or outside contributors who shouldn't need
  the code.

**Deploy** (maintainer, once): Cloudflare Pages, as in `website/README.md` ("Deploy"), with
the build command `python3 plugins/build_catalog.py website/plugins` so the catalogue
publishes too. Output directory: `website`. Then add the custom domain (item 12). Once the
repo is public, GitHub Pages also works for free; either is fine.

**Build out:**
- **Home page:**
  - Use the real assets in `website/img/` (`devices-hero.png`, the tablet and phone frames)
    and the new paw-of-keys icon (`docs/assets/icon-master-rounded.svg` → `website/paw.svg`,
    favicon).
  - Lead with what it is: your old tablet as a Stream Deck.
  - Then the three use cases the screenshots already show (streaming, homelab, desk), how it
    works (pair, edit, press), and the honest status (pre-release, free and open source).
- **Features page:**
  - keys, toggles, sliders, live values, pages and profiles, auto-switching, multi-tablet,
    USB or Wi-Fi
  - integrations: OBS, Home Assistant, Companion, Stream Deck plugins
  - AI assistants (MCP)
  - privacy: nothing leaves your desk
- **Plugins page:** generated by `build_catalog.py`, so improve its template rather than
  hand-writing it. Add a search or filter box and the plugin's icon if the manifest has one.
  It now lists 24 plugins.
- **Stream Deck and Companion compatibility page:** what works and what doesn't (from
  `docs/plan/phase-7.md`).
  - Trademarks: name Elgato's products only to say what PerritoDeck is compatible with.
  - No logos, no look-alike styling, and the existing "not affiliated" line.
- **Download page:**
  - Keep "first release isn't out yet".
  - List platforms with honest status: Windows tested; Linux tested in CI and on a headless
    Linux machine; macOS and iOS not yet.
  - Add a "build it yourself" section linking CONTRIBUTING.
- **Guide:** check every page against the current app, including the MCP section and the new
  Random and Switch profile keys. Add screenshots from `docs/assets/screenshot-*.png`.
- **FAQ:**
  - Which tablets? Fire tablets: why minSdk 22.
  - Do I need a Stream Deck? Does it work with my Stream Deck plugins? iPad?
  - Is it free? What leaves my computer?
- **Privacy:**
  - Self-host the two fonts (Bricolage Grotesque, Atkinson Hyperlegible; both OFL) instead of
    loading Google Fonts. That includes the page template in `plugins/build_catalog.py`.
  - No analytics, no third-party requests at all.
  - Keep `privacy.html` matching what the apps do.
- **Quality bar:** works at phone width, light and dark, keyboard focus visible, no JS
  needed except the demo deck, every page under ~200 KB before images. Check every page in a
  browser at 375 px and 1440 px, and fix what looks off.

## 8. Going public (maintainer decides when) — then turn CI back on

Making the repo public makes GitHub Actions free on **standard GitHub-hosted runners**:
Linux, Windows, and the standard macOS runners like `macos-latest`. **Larger runners** are
always billed, so don't use them. Actions hasn't run since 2026-10-11; check that workflows
run again after the switch (decision 5 below).

Before flipping visibility, work through `docs/launch-checklist.md`.

**Status (2026-10-13).** A sweep of every commit on every ref found no secrets, personal
data or local paths, so nothing in the history stops the repo going public. The 224 commits
are authored only by `dmang-dev <dev@perritodeck.dev>` and `github-actions[bot]`; emails in
content are `dev@`, noreply addresses, `example.com` placeholders and the vendored enigo's
upstream authors; home paths are placeholders and `/home/runner`; no tracked image has text
or EXIF metadata. Most of what's left is GitHub settings and a few decisions.

| Item | Status |
|---|---|
| Old `v0.1.0` tag | Never pushed: there are no tags in this repository or on GitHub. A stale local `v0.1.0` tag on an early commit is left in one other clone; delete it there so it can't be pushed by mistake **(maintainer)**. Every version field is still 0.1.0 / versionCode 1, so v0.1.0 can be released from current `main` (see item 13) |
| `ci/e2e-*` branches | Done (2026-10-13): E2E no longer pushes them (its screenshots and logs are workflow artifacts), and the four branches were deleted on GitHub with the maintainer's OK |
| gitleaks | Re-run 2026-10-13 with gitleaks 8.30.1 over every ref (236 commits): no leaks; three new false positives (made-up fixture secrets, a hotkey example) are in `.gitleaksignore`. Re-run once more right before the flip |
| `publish = false` | Done (2026-10-13): every `perritodeck-*` crate has it. `wildcards = "deny"` is set in `host/deny.toml`: cargo-deny 0.20.2 passes licences, bans and sources on both workspaces (2026-10-13). `cargo deny check advisories` flags one notice, RUSTSEC-2026-0192: `ttf-parser` (used by `ab_glyph` in `perritodeck-render` to draw key titles) is unmaintained; not a vulnerability. Replace it (its author suggests `skrifa`) or accept it with a reason in `deny.toml` before CI runs advisories |
| `cargo deny` and `third-party-notices.py --check` in CI | The Release workflow's licences job runs `third-party-notices.py --check` (2026-10-13), so a release fails if `THIRD_PARTY_NOTICES.md` is stale. No push workflow runs it or `cargo deny` yet (item 16). The notices check passes locally |
| `third-party-licenses.txt` in releases | `release.yml` generates it (2026-10-13), attaches it to the release and puts it in the command-line archives and the desktop installers (in the app's resource folder, merged in with `tauri build --config`). Only the APK doesn't carry it (next row) |
| Licences screen in the apps | None in the desktop app, Android or iOS. **Blocks the release for Android:** the APK carries no third-party licence texts, so it needs them as an asset or a licences screen (launch checklist §3). Android: a screen that reads a text asset generated at build time (not Play Services' oss-licenses, which F-Droid forbids). Desktop, not blocking (the installers carry the file): a link next to Version in Settings |
| Plugin licence | Done (2026-10-13): every catalogue zip carries `plugins/LICENSE` (unless the plugin has its own), and the desktop app bundles it next to the plugins |
| `.gitignore` | Done (2026-10-13): signing files (`*.p12`, `*.p8`, `*.pfx`, `*.mobileprovision`), `.env` and `keystore.properties` are ignored. No signing file is tracked |

**Decided (maintainer, 2026-10-13; recorded in `docs/DECISIONS.md`):**
- ✅ **Copyright wording:** "PerritoDeck contributors" everywhere. `plugins/LICENSE` now says
  so too (it said "The PerritoDeck Authors"); the README, `tauri.conf.json`, the F-Droid
  draft, the Flatpak metainfo, the PKGBUILD and all 24 plugin manifests already did.
- ✅ **Session docs and history:** session, machine and billing details are trimmed in place
  into neutral contributor notes that keep the technical facts: `CLAUDE.md`,
  `docs/HANDOFF.md`, `docs/windows-review-2026-10.md`, `docs/DECISIONS.md`, `SPEC.md`'s
  header, this file, `docs/plan/phase-1.md` and `phase-7.md`. The git history is kept as
  it is: no rewrite and no force-push, so the 81 `Claude-Session:` trailers and the 211
  `-0700` offsets stay. None of them carries a personal name, email or absolute user path.
- ✅ **DCO:** commits are signed off from now on (`git commit -s`), as `CONTRIBUTING.md`,
  the README and the PR template ask. Earlier commits (none of the 224 swept had a
  sign-off) predate the rule and stay as they are (`CONTRIBUTING.md` says so). Enforcing
  it with the DCO app is optional (launch checklist §5); it checks the commits of pull
  requests, so the old ones don't matter.

**Still to decide or do (maintainer):**
1. **Mail and DNS:** send a test mail to dev@perritodeck.dev. It's forwarded by the
   registrar today, which only receives, and replying from the inbox it forwards to would
   show a personal address. Moving the nameservers for the website ends that forwarding, so
   set up the replacement first (item 12).
2. **Old runs:** Actions logs and artifacts become public (E2E host logs, logcat, the
   2026-10-11 desktop bundles). There are no issues or pull requests yet.
3. **Website and visibility together:** the site links to the GitHub repo throughout, and
   the repo links to the site, so deploy the site and flip visibility close together.
4. **Trademark sign-off:** the website's compatibility page, the Key Light plugin, the
   README and the store text name Elgato's products (nominatively, with a not-affiliated
   line). A person should sign off before they go live (`docs/plan/phase-7.md`).
5. **Actions:** confirm that workflows run again once the repo is public.
6. **Account security:** the app trusts perritodeck.dev for plugin code and
   github.com/perritodeck for releases. Require 2FA in the GitHub org (it's off), turn on
   2FA and the registrar lock at the domain registrar, set the domain to auto-renew, and
   use 2FA on the Cloudflare account.

**Repo settings after the flip (maintainer):** private vulnerability reporting (`SECURITY.md`
and the issue template already link to it); Dependabot alerts and security updates; secret
scanning with push protection; a ruleset for `main` and one for `v*` tags; approval for
first-time contributors' workflows; description, topics, website and social preview; the
`triage` and `plugin` labels the issue forms use. Every workflow is path-filtered, so a
required check whose workflow didn't run stays pending and blocks the PR. The ruleset
should require one always-running gate job, but **the gate doesn't exist yet** (item 16):
until it does, require no status checks. Every workflow file sets read-only `permissions`
(only `bootstrap-gradle-wrapper.yml` and the Release workflow's publish job ask for write);
also check that the repository's default under Settings → Actions → General is read-only,
which hasn't been confirmed. `bootstrap-gradle-wrapper.yml` pushes straight to `main`, so
it will fail under the ruleset; make it open a PR.

After it's public:
- Restore push and PR triggers for Linux, macOS and the other emulators, which are weekly or
  manual today because of cost (`host.yml`, `desktop-app.yml`, `e2e.yml`).
- Run the iOS workflow on `macos-latest` (it's on `macos-14`). That's the first iOS build,
  with no rented Mac needed.
- Update CLAUDE.md's CI section to match.
- Set the Android signing secrets and variables (item 11); they're needed for the first
  release, not for going public. A `release` environment, recommended for the three
  secrets, can only be created once the repo is public (a private repo on the Free plan
  can't have one), so create it first if you want one (`docs/releasing.md`).

## 9. OBS: a real-world test pass, both paths 🖥 (maintainer for the setup)

**Plan: [docs/testing/obs.md](testing/obs.md).** The built-in integration (10 actions,
`{obs:*}` values, an OBS slider) and the OBS plugin (26 actions, 30 values) have only run
against fake obs-websocket servers. The plan covers every action and value of both,
sliders, toggles that follow values, Studio Mode, errors and reconnects, performance, and an
automated harness (an independent obs-websocket checker, the plugin driven over its own
protocol, the command-line host and fakedeck on a spare port, and the desktop app over
DevTools).

The research for the plan found bugs in the built-in path that were fixed before the pass:
OBS's status codes explained, a backoff while OBS is unreachable, toggles no longer resent
after a timeout, braces allowed in Follow, and a clear note for a mistake in
`integrations.toml`. The plan's §1 has them all, plus the plugin-side fixes (plugin
1.0.2): the slider's follow with an argument in the editor, the README's examples, the
replay buffer's 604, and `network_dropped_frames` next to the render and encoder skips.
Re-check each one there.

**Needs the maintainer first:**
- Turn on OBS's WebSocket server (keep authentication) and put its password in an
  environment variable for the harness.
- A **PD Test** profile and scene collection, built by the harness through obs-websocket
  requests or by hand; never your real ones.
- A safe streaming target in the PD Test profile only: Twitch's `?bandwidthtest=true` on the
  test broadcaster's stream key (not the main channel's), or a private YouTube event. You
  enter the key; the harness refuses to stream unless the key still ends with the
  bandwidth-test suffix or the server is the private or local target you approved.
- A tablet on USB for the slider and feel checks.
- Decide whether shipping both OBS paths confuses users, and say so in the guide either way.

**Done when** every Auto case passes against the current OBS release, the manual cases are
recorded, OBS's real error wording is in `plugins/obs/README.md` and the guide, and anything
found is fixed or listed here.

## 10. Twitch: a real-world test pass 🖥 (maintainer for accounts and sign-in)

**Plan: [docs/testing/twitch.md](testing/twitch.md).** The plugin's 37 actions and 34
values have only run against a fake Twitch. The plan orders the cases from harmless to
risky, marks which are safe on the main channel, and describes a stdio harness with an HTTP
trace that turns Twitch's real answers into recorded contract fixtures for the unit tests.

The research found bugs that were fixed before the pass (plugin 1.1.2; the plan's §1),
among them the `chatters` value calling an endpoint that doesn't exist, EventSub
reconnecting after WebSocket Pings, no validation at start, Sign out not revoking the
token (or not winning against an approval on its way), a ban with no duration being
permanent, one bad channel name blanking the others, and refused subscriptions being
retried every few seconds.

**Needs the maintainer:**
- A developer app at dev.twitch.tv, Client Type **Public** (the plugin uses the device code
  flow with no secret).
- Three accounts: a test broadcaster, a test viewer and a raid target that can stream.
- **Affiliate decision:** ads, channel points, polls, predictions and real subs, cheers and
  hype trains need an Affiliate or Partner channel. Use the main channel if it qualifies
  (offline where possible), a consenting Affiliate, or the Twitch CLI's mocks (a download,
  so approval first).
- Approving the device code in the browser, going live, and acting as the viewer.
- **Product decision:** today every user registers their own Twitch app, which is the
  biggest setup hurdle. A project-owned public client ID with the device code flow would
  remove it.

**Done when** the non-Affiliate groups pass, the Affiliate cases are run or deferred on
purpose, real wording and payloads are in the fixtures and the plugin README, and anything
found is fixed or listed here.

## 11. Signed APK releases (the key: maintainer; the pipeline: in place)

**Decided (maintainer, 2026-10-13).** The maintainer generates a **new** release key; the
key noted on 2026-10-11 (`docs/DECISIONS.md`) won't be used, and nothing real has been
signed with either. The signing pipeline is in place (2026-10-13) and waits for the key:
until it's set up, the Release workflow can't publish. Builds without the key, Android
CI's included, are unsigned, which is what F-Droid wants.

**The pipeline (in place).**
- `android/app/build.gradle.kts` reads four environment variables, `PERRITO_KEYSTORE`,
  `PERRITO_KEYSTORE_PASSWORD`, `PERRITO_KEY_ALIAS` and `PERRITO_KEY_PASSWORD`. A blank
  `PERRITO_KEYSTORE` counts as unset; a half-set group, or a keystore path that doesn't
  exist, stops the build with a plain message instead of failing deep inside AGP. v1 and
  v2 signing are set explicitly (v1 for Android 5.1 and 6, v2 for 7 and newer) and v3 is
  off. Release builds leave out the git commit (`vcsInfo { include = false }`) and keep
  the prebuilt `.so` files as they come (`keepDebugSymbols`), so builds with and without
  an NDK match. `buildToolsVersion` is pinned at 35.0.0.
- `release.yml` (on a pushed `v*.*.*` tag or by hand) restores the key from three secrets,
  `PERRITO_KEYSTORE_BASE64`, `PERRITO_KEYSTORE_PASSWORD` and `PERRITO_KEY_PASSWORD`, and
  the `PERRITO_KEY_ALIAS` variable, and checks with `keytool` that the keystore opens and
  has that alias before anything is built. A tag build, or a manual run that publishes,
  fails without the key instead of attaching an uninstallable `-unsigned.apk`, and a
  half-set group fails every run. A manual run without "publish" and without the key
  builds `perrito-X.Y.Z-unsigned.apk` as a workflow artifact only.
- The build (`testReleaseUnitTest assembleRelease bundleRelease`) runs with
  `--no-configuration-cache`, since the configuration cache would keep the passwords, and
  a read-only Gradle cache. The decoded keystore is deleted in an `always()` step. Steps
  run in bash with `pipefail`, and every job has `timeout-minutes`.
- `apksigner verify --min-sdk-version 22` must pass with both v1 and v2 verified and
  exactly one signer. When the `PERRITO_CERT_SHA256` variable is set, the certificate must
  match it; without it the job only warns.
- The certificate's SHA-256 goes into the job summary and the release notes, with the
  `apksigner` command to check it, and `website/download.html` has a certificate section
  (`#android-certificate`) waiting for it. `perrito-X.Y.Z.apk` and `SHA256SUMS` go into a
  draft release; the AAB stays the `play-bundle` artifact.
- `android.yml` builds the unsigned, minified (R8) `assembleRelease` in every Android CI
  run, which also runs `lintVitalRelease`, and reports its size against IzzyOnDroid's
  ~30 MB limit (item 13).

**Maintainer steps:**
1. **Generate the key** with JDK 17's `keytool`, letting it prompt for the password:
   `keytool -genkeypair -v -storetype PKCS12 -keystore <SAFE_DIR>\perrito-release.p12
   -alias perrito -keyalg RSA -keysize 4096 -validity 10950 -dname "CN=PerritoDeck,
   O=PerritoDeck"`. With PKCS12 the key password is the store password. Keep it outside
   the repository (`.gitignore` ignores `*.p12` anyway).
2. **Back it up twice, offline:** a password manager (the file and the password) and an
   encrypted USB stick kept somewhere else. Run `keytool -list -v -keystore <copy>` on
   each copy to prove it opens. If the key is lost, existing users can't update.
3. **Send the public SHA-256 fingerprint** from `keytool -list -v` (colon form; F-Droid
   wants it lowercase without colons). It's public. The keystore and its password never
   leave your machine.
4. **Release environment** (recommended, below): it has to wait until the repo is public,
   since a private repo on the Free plan can't have one. Then follow
   [`docs/releasing.md`](releasing.md), "Once the repository is public": create `release`,
   limited to `v*` tags with you as the required reviewer, add `--env release` to the three
   `gh secret set` commands in step 5 (not to the variables in step 6, which stay repository
   variables), and remove the `#` before `environment: release` in `release.yml`'s Android
   job at the same time. Secrets set before then are plain repository secrets: set them
   again with `--env release` and delete the repository copies.
5. **Set the three secrets** as the account that owns the repo, feeding the keystore through
   stdin as [`docs/releasing.md`](releasing.md) shows. Never pass it with `--body`, which
   leaves the whole keystore in the shell history and the process list:
   - bash: `base64 -w0 perrito-release.p12 | gh secret set PERRITO_KEYSTORE_BASE64 -R perritodeck/perritodeck`
   - PowerShell: `[Convert]::ToBase64String([IO.File]::ReadAllBytes((Resolve-Path perrito-release.p12))) | gh secret set PERRITO_KEYSTORE_BASE64 -R perritodeck/perritodeck`
   - then `gh secret set PERRITO_KEYSTORE_PASSWORD -R perritodeck/perritodeck` and
     `gh secret set PERRITO_KEY_PASSWORD -R perritodeck/perritodeck` (the same password;
     each asks, hidden).
6. **Set the two variables**, which are public, not secrets:
   - `gh variable set PERRITO_KEY_ALIAS -R perritodeck/perritodeck --body perrito`
   - `gh variable set PERRITO_CERT_SHA256 -R perritodeck/perritodeck --body <SHA256 from
     keytool>` (colons and case don't matter). Recommended for the first release: the
     Release workflow then refuses an APK signed by any other key, where without it it
     only warns.
7. **Put the fingerprint on the download page** before tagging: in `website/download.html`,
   under "Android: the signing certificate" (`#android-certificate`), replace
   `Not published yet` with the SHA-256 in lowercase without colons, the form `apksigner`
   prints.
8. **Optional:** a local signed build to try on the Fire 7 (`.\gradlew.bat
   --no-configuration-cache assembleRelease` with the four `PERRITO_*` variables set,
   `PERRITO_KEYSTORE` being the path to the `.p12`, then `apksigner verify
   --print-certs`). It installs next to the debug build, so pair again. Never publish a
   locally built APK.
9. **Say go and tag** (item 13 for the version), check the draft (no `-unsigned` APK; the
   fingerprint in the notes matches the one you sent and the one on the download page),
   then publish.

**Still to decide (maintainer):**
- **The Play upload key:** a separate upload key, the release key as the upload key, or
  the release key as Play's app-signing key. Today the Release workflow signs the AAB with
  the release key, while `docs/distribution.md` says to keep the two apart.
- **v3 signing** (key rotation): off until decided.
- **The first version:** v0.1.0 on current `main`, or v0.2.0 (item 13).
- **A release environment (recommended):** a `release` environment with a required
  reviewer and deployments limited to `v*` tags, plus the `environment: release` line in
  the Release workflow's Android job (there as a comment), so the key reaches only a
  reviewed tag build. A private repo on the Free plan can't have one, so it waits until the
  repo is public; until then plain repository secrets work. The steps are in
  `docs/releasing.md`.

**Later:**
- A manual reproducibility job: rebuild unsigned in a clean container like F-Droid's
  (Debian OpenJDK 17, no NDK) and compare with `apksigcopier compare`.
- `docs/fdroid/dev.perritodeck.client.yml`: fill in `AllowedAPKSigningKeys` and `Binaries`
  once the builds match.

## 12. The website goes live (maintainer)

The site and the plugin catalogue are ready to deploy; perritodeck.dev is still parked at
the registrar. **Steps: [website/README.md, Deploy](../website/README.md#deploy):**
Cloudflare Pages with Git integration (it works while the repo is private; a Direct Upload
project can never be connected to Git later), build command
`python3 plugins/build_catalog.py website/plugins`, output folder `website`, then the custom
domain.

**Decide first:**
- **The canonical address:** the bare `https://perritodeck.dev`, with `www` redirecting to
  it. The app's catalogue URL and every link in the repo already use it.
- **When to attach the domain:** now, with a temporary `X-Robots-Tag: noindex` until launch
  day (Play's closed test needs a live privacy policy, and the OBS and Twitch passes can use
  the real catalogue), or at the flip. GitHub links on the site 404 for visitors until the
  repo is public.
- **Mail for dev@:** an apex domain on Pages means moving the nameservers to Cloudflare,
  which ends the registrar's mail forwarding. Set up Cloudflare Email Routing first, add a
  DMARC record, and send a test. Routing only receives; replying as dev@ needs an SMTP relay
  or a hosted mailbox.
- **Where the catalogue comes from:** built from `main`, every plugin version bump reaches
  every app's "Look online" before release, and manifests can't yet say which host version
  they need (item 17). Accept `main`, or deploy from a release branch.

**After the deploy:** check that `https://perritodeck.dev/plugins/index.json` loads (24
plugins) and that Plugins → Look online installs one; that Cloudflare's Web Analytics,
Email Address Obfuscation, Rocket Loader and Bot Fight Mode are off (they'd contradict the
privacy page or block the app's catalogue requests); and the headers, redirects and 404
page (all listed under "Launch day" in `website/README.md`). On launch day, work through
that list: it removes the noindex header and the "Not released yet" lines.

## 13. Before the first release: what else blocks it

- **CI hasn't run since 2026-10-11 (maintainer).** Nothing after 16:19 UTC that day has
  been through CI: 182 commits (117 touching `host/`) on no OS. Once it runs, start Host
  and Desktop app (platforms = all) by hand. (`gh run list`;
  `docs/windows-review-2026-10.md`)
- **The minified release APK has run on one tablet only.** On 2026-10-13 `assembleRelease`
  was built with a throwaway key and run on the Fire 7: code pairing worked, a key press
  was acked, and the scan screen opens. `apksigner` verified the v1 and v2 signatures; the
  signed APK is 2,164,101 bytes (unsigned 2,124,736), far under IzzyOnDroid's ~30 MB
  limit. `android.yml` now builds the unsigned minified APK, `lintVitalRelease` included,
  in every Android CI run (item 11). E2E still builds debug only
  (`scripts/e2e-android.sh`), and the keep rules cover only kotlinx.serialization and
  OkHttp (`android/app/proguard-rules.pro`). Still to do by hand: an API 22 emulator, QR
  decoding, USB, the deck and sliders.
- **0.x releases are invisible to the update check.** `release.yml:59` marks every 0.x
  version a pre-release, and the app asks for `/releases/latest`
  (`host/crates/host/src/updates.rs:8`), which skips pre-releases, so it would say "No
  releases are published yet." until 1.0. Mark only `-suffix` versions as pre-releases, or
  read `/releases` and take the newest non-draft.
- **First version and tag (maintainer).** No tags exist in this repository or on GitHub.
  Re-tag v0.1.0 on current `main` (rewrite `fastlane/.../changelogs/1.txt`, which
  describes the very first slice) or release v0.2.0 (`scripts/release-version.py` raises
  versionCode to 2, so `2.txt` is needed). The script has no same-version guard
  (`scripts/release-version.py:177-182`). The F-Droid draft, PKGBUILD, Flatpak manifest
  and metainfo hardcode 0.1.0.
- **Installed icons are the old plain paw.** `android/.../res/drawable/ic_launcher.xml` says
  it's a placeholder; the finished adaptive and monochrome paw-of-keys set sits unused in
  `docs/assets/android/`. The desktop icons (`host/app/src-tauri/icons/`), the iOS AppIcon
  and the Linux packaging use the old paw too, while the store icon and the website use the
  new one.
- **macOS (maintainer).** Built in CI once on 2026-10-11 and never run on a Mac. Since then
  117 host commits (the second binary in the app bundle, the plugin `open` message, the
  firewall code) haven't been compiled for macOS, and `release.yml`'s publish job needs
  every leg to pass. Once CI runs: test the `.dmg` on a Mac or leave macOS out of the first
  release (`website/download.html` already says "Not built yet").
- **Linux on a real desktop.** 7 of the 17 commits from 2026-10-13 touch `host/` or
  `android/` and haven't been compiled on Linux. Never done: a physical X11 or Wayland
  desktop with a tablet on adb USB, the libei portal prompt, the tray on GNOME and KDE,
  installing the `.deb`, `.rpm` or AppImage. (`docs/HANDOFF.md`, "Still open")
- **The MSI (maintainer).** Built once in CI (`tauri.conf.json` bundle targets "all") but
  never installed. Installing both it (per machine) and the NSIS installer (per user) gives
  two copies and two autostart entries fighting over port 46710. Test it (install, upgrade,
  uninstall, autostart, `perritodeck-mcp.exe` present) or drop it from the first release.
- **Release notes.** The Release workflow's notes are a downloads table, the Android
  certificate and the Android changelog; there's no CHANGELOG and no pull requests, so
  `--generate-notes` adds only a compare link (`release.yml` publish job). The first
  release needs a hand-written summary: what works, what's experimental, known issues.
- **Third-party notices in the APK (release blocker).** The desktop installers and the
  command-line archives carry `third-party-licenses.txt`; the APK carries no licence
  texts. Add them as an asset or a licences screen (item 8, launch checklist §3).

## 14. Desktop app and host

- **Built-in OBS, Home Assistant and Companion are set up by hand-editing
  `integrations.toml` and restarting** (`website/guide/integrations.html`, "The
  integrations file"). There's no form and no "Test connection", and the file the user
  makes isn't owner-only. Plugins already get generated settings forms with masked
  passwords. This is the hardest setup
  step for newcomers and the first thing the OBS pass hits.
- **Importing a `.perritoprofile` gives no warning** about Command, Launch or plugin keys
  that run on the next press (`host/app/src-tauri/src/lib.rs:341-358`,
  `docs/SECURITY.md:20,24`). MCP edits that add such keys need the separate "run" permission;
  imports have no equivalent.
- **macOS bundle:** no entitlements or usage strings (the `macOS` block of
  `tauri.conf.json`), though the host and five plugins drive other apps through
  `osascript`, and ad-hoc signing likely makes users grant Accessibility again after every
  update. Unverified until a Mac run.
- **NSIS findings (maintainer, fix or accept):** an upgrade runs the old uninstaller, whose
  "Delete the application data" checkbox wipes pairings and profiles if ticked; the
  running-app message reads "PerritoDeck is running! Click OK to kill it"; uninstall leaves
  `HKCU\Software\PerritoDeck`; the login entry's path isn't quoted.
  (`docs/launch-checklist.md` §7)
- **Windows manual test-plan steps not yet run (maintainer):** ctrl+shift+esc, win+d, media
  and volume keys, openUrl timing; "Restart now" after a port change, Open data folder, Open
  profile file; auto-switch with Calculator; the Store python stub; mixed-DPI and region
  screenshots; the media plugin's idle CPU; a native `.exe` Stream Deck plugin; an elevated
  OBS. Findings 7, 17 and 18 are fixed but not re-tested, and nothing tells the user when
  the app in front is elevated. (`docs/windows-review-2026-10.md`, "Status after the Windows
  run" and "Windows manual test plan")
- **A real Windows Firewall block has never been tried (maintainer):** detection was checked
  only through the cable flag and wording tests (item 2).
- **Companion (maintainer):** tested only against a fake Companion; check a real 3.x or 4.x
  (an install). Not built: the WebSocket transport, `LAYOUT_MANIFEST`, button subscriptions.
  (`docs/plan/phase-7.md`)
- **Stream Deck plugins (maintainer):** only Elgato's two samples have run. Not tried:
  third-party published plugins and a native `.exe` plugin. (`docs/plan/phase-7.md`)

## 15. Android

- **16 KB pages and Android 15/16.** Play requires 16 KB page support for apps targeting
  Android 15+ that ship native libraries; the APK carries CameraX's and
  `androidx.graphics.path`'s `.so` files, pinned old for minSdk 22. Check the release APK
  with `zipalign -c -P 16 -v 4`. E2E stops at API 34 while targetSdk is 36, so enforced
  edge-to-edge (15) and predictive back (16) are never exercised; add an API 35/36 run.
  (`compileSdk` and `targetSdk` in `android/app/build.gradle.kts`, `.github/workflows/e2e.yml`)
- **Hardware matrix, then minSdk (maintainer).** The only real tablet so far is the Fire 7
  (Fire OS 7). minSdk 22, with AndroidX and Compose pinned for it, is justified only by
  Fire OS 5, seen only in emulators; raising it after the first release would strand
  installed users. Also untried: a phone, two real tablets on one host, and the Phase 4
  slider-feel and haptics checks. More Kindles are on order. (`docs/DECISIONS.md:14,16`)
- **Page switches stall the main thread on slow tablets** (~560 ms of long frames per
  switch on the Fire 7; item 6 has the numbers). Try per-key image state in the deck screen.
- **Reproducible builds have never been compared, and `fdroid lint` / `fdroid build` never
  run (maintainer, an install)** (`docs/fdroid/README.md`).

## 16. CI

- **An always-running gate job** for branch protection: every workflow is path-filtered,
  so a required check would wait forever on a docs-only PR. Until it exists, the `main`
  ruleset requires no status checks (item 8). Build it as one workflow that decides with a
  paths filter (e.g. `dorny/paths-filter`) which jobs run, plus a final `if: always()`
  job that fails if any job it needed failed or was cancelled; require only that final
  job. It can't simply `needs:` the jobs of the other workflows, because `needs` works
  only inside one workflow. Also, `host.yml` triggers only on `host/**` and `protocol/**`
  though host tests read `plugins/` (manifest validation, the Python SDK), and
  `desktop-app.yml` ignores `plugins/` though the app bundles all 24.
  (`.github/workflows/host.yml:4-8`, `desktop-app.yml:4-8`)
- **Plugin tests run on Linux only** (`.github/workflows/plugins.yml:18-30`), so the
  Windows-only tests in clipboard, windows, sdk-python, sysmon and soundboard are always
  skipped; obs, screenshot, windows and spotify needed Windows fixes Linux CI didn't catch.
  Add `windows-latest` on push and macOS weekly.
- **Stream Deck plugin tests may skip silently in Host CI:** no `setup-node`, and they
  print "skipped: needs Node.js 22 or newer" and pass on an older Node
  (`host/crates/sdcompat/tests/plugins.rs:81-83`). Add `actions/setup-node` with Node 22.
- **`cargo deny` and the notices check** (item 8): a cheap Ubuntu job for both workspaces
  plus `scripts/third-party-notices.py --check`. Today only the Release workflow's licences
  job runs the notices check, so stale notices show up at tag time. The Android section of
  the notices is a hand-kept table, so the check can't catch Android dependency changes.
- **Untested MSRV:** `host/Cargo.toml` declares `rust-version = "1.80"`, but only the pinned
  1.97 is ever built. Test it, or raise it to what's verified.

## 17. Plugins and integrations

- **The catalogue tracks `main`, and manifests can't say which host they need
  (maintainer).** The catalogue is rebuilt from `main` on every push (`docs/releasing.md`
  step 7, `website/README.md` "Deploy"), so plugin bumps reach every app's Look online
  before a release. There's no minimum host version or supported-OS field
  (`plugins/PROTOCOL.md`), so a plugin that needs a newer host feature can install onto
  an older host. Accept that or deploy from a release branch (item 12), and add the
  fields before third parties publish.
- **Other service plugins have only met fake servers (maintainer):** Spotify (needs
  Premium), Hue, Key Light, GitLab, Proxmox VE, TrueNAS, Ollama, Weather, Is it up?,
  Wake-on-LAN, MQTT and HTTP, and the built-in Home Assistant. Test them, or label them
  clearly in the catalogue (today only Companion and Stream Deck plugins say Experimental).
  (each plugin's README, "What's tested")
- **OBS and Twitch issues the research left open** are listed in the test plans
  ([OBS](testing/obs.md#1-fixed-before-this-pass-re-check-these),
  [Twitch](testing/twitch.md#1-fixed-before-this-pass-re-check-these)).

## 18. Docs and store text

- **The store listing text is stale:** `full_description.txt` says "PerritoDeck Host",
  leaves out sliders, QR pairing, pages, profiles, live values and plugins, and promises
  macOS. (`fastlane/metadata/android/en-US/full_description.txt:4,15-22`)
- **`docs/ARCHITECTURE.md` is out of date:** an Android-only client, a "Tauri UI (Phase 3)"
  box, dial and touch messages that aren't in the schema, and only four host crates of
  fourteen; no iOS, MCP or plugins. `CONTRIBUTING.md` links to it.
- **Stale status lines elsewhere:** `ios/README.md:68-69` (the protocol has `ios` now); the
  audio, windows, screenshot, soundboard, sysmon and clipboard READMEs still call their
  Windows code unrun; `protocol/README.md:83` leaves the Swift types out of "changing the
  protocol"; `docs/DECISIONS.md:64` says thirteen plugins (there are 24).
- **No long-session soak or sleep test** has been recorded: memory over hours, plugin
  restarts, OBS and Twitch connections held for a whole stream, PC sleep and resume, tablet
  doze, Wi-Fi dropping and coming back. Pair it with the OBS and Twitch passes (a 3–4 hour
  session with logs and memory sampled).
- **Release identities can publish a legal name (maintainer):** an individual Apple
  Developer membership shows it as the seller and in every Developer ID signature; Azure
  Trusted Signing for individuals puts it in the certificate; check what a personal Google
  Play account shows. SignPath Foundation certificates name the foundation. Decide per
  channel before signing anything (`docs/releasing.md`, "Signing secrets").
- **Windows code signing (maintainer):** unsigned installers get SmartScreen's "Unknown
  publisher". SignPath Foundation (free, needs the public repo), Azure Trusted Signing
  (about USD 10 a month) or a hardware token (not usable from hosted CI). Sign, or say so
  on the download page.

## Nice to have

- **Security model leftovers** (`docs/SECURITY.md`): OS keychain storage, frame rate
  limiting, per-launch tokens for the Stream Deck plugin socket, per-assistant MCP tokens
  and an approve-on-host prompt, plugin permissions and signing, a signed catalogue index.
- **The MCP bridge exe is copied into the data folder on every start**, even with MCP off
  (`host/crates/host/src/lib.rs:555-575`); copy it only when MCP is turned on.
- **Windows review findings 16 and 19:** private files get no ACL of their own and the data
  folder roams; the tray menu opens on left-click (left as is at the maintainer's request).
- **The Windows installer needs internet** for WebView2 on some Windows 10 PCs; say so on
  the download page.
- **macOS signing and notarisation** (USD 99 a year, six secrets).
- **Phase leftovers:** Wayland auto-switch, per-tablet latency and link type, the key
  inspector's font size and title position, an overflow page for keys outside a smaller
  grid (Phase 3); APNG, dials and touch strip, subscription-based live values (Phase 4);
  plugins drawing key images, plugin icons (Phase 5); `setState`/`setBrightness`/
  `switchPage` in the protocol, golden-image renderer tests, upstreaming the enigo patch
  (Phase 2); SPEC items never scheduled (long-press, burn-in protection, kiosk mode,
  Discord, a `.streamDeckProfile` importer).
- **Localization:** Android pairing and connection messages are Kotlin literals, and the
  desktop app has no i18n layer; extract them before contributors arrive.
- **Accessibility:** no recorded Narrator, TalkBack, VoiceOver or keyboard-only pass, and
  no automated check; do one before announcing.
- **Node SDK:** plugins don't exit on `shutdown`, so each stop waits ~2 s for the host to
  end them.
- **READMEs** for the HTTP, MQTT and Now playing plugins; macOS, Wayland and some Windows
  paths of the local-system plugins have never run on real systems.
- **A dozing Fire 7 gets messages from the computer 0.3–0.9 s late over Wi-Fi** (item 6); an
  FAQ line.
- **iOS:** never compiled; needs a `PrivacyInfo.xcprivacy` (UserDefaults is a
  required-reason API), a real icon, TestFlight. Not part of the first release.
- **Flatpak and AUR drafts** bundle 13 of 24 plugins, leave out `perritodeck-mcp` and use
  the old icon; never built.
- **Social preview image, Open Graph tags, and a Gradle entry in `.github/dependabot.yml`**
  (it covers only GitHub Actions today; a Gradle entry must ignore AndroidX and Compose bumps
  that need minSdk 23).
- **Store submissions** (IzzyOnDroid, F-Droid, the 14-day Play closed test, winget, Flathub,
  AUR, Homebrew) and **release automation** for them; **announcement prep** (demo video,
  launch post, good first issues). See `docs/launch-checklist.md` §8–9.
