# Next steps (2026-10-15)

Where the work stands, and the open work in priority order. Read `CLAUDE.md` first;
`docs/HANDOFF.md` has the Windows notes and how to test without a person at the keyboard.
Follow CLAUDE.md's rules: commit as `dmang-dev <dev@perritodeck.dev>` and sign off every
commit (`git commit -s`, item 8), no personal names anywhere, no GPL code, and ask before
tagging, pushing tags, force-pushing, setting secrets or changing repo settings. Commit in
small pieces, push in batches, and tick items off here as they land.

Items 1–7 are done, and the OBS pass (item 9) has run with its fixes in. Item 8 (going
public) and items 10–12 (the Twitch pass, signed APKs, the rest of the website) come next;
items 13–18 collect the rest of the open work by area,
and the list ends with nice-to-haves. **The order to do it in, who does what, and the
decisions it needs are in [`docs/plan/phase-8.md`](plan/phase-8.md).**

**Where things stand**

| Platform | Status |
|---|---|
| Windows 11 + Fire 7 (KFMUWI, Fire OS 7.3.1.7) | Builds, tests and the manual test plan pass on real hardware (2026-10-12); a few manual steps are still to run (item 14) |
| Linux | Checked on a headless Linux machine on 2026-10-12, after the 109 commits of the Windows bring-up: host fmt, clippy and 34 test suites; app check and 51 Playwright tests; plugins and protocol; the real-app e2e. Not compiled since the 2026-10-13 commits, and never run on a real Linux desktop with a tablet. Not in v0.1.0 (D5): it builds from source and in CI, and joins a later release once tested on real hardware |
| macOS | Host tests passed and a universal `.dmg` was built in CI once (2026-10-11). Stale since then (over 100 host commits later), and never run on a real Mac. Not in v0.1.0 (D5) |
| iOS | Never compiled (its one scheduled CI run never started) |
| Installers (NSIS/MSI) | NSIS built and tested on Windows (2026-10-13); v0.1.0 ships it and no MSI (D5). The MSI was built in CI once (2026-10-11) but is stale, and was never installed or tested |
| CI | Every run of the repository's workflows since 2026-10-11 has stopped within seconds, before any job started: a billing block the maintainer is leaving until the repo goes public, where Actions are free. Nothing since then has been through CI on any OS, and the Release workflow has never run. Until the flip, checks run locally (Linux in WSL); right after it, every workflow runs once by hand and goes green before the first tag (launch checklist §4) |

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

## 7. The website (perritodeck.dev) — ✅ done (2026-10-13), and live (item 12)

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
- Images are WebP from PNG masters, which moved to `docs/assets/site/`;
  [`docs/website.md`](website.md) says how. Every page was checked at 375 px and 1440 px in
  light and dark: no sideways scrolling, and about 155 KB at most before images.
- **Later (2026-10-15):** canonical, Open Graph and `twitter:card` tags on every page and
  a link-preview image (`website/img/og.png`, made by `scripts/og-image.py`); the site's
  notes moved from `website/README.md`, which the site published, to `docs/website.md`.

