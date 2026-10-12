# Handoff: cloud session → local session (2026-10-11)

Everything is on `main`. On 2026-10-11 the cloud session also built most of Phases 3–5
(editor, profiles, switching, sliders, live values, USB, plugins, animated keys, online
plugin catalogue) and, pulled forward from Phase 7, **Bitfocus Companion mode** and
**Stream Deck plugin support**; see `docs/plan/phase-3.md` … `phase-7.md` for what's done
and what isn't. Phase 6 (packaging) is next after the hardware checks.

**CI is paused:** GitHub stopped starting jobs (spending limit / payment) at about 09:30.
The last green CI was before the sliders work. Host and desktop-app changes since then
were tested in the cloud session on Linux (Rust tests, Playwright, the real app under
Xvfb); **the Android changes since then (sliders, USB fallback, alert haptics) have never
been compiled.** Run the Android build and unit tests locally first (section 1), and fix
anything before the Kindle check. Windows and macOS code (focus detection, volume) has
only been compiled for Linux since then too: `cargo test` in `host\` on Windows checks it.

**Later on 2026-10-11 (cloud session, not committed yet):** debugging aids and the MCP
server were wired in. Tested on Linux only (Rust tests, Playwright, the real app under Xvfb,
and a manual CLI run piping JSON-RPC through `perritodeck-host mcp`):

- **Logs:** `logs/` in the data folder, rotating daily or at 1.4 MB, seven files kept,
  everything redacted (`host/crates/host/src/logging.rs`). Settings → Getting help has
  "Copy diagnostics", "Open logs folder" and "Detailed logs" (live). CLI:
  `perritodeck-host diagnostics`; `paths` shows the logs folder and MCP token file.
- **MCP:** Settings → "AI assistants (MCP)" turns on the local server (127.0.0.1:46711,
  off by default, press/edit/run permissions off by default) and shows copyable snippets.
  Assistants launch `perritodeck-host mcp` or `perritodeck-app mcp` (the app handles that
  argument before Tauri starts), a stdio bridge to the running host. `mcp --config`
  prints the snippets. New: `Deck::press_at`, `Host::apply_mcp/set_mcp/new_mcp_token`.
- **Untested:** Windows and macOS for all of it. Worth checking on Windows: the app
  binary as a stdio bridge (it's a GUI-subsystem exe in release builds; stdio should work
  when the assistant pipes it, but try it with Claude Desktop), the log folder opening,
  and log rotation while the file is open (the writer closes it before renaming). The
  Claude Desktop/Code wording in the website guide wasn't checked against their current docs.

**Windows review fixes (cloud session, not committed yet):** most findings in
`docs/windows-review-2026-10.md` are fixed (see its status table), written and tested on
Linux only, because the Windows Rust target couldn't be downloaded in the cloud session.
`cargo clippy --all-targets -- -D warnings` and `cargo test` in `host\` on Windows are
the first check; the Windows-only code is in `actions/src/launch.rs`, `actions/src/job.rs`,
`host/src/focus.rs` and `host/src/usb.rs`, plus `plugins/soundboard/winmm.py`.

## 1. Set up the Windows clone (`C:\perritodeck\perritodeck`)

In PowerShell:

```powershell
cd C:\perritodeck\perritodeck
git pull

# Commit identity for this repo only; keeps personal details out of history.
git config user.name "dmang-dev"
git config user.email "dev@perritodeck.dev"

