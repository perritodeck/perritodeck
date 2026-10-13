# Next steps (handoff, 2026-10-12)

The task list for the next working session, in priority order. Read `CLAUDE.md` and
`docs/HANDOFF.md` (what the Windows session found) first, and follow CLAUDE.md's rules:
commit as `dmang-dev <dev@perritodeck.dev>`, no personal names anywhere, no GPL code, and ask
before tagging, pushing tags, force-pushing, setting secrets or changing repo settings.
Commit in small pieces, push in batches, and tick items off here as they land.

**Where things stand**

| Platform | Status |
|---|---|
| Windows 11 + Fire 7 (KFMUWI, Fire OS 7.3.1.7) | Builds, tests and the full manual test plan pass on real hardware (2026-10-12) |
| Linux | After pulling all 109 Windows-session commits, these all pass: host fmt, clippy and 34 test suites; app check and 51 Playwright tests; plugins and protocol; the real-app e2e (cloud session, 2026-10-12) |
| macOS, iOS | Never compiled |
| Installers (NSIS/MSI) | Never built |

Items marked 🖥 need the Windows PC (and the tablet). The rest can run in any session,
including the cloud one.

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
real-app e2e on Linux (CI's weekly job or the cloud session; WSL here has no toolchain). Also
untested: a macOS universal build with two binaries.

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
  on this PC (Public network, PerritoDeck allowed, tablet always on the cable).
- The Tablets and Pair pages show the steps; the guide has a "Tablet only works with the
  cable" entry.
- Verified on the Fire 7: flagged, banner shown, and cleared by itself 30 s after Wi-Fi
  answered again; no false alarm after a restart.
- Not tried on hardware: a real Windows Firewall block (changing firewall rules was out of
  scope). The query was checked against allowed and unlisted programs, and the banner
  wording by Playwright.

**Problem.** On a network Windows classes as **Public** (this PC's is), the first-run firewall
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

## 4. The last manual check: a UAC prompt during auto-switch 🖥 (1 minute, needs a person)

With an auto-switch rule active, trigger a UAC prompt (e.g. open an admin terminal). The
profile should stay put, and should switch correctly after the prompt is answered. Record the
result in `docs/windows-review-2026-10.md`.

## 5. Installers on Windows 🖥 (no signing yet; signing is on hold)

Build the NSIS installer (`npx tauri build`; download the NSIS/WiX tools when asked), then
check:
- install
- first run
- autostart at login
- upgrade over a running app, with the item-1 bridge connected
- uninstall: no leftover processes; the data folder is kept unless asked

Note SmartScreen's "unknown publisher" for the launch checklist. Do **not** create a release
or tag.

## 6. Small fixes from the Windows session (any session)

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

## 7. The website (perritodeck.dev) — any session

**Decision: keep it in this repo, in `website/`.**
- The guide describes the app's features and changes in the same commits as the code.
- The plugin catalogue is built from `plugins/`.
- `scripts/release-version.py` updates `download.html`.
- CI workflows already filter by path, so website-only commits trigger no builds.
- A separate repo would mean cross-repo syncing for all three, for no gain. Cloudflare Pages
  serves the site from this private repo already.
- Revisit only if the site grows its own framework or outside contributors who shouldn't need
  the code.

**Deploy** (maintainer, once): Cloudflare Pages, as in `website/README.md`, but with the build
command `python3 plugins/build_catalog.py website/plugins` so the catalogue publishes too.
Output directory: `website`. Then add the custom domains. Once the repo is public, GitHub Pages
also works for free; either is fine.

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
  - List platforms with honest status: Windows tested; Linux tested in CI and the cloud;
    macOS and iOS not yet.
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
always billed, so don't use them. The account currently shows "recent account payments have
failed"; check Settings → Billing that nothing still blocks Actions after the switch.

Before flipping visibility, work through `docs/launch-checklist.md`. The open items from it:
- Decide on the old `v0.1.0` tag. It exists only in the cloud clone, on an early commit, and
  was never pushed: delete it there, or release as v0.2.0.
- Delete the `ci/e2e-*` branches, or move E2E screenshots and logs to workflow artifacts.
- Run gitleaks one more time over the full history.
- Add `publish = false` to the `perritodeck-*` crates, then set `wildcards = "deny"` in
  `host/deny.toml`.
- In CI: run `cargo deny` and `scripts/third-party-notices.py --check`, and ship
  `third-party-licenses.txt` with release assets.
- Trim `CLAUDE.md` and `docs/HANDOFF.md` for a public audience, or keep them as contributor
  notes.
- `plugins/LICENSE` says "The PerritoDeck Authors"; everything else says "PerritoDeck
  contributors". Pick one.
- An open-source licences screen in the Android and desktop apps.

After it's public:
- Restore push and PR triggers for Linux, macOS and the other emulators, which are weekly or
  manual today because of cost.
- Run the iOS workflow on `macos-latest`. That's the first iOS build, with no rented Mac
  needed.
- Update CLAUDE.md's CI section to match.