**Deployed:** the site is live (item 12 has what's left). [`docs/website.md`](website.md)
has the setup and the build command.

**Decision: keep it in this repo, in `website/`.**
- The guide describes the app's features and changes in the same commits as the code.
- The plugin catalogue is built from `plugins/`.
- `scripts/release-version.py` updates `download.html`.
- CI workflows already filter by path, so website-only commits trigger no builds.
- A separate repo would mean cross-repo syncing for all three, for no gain. Cloudflare Pages
  can serve the site from this repo while it's still private.
- Revisit only if the site grows its own framework or outside contributors who shouldn't need
  the code.

**Deploy** (done): the plan here was Cloudflare Pages, as in `docs/website.md` ("Deploy"),
with the build command `python3 plugins/build_catalog.py website/plugins` so the catalogue
publishes too, and the output directory `website`. The live site is a Cloudflare Worker
with static assets on the custom domain instead (item 12). Once the repo is public, GitHub
Pages also works for free.

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
| Old `v0.1.0` tag | Done (2026-10-15): no other clone exists, and there are no tags in this repository or on GitHub. Every version field is still 0.1.0 / versionCode 1, and the first release is v0.1.0 from current `main` (D5, item 13) |
| `ci/e2e-*` branches | Done (2026-10-13): E2E no longer pushes them (its screenshots and logs are workflow artifacts), and the four branches were deleted on GitHub with the maintainer's OK |
| gitleaks | Re-run 2026-10-13 with gitleaks 8.30.1 over every ref (236 commits): no leaks; three new false positives (made-up fixture secrets, a hotkey example) are in `.gitleaksignore`. Re-run once more right before the flip |
| `publish = false` | Done (2026-10-13): every `perritodeck-*` crate has it. `wildcards = "deny"` is set in `host/deny.toml`: cargo-deny 0.20.2 passes licences, bans and sources on both workspaces (2026-10-13). RUSTSEC-2026-0192 is accepted with a reason in `deny.toml` (D6, 2026-10-17): `ttf-parser` (used by `ab_glyph` in `perritodeck-render` to draw key titles) is unmaintained, not a vulnerability, and the library is replaced in a later release (its author suggests `skrifa`). `cargo deny check advisories` still fails on both workspaces (2026-10-17) on advisories published since 2026-10-13: four in `rustls` 0.23.45 (RUSTSEC-2026-0346 to 0349, fixed in 0.23.46), three in `zip` 2.4.2 (RUSTSEC-2026-0367 to 0369, fixed in 9.0.0), and the unmaintained `zvariant`, `zbus_names` and `zvariant_derive` (RUSTSEC-2026-0370 to 0372, through `enigo`'s `ashpd`, and in the app also Tauri's opener and single-instance plugins). The app's workspace also flags the unmaintained `proc-macro-error` (RUSTSEC-2024-0370, through GTK on Linux). Deal with them before CI runs advisories |
| `cargo deny` and `third-party-notices.py --check` in CI | The Release workflow's licences job runs `third-party-notices.py --check` (2026-10-13), so a release fails if `THIRD_PARTY_NOTICES.md` is stale. No push workflow runs it or `cargo deny` yet (item 16). The notices check passes locally |
| `third-party-licenses.txt` in releases | `release.yml` generates it (2026-10-13), attaches it to the release and puts it in the command-line archives and the desktop installers (in the app's resource folder, merged in with `tauri build --config`). The APK carries its own list, generated at build time (next row) |
| Licences screen in the apps | Done (2026-10-15). **Android:** a Gradle task writes `assets/third-party-licenses.txt` from the release runtime classpath (86 libraries; licences from each POM and its parents, resolved through Gradle, not Play Services' oss-licenses, which F-Droid forbids), and Open-source licences on Perrito's home screen shows it, so the APK carries every licence text (launch checklist §3). **Desktop:** Settings → Version → Open-source licences opens the installers' `third-party-licenses.txt`, or the notices on `main` in a build without it. iOS: none yet (not part of the first release). Neither screen has been looked at on a device yet (item 15) |
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
1. ✅ **Mail (D2 in `docs/plan/phase-8.md`, decided 2026-10-15):** ImprovMX (a
   grandfathered Light plan) forwards dev@perritodeck.dev to the maintainer's mailbox. Its
   `MX` records (`mx1.improvmx.com`, `mx2.improvmx.com`), SPF (`v=spf1
   include:spf.improvmx.com ~all`), one DMARC record (`p=none`, aggregate reports to
   Cloudflare's DMARC Management) and its two DKIM `CNAME` records are in Cloudflare DNS.
   Replies as dev@ go through Gmail's "Send mail as" with Gmail's own SMTP, which fails
   DMARC alignment for perritodeck.dev, so DMARC stays at `p=none` (`p=reject` would
   bounce those replies). A mail-tester run scored this route about 5/10 for a real
   message. The upgrade is ImprovMX Premium (its SMTP; the DKIM records are already in
   DNS) or a free SMTP relay, either of which allows `p=reject`. Google removes "Send mail
   as" for custom domains in January 2027, so replies need another route before then.
2. **Old runs and PR #1:** Actions logs and artifacts become public with the repo (E2E
   host logs, logcat, the 2026-10-11 desktop bundles). Decided (D9, 2026-10-15): right
   before the flip, every run and its artifacts are deleted, by Claude with the
   maintainer's OK at that moment; not done yet. Dependabot's pull request #1 (eight
   GitHub Actions updates, opened 2026-10-13) is still open. There are no issues.
3. **Website and visibility together:** the site is live (with noindex) and links to the
   GitHub repo throughout, where visitors get a 404 until the flip, and the repo links to
   the site. Keep the noindex until the flip, and flip soon after the site is complete.
4. ✅ **Trademark sign-off (D4, 2026-10-15):** the nominative naming of Stream Deck,
   Elgato and Companion on the website's compatibility page, in the Key Light plugin, the
   README and the store text (with the not-affiliated line) is signed off as written.
5. **Actions:** confirm that workflows run again once the repo is public.
6. ✅ **Account security (done 2026-10-15, finished 2026-10-17):** the app trusts
   perritodeck.dev for plugin code and github.com/perritodeck for releases. 2FA is on for
   the maintainer's GitHub, Namecheap, Cloudflare and ImprovMX accounts; the perritodeck
   org requires 2FA (`two_factor_requirement_enabled: true`, and no member without 2FA,
   checked through the API); the domain auto-renews at Namecheap. The two parts that check
   didn't cover were confirmed on 2026-10-17: passkey 2FA on the Azure account (D1 keeps
   it for Windows signing), and Namecheap's Domain Lock on perritodeck.dev (the `.dev`
   registry's RDAP shows "client transfer prohibited"). Nothing is left to confirm.

**Repo settings after the flip (maintainer):** private vulnerability reporting (`SECURITY.md`
and the issue template already link to it); Dependabot alerts and security updates; secret
scanning with push protection; a ruleset for `main` and one for `v*` tags; approval for
first-time contributors' workflows; description, topics, website and social preview; the
`triage` and `plugin` labels the issue forms use. Every workflow is path-filtered, so a
required check whose workflow didn't run stays pending and blocks the PR. The ruleset
should require one always-running gate job, but **the gate doesn't exist yet** (item 16):
until it does, require no status checks. Every workflow file sets read-only `permissions`
at the top; only the jobs of `bootstrap-gradle-wrapper.yml` (`contents: write`, to push
its branch) and the Release workflow's publish job ask for write. The repository's default
under Settings → Actions → General is read-only, with Actions not allowed to create or
approve pull requests (checked through the API on 2026-10-13 and 2026-10-15), and it stays
that way: `bootstrap-gradle-wrapper.yml` no longer pushes to `main` but to a
`gradle-wrapper-<version>` branch, and links a compare page in its run summary from which
a person opens the pull request (2026-10-15, DECISIONS), so it works under the ruleset.

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

## 9. OBS: a real-world test pass, both paths 🖥 (maintainer for the setup) — ✅ ran (2026-10-13), fixes re-checked (2026-10-15)

**Result: [docs/testing/obs-results-2026-10-13.md](testing/obs-results-2026-10-13.md).**
Against OBS 31.0.3 with obs-websocket 5.5.6 on Windows 11, 93 of the plan's 110 cases ran.
It found 22 bugs, none blocking a release; all are fixed (OBS plugin 1.0.3 and the host)
and a re-check against the same OBS on 2026-10-15 passed every case it ran. The plugin is
no longer marked Experimental. Still to run (the results' §7):
- with the OBS plugin installed in the app: the host half of every E case, F-follow, D3
  (tablet), D6 (plugin slider), D4, K1, K3 and K6, and plugin values through a tablet (I1);
- with a live stream target: B6, C4, E5, E24, F-stats during a stream, and I6;
- with a person at OBS: A4, A6 (server off), H6, H7, H8, H11, full-screen projectors
  (E21), the virtual camera in a camera app (E10), D8 and L2;
- with a second computer: A9 and J4;
- with a scratch data folder for the app: the app's halves of A2, A3, A5 and H12, H10
  with a host restart, and C8 "not set up";
- not yet run anywhere: H3 (kill), I4 over 30 minutes, and groups A, B, C and F on Linux
  and macOS.

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

The research found bugs that were fixed before the pass (plugin 1.1.2 and 1.1.3; the plan's §1),
among them the `chatters` value calling an endpoint that doesn't exist, EventSub
reconnecting after WebSocket Pings, no validation at start, Sign out not revoking the
token (or not winning against an approval on its way), a ban with no duration being
permanent, one bad channel name blanking the others, and refused subscriptions being
retried every few seconds.

**Needs the maintainer:**
- A developer app at dev.twitch.tv, Client Type **Public** (the plugin uses the device code
  flow with no secret).
- Three accounts: a test broadcaster, a test viewer and a raid target that can stream.
- ~~Affiliate decision~~ **Decided (maintainer, 2026-10-13; `docs/DECISIONS.md`,
  2026-10-15): test accounts only, no Affiliate channel.** Ads, channel points, polls and
  predictions need an Affiliate or Partner channel, so those cases (sections I, J and K)
  are deferred to a later pass on an Affiliate's channel, which the harness's
  `--no-skip-affiliate` is for. On the test broadcaster the pass still records what Twitch
  answers a channel that isn't an Affiliate (B7). The test tools are built
  (`plugins/tools/`).
- **L6 (subs, gifts, cheers, hype trains), still open:** real ones need money on an
  Affiliate's channel, so with test accounts L6 either runs against the Twitch CLI's mock
  server (`event trigger`; the CLI is a download, which needs the maintainer's OK) or waits
  for the Affiliate pass. The harness offers it in stage 8 as a manual case either way.
- Approving the device code in the browser, going live, and acting as the viewer.
- **Product decision:** today every user registers their own Twitch app, which is the
  biggest setup hurdle. A project-owned public client ID with the device code flow would
  remove it.

**Done when** the non-Affiliate groups pass (L6 only if it runs on the mock server),
real wording and payloads are in the fixtures and the plugin README, and anything found is
fixed or listed here. The Affiliate cases stay deferred until an Affiliate channel is
available.

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
- ✅ **The Play upload key (D7, decided 2026-10-17):** if Play ever happens (not for now,
  D8), Play App Signing with the app-signing key Google generates, and the release key as
  the upload key. The Release workflow already signs the AAB with the release key, so
  nothing in it changes. Play installs and GitHub, IzzyOnDroid or F-Droid installs can't
  update each other.
- ✅ **v3 signing** (key rotation): stays off (D7).
- ✅ **The first version:** v0.1.0 on current `main` (D5, decided 2026-10-15; item 13).
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

## 12. The website goes live (maintainer) — mostly done (2026-10-15)

**Where it stands** (checked 2026-10-15; [docs/website.md, Deploy](website.md#deploy) has
the setup and the same checks). `https://perritodeck.dev` is live with the temporary
`X-Robots-Tag: noindex`. A Cloudflare Worker named `perritodeck` with static assets serves
it on the custom domain: it isn't a Pages project, as the 2026-10-13 decision planned, and
`perritodeck.pages.dev` doesn't exist. That settles two earlier questions: the bare domain
is the canonical address, and the domain is attached before launch (the store listings
link the privacy policy, and the real-world passes can use the real catalogue), with
noindex until launch day. The site matches the current `main` (`6861f46`, checked by
15:16 UTC and again at 15:46): `/README.md` answers 404, `/img/og.png` loads, and
`/plugins/index.json` lists the 24 plugins, 13 of them marked Experimental, also for the
app's user agent. `http://` redirects to `https://`, and `www` answers 301 to
`https://perritodeck.dev`, keeping the path and query (one Redirect Rule, plus a proxied
`A` record to 192.0.2.1 for `www`). HSTS is the site's own two-year header from
`_headers`, and Network Error Logging is off. DNSSEC is in force: DS 2371/13/2 is at the
`.dev` registry, and resolvers validate. From outside, the pages carry nothing Cloudflare
injects and no Speed Brain header. Mail is set up as D2 decided (item 8).

**Left:**
- **Keep the Worker or move to Pages (decision D12 in `docs/plan/phase-8.md`):** the
  Worker answers `/nope` with an empty 404 instead of `404.html`, redirects
  `/privacy.html` with a 307, and lacks the `X-Content-Type-Options: nosniff` header Pages
  would add. [docs/website.md, The project](website.md#the-project) lists what keeping the
  Worker takes (a Wrangler config with `not_found_handling`, the header in `_headers`, the
  `workers.dev` address and preview URLs off, and a DECISIONS entry, since the 2026-10-13
  one says Pages), and how to move to Pages instead.
- **Zone settings only the dashboard shows (to confirm with the maintainer):** SSL/TLS
  Full (strict) and minimum TLS 1.2; Web Analytics, Email Address Obfuscation, Rocket
  Loader, Bot Fight Mode and Speed Brain off; Browser Cache TTL set to respect the
  headers ([docs/website.md, The domain](website.md#the-domain), "Left"). From outside,
  the pages carry nothing Cloudflare injects and no Speed Brain header, but that doesn't
  prove they're off. Bot Fight Mode matters most: it can challenge the app's `PerritoDeck`
  user agent, and Look online then fails with a 403.
- ✅ **Where the catalogue comes from (D3, decided 2026-10-17):** `main`. The site
  rebuilds it on every push to `main`, so plugin fixes reach every app's "Look online"
  without an app release. Manifests can't yet say which host version they need (item 17);
  revisit D3 once they can.
- **Look online against the live catalogue:** Plugins → Look online installs a plugin
  from it (not tried yet).
- **Launch day:** [docs/website.md, Launch day](website.md#launch-day) removes the noindex
  header and the "Not released yet" lines, and lists the checks after the deploy.

## 13. Before the first release: what else blocks it

- **CI hasn't run since 2026-10-11 (maintainer).** Nothing after 16:19 UTC that day has
  been through CI: 182 commits (117 touching `host/`) on no OS. Once it runs, start Host
  and Desktop app (platforms = all) by hand. (`gh run list`;
  `docs/windows-review-2026-10.md`)
- **The minified release APK has run on one tablet only.** On 2026-10-13 `assembleRelease`
  was built with a throwaway key and run on the Fire 7: code pairing worked, a key press
  was acked, and the scan screen opens. `apksigner` verified the v1 and v2 signatures; that
  signed APK was 2,164,101 bytes (unsigned 2,124,736). With the licence list and the new
  icons the unsigned APK is 2,179,425 bytes (2026-10-15), far under IzzyOnDroid's ~30 MB
  limit; the signed size needs measuring again once the release key exists.
  `android.yml` now builds the unsigned minified APK, `lintVitalRelease` included,
  in every Android CI run (item 11). E2E still builds debug only
  (`scripts/e2e-android.sh`), and the keep rules cover only kotlinx.serialization and
  OkHttp (`android/app/proguard-rules.pro`). Still to do by hand: an API 22 emulator, QR
  decoding, USB, the deck and sliders, and now the licences screen and the launcher icons
  (item 15).
- ✅ **0.x releases reach the update check (2026-10-15).** `release.yml:59` marks every
  0.x version a pre-release, which `/releases/latest` skips. The app now lists
  `/releases?per_page=10` and picks the highest release meant for the running version:
  drafts and tags `release.yml` doesn't make are skipped, pre-releases count while the app
  is 0.x or a pre-release itself, and `-rc` style versions only for someone already on one
  (`host/crates/host/src/updates.rs`, DECISIONS).
- **The first release is v0.1.0, Windows and Android only (D5, decided 2026-10-15).** It
  publishes the NSIS installer and the APK; the MSI, macOS, the Linux packages and the
  command-line archives aren't in it. No tags exist in this repository or on GitHub, and
  no other clone has one. Every version field already says 0.1.0 / versionCode 1, so the
  tag goes on current `main`; rewrite `fastlane/.../changelogs/1.txt`, which describes the
  very first slice, before tagging. The script has no same-version guard
  (`scripts/release-version.py:177-182`). The F-Droid draft, PKGBUILD, Flatpak manifest
  and metainfo hardcode 0.1.0. `release.yml` publishes only what its `SHIP` list names,
  `windows-nsis` for v0.1.0, besides the APK, the plugin catalogue, the licences and
  `SHA256SUMS` (done 2026-10-15, phase-8 M3-12;
  [releasing, What ships](releasing.md#what-ships)). It hasn't run on GitHub yet.
- ✅ **The installed icons are the paw-of-keys (2026-10-15).** Perrito has the adaptive and
  monochrome set from `docs/assets/android/` and a plain vector for API 22–25
  (`LauncherIconTest` keeps them matching the SVGs); the desktop icons in
  `host/app/src-tauri/icons/` were made with `npx tauri icon` from
  `docs/assets/icon-master-rounded.svg`, and the Flatpak and AUR drafts take theirs from
  there; the desktop window's brand mark and the iOS AppIcon match. Not yet seen on a
  device (item 15).
- **macOS: not in v0.1.0 (D5).** Built in CI once on 2026-10-11 and never run on a Mac.
  Since then 117 host commits (the second binary in the app bundle, the plugin `open`
  message, the firewall code) haven't been compiled for macOS. It keeps building and
  testing in CI once CI runs, and joins a later release after a test on a real Mac
  (`website/download.html` says it's not in this release). A tag doesn't build it, so it
  can't fail the release; a manual Release run with **untested** ticked builds the `.dmg`
  to test.
- **Linux on a real desktop, before Linux joins a release (not in v0.1.0, D5).** 7 of the
  17 commits from 2026-10-13 touch `host/` or `android/` and haven't been compiled on
  Linux. Never done: a physical X11 or Wayland desktop with a tablet on adb USB, the libei
  portal prompt, the tray on GNOME and KDE, installing the `.deb`, `.rpm` or AppImage.
  (`docs/HANDOFF.md`, "Still open")
- **The MSI: not in v0.1.0 (D5).** Built once in CI (`tauri.conf.json` bundle targets
  "all") but never installed. Installing both it (per machine) and the NSIS installer (per
  user) gives two copies and two autostart entries fighting over port 46710. Test it
  (install, upgrade, uninstall, autostart, `perritodeck-mcp.exe` present) before a release
  adds it.
- **Release notes.** The Release workflow's notes are a downloads table, the Android
  certificate and the Android changelog; there's no CHANGELOG, and the only pull request
  is Dependabot's #1, so `--generate-notes` adds little beyond a compare link
  (`release.yml` publish job). The first release needs a hand-written summary: what works,
  what's experimental, known issues.
- ✅ **Third-party notices in the APK (2026-10-15).** The APK carries
  `assets/third-party-licenses.txt`, generated at build time, and Perrito shows it under
  Open-source licences (item 8, launch checklist §3).

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
- **Settings → Open-source licences in a real app:** Playwright covers the window's side and
  the file name `release.yml` bundles, but nobody has run `npx tauri dev` to see it open
  the notices on `main` when the build has no resource file, or an installed build open
  its own copy.
- **A timing-sensitive host test:** `tests::tablets_become_companion_surfaces`
  (`host/crates/host/src/tests.rs`) failed once under load on 2026-10-15 and passed on every
  rerun.

## 15. Android

- ✅ **16 KB pages: checked (2026-10-15), no change needed.** Play requires 16 KB page
  support for apps targeting Android 15+ that ship native libraries. The release APK's 12
  `.so` files (CameraX's and `androidx.graphics.path`'s, for four ABIs, pinned old for
  minSdk 22) are stored compressed, so they're extracted at install and zip alignment
  doesn't apply, and every `LOAD` segment in them is 16 KB-aligned.
- **Android 15/16 on a device.** E2E stops at API 34 while targetSdk is 36, so enforced
  edge-to-edge (15) and predictive back (16) are never exercised; add an API 35/36 run.
  The Home, Licences, Pair and Scan screens now pad for the system bars (`safeDrawing`),
  and `SystemBarsTest` checks Home, Licences and Pair with edge to edge forced on any API
  level; it compiles and runs in e2e's instrumented tests, but hasn't run yet. Two things
  left for a separate look: the deck screen can draw into a display cutout while the bars
  are hidden (phones held sideways), and whether MainActivity should call
  `enableEdgeToEdge()`, which would change how older Android, Fire OS 5–9 included, looks
  too.
  (`compileSdk` and `targetSdk` in `android/app/build.gradle.kts`, `.github/workflows/e2e.yml`)
- **Not yet looked at on a device or emulator:** the licences screen, and the launcher
  icons: the adaptive icon on API 26+ (the Fire 7, Fire OS 7.3 = API 28, shows that one),
  the plain vector fallback on API 22–25 (an API 22 or 25 emulator, or a Fire OS 5 tablet),
  and the themed icon on API 33+.
- **Hardware matrix, then minSdk (maintainer).** The only real tablet so far is the Fire 7
  (Fire OS 7). minSdk 22, with AndroidX and Compose pinned for it, is justified only by
  Fire OS 5, seen only in emulators; raising it after the first release would strand
  installed users. Also untried: a phone, two real tablets on one host, and the Phase 4
  slider-feel and haptics checks. More Kindles are on order. (`docs/DECISIONS.md:14,16`)
- **Page switches stall the main thread on slow tablets** (~560 ms of long frames per
  switch on the Fire 7; item 6 has the numbers). Try per-key image state in the deck screen.
- **Reproducible builds have never been compared between machines, and `fdroid lint` /
  `fdroid build` never run (maintainer, an install)** (`docs/fdroid/README.md`). On one PC,
  clean release builds with the build cache off match byte for byte across folders and
  drives (2026-10-15); an incremental Kotlin compile, or one taken from the build cache,
  doesn't. Whether to keep the startup profile is decision D11 in `docs/plan/phase-8.md`.

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
- **Path filters that miss files the checks read** (2026-10-15): `android.yml` doesn't run
  for `docs/assets/**`, though `LauncherIconTest` checks the launcher icons against
  `docs/assets/*.svg` and `docs/assets/android/**`; `desktop-app.yml` doesn't run for
  `.github/workflows/release.yml`, though `host/app/tests/licences.spec.ts` checks the
  bundled licences' file name against it. Add those paths. Also, no workflow runs
  `cargo test` in `host/app/src-tauri` (`host.yml` tests the host workspace, and
  `desktop-app.yml` runs only fmt and clippy there).

## 17. Plugins and integrations

- **The catalogue tracks `main`, and manifests can't say which host they need.** The
  catalogue is rebuilt from `main` on every push (`docs/releasing.md` step 7,
  `docs/website.md` "Deploy"), so plugin bumps reach every app's Look online before a
  release; D3 (2026-10-17) keeps it that way. There's no minimum host version or
  supported-OS field (`plugins/PROTOCOL.md`), so a plugin that needs a newer host feature
  can install onto an older host. Add the fields before third parties publish, then
  revisit D3.
- **Other service plugins have only met fake servers (maintainer):** Spotify (needs
  Premium), Hue, Key Light, GitLab, Proxmox VE, TrueNAS, Ollama, Weather, Is it up?,
  Wake-on-LAN, MQTT and HTTP, and the built-in Home Assistant. Test them. Until then the
  online catalogue and the desktop app's Look online mark the plugins Experimental, with
  what each was tested against, from `plugins/catalogue_status.json` (2026-10-15,
  DECISIONS; Twitch too, until item 10). Take a plugin off that list once it works against
  the real thing. (each plugin's README, "What's tested") The built-in Home Assistant isn't
  a plugin, so nothing marks it: only the store text calls it experimental (item 18), while
  the website's features page (`features.html:119`), home page (`index.html:138`) and
  integrations guide present it like OBS, which has passed against a real instance.
- **OBS and Twitch issues the research left open** are listed in the test plans
  ([OBS](testing/obs.md#1-fixed-before-this-pass-re-check-these),
  [Twitch](testing/twitch.md#1-fixed-before-this-pass-re-check-these)).

## 18. Docs and store text

- ✅ **Done (2026-10-13), the docs public readers hit first:** the store listing;
  `docs/ARCHITECTURE.md`, rewritten from the code (all thirteen host crates, the desktop
  app, both tablet apps, the protocol's messages, plugins, MCP, Stream Deck plugins,
  Companion, ports, the data folder, the website); `ios/README.md` (`hello` says `ios`,
  and a status line: not yet built); `protocol/README.md` (the Swift types and each app's
  test command); and the audio, windows, screenshot, soundboard, sysmon and clipboard
  READMEs, which now say what ran on Windows on 2026-10-12 and what didn't.
- ✅ **Corrected after review (2026-10-15):** the store listing (`full_description.txt`)
  names the desktop app "PerritoDeck" (D10 in `docs/plan/phase-8.md`; it said
  "PerritoDeck Host" until that was decided); lists the plugins that aren't experimental
  first (timers, clocks, counters, the soundboard, the system monitor, screenshots,
  windows and apps, audio devices, clipboard snippets, now playing, more OBS controls)
  and calls the rest experimental without naming them; names other brands only where
  needed (OBS Studio and Home Assistant, which are built in, Amazon Fire tablets, and the
  closing line, which says it isn't affiliated with or endorsed by Elgato, Corsair or any
  company whose products it works with); marks the built-in Home Assistant "still
  experimental" and takes its slider and live-value examples from OBS, the computer and
  what's playing rather than from Home Assistant (item 17); says USB needs USB debugging
  and Android's adb tool; says ⚠ shows when an action fails and ✓ when a command or a row
  of actions finishes; and says the desktop app has an installer for Windows and runs on
  Linux from source (D5, `docs/distribution.md`). `ARCHITECTURE.md`'s feedback, port and
  website lines, `host/README.md` (now pointing at ARCHITECTURE's crate table) and the
  Stream Deck crate's module comment (property inspectors are served) were fixed too.
- **No long-session soak or sleep test** has been recorded: memory over hours, plugin
  restarts, OBS and Twitch connections held for a whole stream, PC sleep and resume, tablet
  doze, Wi-Fi dropping and coming back. Pair it with the OBS and Twitch passes (a 3–4 hour
  session with logs and memory sampled).
- **Release identities can publish a legal name (maintainer):** an individual Apple
  Developer membership shows it as the seller and in every Developer ID signature; Azure
  Artifact Signing for individuals puts it in the certificate (D1 checks that name before
  wiring it in, next item); a personal Google Play account would publicly show the legal
  name, country and developer email, which is why there's no Play for now (D8,
  2026-10-17). SignPath Foundation certificates name the foundation. Decide per channel
  before signing anything (`docs/releasing.md`, "Signing secrets"). Next to watch:
  Google's Android developer verification (global from 2027) will require apps on
  Google-certified devices, sideloaded ones included, to come from a verified developer.
  It affects the GitHub, IzzyOnDroid and F-Droid builds, not Fire tablets (phase-8 M4).
- **Windows code signing (D1, decided 2026-10-15):** the Azure Artifact Signing account
  stays, and the maintainer is waiting for its identity validation and certificate.
  v0.1.0 ships unsigned unless the certificate arrives first; unsigned installers get
  SmartScreen's "Unknown publisher", so the download page should say so. Before wiring
  the certificate in, check which name it shows as publisher (an individual validation
  shows a legal name). SignPath Foundation (free, needs the public repo) stays the
  alternative.

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
  required-reason API) and TestFlight. It has the paw-of-keys icon now (2026-10-15). Not
  part of the first release.
- **Flatpak and AUR drafts** bundle 13 of 24 plugins and leave out `perritodeck-mcp`;
  never built. Their icons come from `host/app/src-tauri/icons/`, so they have the new one.
- **A Gradle entry in `.github/dependabot.yml`** (it covers only GitHub Actions today; a
  Gradle entry must ignore AndroidX and Compose bumps that need minSdk 23). The site's
  Open Graph tags and preview image are done (2026-10-15); the repository's own social
  preview is a GitHub setting (launch checklist §4).
- **The Settings screenshots** (`docs/assets/screenshot-settings*.png`, made by
  `scripts/screenshots.mjs`) don't show the Open-source licences link yet; refresh them
  with the next round of screenshots.
- **Store submissions** (IzzyOnDroid, F-Droid, winget; Flathub, AUR and Homebrew once
  Linux and macOS are in a release, D5; no Google Play for now, D8) and **release
  automation** for them; **announcement prep** (demo video, launch post, good first
  issues). See `docs/launch-checklist.md` §8–9.