# The repo now pins LF line endings (.gitattributes). Re-checkout so existing files match.
git rm -r --cached -q .
git reset --hard
git status   # should be clean
```

Then install the toolchains. Skip any you already have:

- **Rust:** `winget install Rustlang.Rustup`. The pinned 1.97.0 toolchain installs itself
  on the first `cargo` command in `host\`. It needs the MSVC build tools:
  `winget install Microsoft.VisualStudio.2022.BuildTools`, then add the "Desktop development
  with C++" workload.
- **Node 22:** `winget install OpenJS.NodeJS.LTS` (desktop app UI).
- **Android:** `winget install Google.AndroidStudio`. Open `android\` once so it installs
  SDK 36. Gradle needs JDK 17, which Android Studio bundles. If `adb` isn't found, add
  `%LOCALAPPDATA%\Android\Sdk\platform-tools` to PATH.
- The WebView2 runtime the desktop app needs is already part of Windows 10 and 11.

Smoke test:

```powershell
cd host; cargo test; cd ..
cd host\app; npm ci; npx playwright install chromium; npx playwright test; cd ..\..
cd android; .\gradlew.bat testDebugUnitTest assembleDebug; cd ..
```

Two things haven't been tried on Windows yet: the desktop app running for real, and
hotkeys via enigo on Windows. Both are worth a first look with `cd host\app; npx tauri dev`.
Windows Firewall will ask about port 46710 the first time; allow it on private networks.

The release signing key (sent separately as a file) stays **outside** the repo. It's only
needed for the v0.1.0 release, which is on hold.

## 2. Next up, in order

1. **Kindle hardware check.** This is the reason for going local.
   - On the Kindle: Settings → Device Options → tap Serial Number 7 times, then Developer
     Options → enable ADB. Connect it over USB and accept the prompt.
   - The tablet is a **KFMUWI = Fire 7 (2019)**: 1024×600 mdpi, 1 GB RAM, cameras front
     and back (so in-app QR scanning should work). It shipped with Fire OS 6 (API 25) and
     updates to Fire OS 7 (API 28).
   - `adb shell getprop ro.build.version.sdk` gives the API level. 22 is Fire OS 5 and 25
     is Fire OS 6. 28 is Fire OS 7 and 30 is Fire OS 8.
   - `cd android; .\gradlew.bat installDebug`, then pair with the host running on the
     laptop (`cd host\app; npx tauri dev`, or `cargo run -p perritodeck-host -- --pair`).
     QR or code both work; this Fire has a rear camera.
   - Over USB: with adb installed, the host forwards its port to the tablet by itself and
     Perrito falls back to 127.0.0.1, so it should keep working with Wi-Fi off. Try it.
   - The starter profile's "More" page has a Volume slider: drag it (system volume on
     Windows) and check the feel and the haptic ticks.
   - Record the result in `docs/DECISIONS.md`. If it's API 23 or higher, raise minSdk and
     unpin AndroidX/Compose (see the 2026-10-11 decision about minSdk 22).
   - Check for: layout fitting the screen, key-image sharpness, latency (`fakedeck bench`
     gives a baseline), mDNS discovery on Fire OS, and reconnection after the screen sleeps.
2. **Windows check of the desktop app:** editor, profiles, Switching (focus detection on
   Windows), Settings → start at login, network port and "Restart now", Plugins → add HTTP
   requests (needs Python on the laptop), a Volume slider, tray Pause keys, an animated
   GIF key, and the Now playing plugin (its Windows part has never run on Windows).
3. **Companion and Stream Deck plugins with the real things** (both only tested against
   fakes so far):
   - Companion: install Bitfocus Companion, add `[companion]` to `integrations.toml`
     (Settings → Open folder), restart, make a profile with "Show Bitfocus Companion's
     buttons" and show it on the tablet. The tablet should appear under Companion's
     Surfaces.
   - Stream Deck: needs Node.js (`winget install OpenJS.NodeJS.LTS`). Download a free
     Node-based plugin from the Elgato Marketplace (a `.streamDeckPlugin` file) and add it
     on the Plugins page; put one of its actions on a key.
4. **The new plugins with the real services.** Thirteen first-party plugins ship in the app
   (Plugins → "Comes with PerritoDeck"); the new ones (OBS Studio, Timers, Counters, Is it
   up?, Wake-on-LAN, Weather, Ollama, Proxmox VE, TrueNAS, GitLab) have only met fake
   servers. Each needs its settings filled in on the Plugins page (Settings button), and
   titles pick what to show with the key editor's "Insert a live value". Each plugin's
   README lists what's untested.
5. **First iOS build.** `ios/` has never been compiled. Cheapest way: rent a Scaleway
   Mac mini M2 for a day (about €4; 24-hour minimum), open its desktop over VNC, install
   Xcode and the Claude desktop app, open this conversation there and choose "Link to
   this computer". Or run the `iOS` workflow by hand once Actions billing works.
6. **Leftovers in the phase docs** ("Not done" lists): e.g. Stream Deck property
   inspectors, Wayland auto-switch, publishing the plugin catalogue on the website.
7. **Phase 6 / v0.1.0 release** when the maintainer says go. Set the signing secrets, tag, and watch
   the untested `release.yml`. See `docs/releasing.md`.

## 3. Open questions for the maintainer

- The Kindle's Fire OS version decides minSdk.
- Phase 3 open questions are listed at the bottom of `docs/plan/phase-3.md`: editing on the
  tablet, theme, crash reports.
- When to make the repo public.

## 4. Gotchas learned so far

- The host now runs plugins (`<settings folder>\plugins\<id>\`) and Stream Deck plugins
  (`<settings folder>\sdplugins\`), forwards its port to USB tablets when it finds adb,
  and watches which app is in front (for Switching). `--no-usb` turns the USB part off
  for the command-line host.
- `perritodeck-sdcompat`'s test runs a Node.js fixture plugin; it's skipped when Node
  is missing or older than 22 (no built-in WebSocket).
- The app's network settings (port, Wi-Fi) apply only to the app; the command-line host
  keeps its `--port`/`--listen` flags.
- CI on pushes is deliberately small (see `CLAUDE.md`); run the bigger workflows by hand.

- `host/vendor/enigo` is a patched copy of enigo 0.6.1 (see its `PERRITODECK.md`).
  `host/app/src-tauri` is a separate Cargo workspace and repeats the
  `[patch.crates-io]` entry, so keep the two in sync.
- The Android unit test `DeckConnectionTest` used to flake on MockWebServer shutdown. The
  fake host now answers the close handshake. If it flakes again, look there first.
- Log output goes through `logging::redact`; a field whose name ends in `token`,
  `secret`, `password`… has its value hidden, so don't name ordinary fields that way
  (diagnostics says "token file", not "token").
- `perritodeck-host mcp` must never print anything but protocol on stdout; logging is on
  stderr at `warn` there.
- The E2E test pairs with the fixed code `424242`, using the hidden `--pair-code` host flag
  (testing only).
- Android UI tests find keys by accessibility label, not by visible text, because keys are
  host-rendered images.
