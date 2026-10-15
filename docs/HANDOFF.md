# Windows bring-up notes (2026-10)

> **Next:** the prioritised task list is in [`next-steps.md`](next-steps.md).

About 40 commits, much of the Windows-only code among them, had landed without being
compiled for Windows. On 2026-10-12 they were built and run on Windows 11 with an Amazon
Fire 7 (2019, KFMUWI, Fire OS 7.3.1.7 = Android 9 / API 28) on USB and Wi-Fi. This file
keeps what that bring-up found and how to test on Windows without a person at the
keyboard. The Windows review (`docs/windows-review-2026-10.md`) has the per-finding status.

**Short version:** the host, the desktop app and Perrito all build and pass their tests on
Windows. The review's manual test plan was run on a real PC and tablet, and everything that
broke was fixed, reviewed and re-verified on the hardware. What's left is listed under
"Still open".

## 1. Builds and tests on Windows

| Check | Result |
|---|---|
| `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings` (host) | clean on the first try; the never-compiled Windows code built as written |
| `cargo test` (host) | all pass; the plugin-removal test runs on Windows too |
| `npm ci`, `npm run check`, `npx playwright test` (host/app) | 0 errors/warnings; all Playwright tests pass (use `CI=1` if another checkout's dev server may be on port 5174) |
| `npx tauri dev`, `npx tauri build --no-bundle` | both build and run; `npx tauri build` with bundling fetches Tauri's NSIS and WiX tools the first time |
| `.\gradlew.bat testDebugUnitTest assembleDebug` (android, JDK 17) | all unit tests pass; AGP installs Platform 36 and Build-Tools 35 on first build |
| `py -3 plugins\test_all.py` | all plugin tests pass on Windows (obs, screenshot, windows and spotify needed Windows fixes) |

Tested with Rust 1.97.0 (pinned), MSVC 2022 Build Tools 14.44, Node 26, JDK 17, Python 3.14
via `py -3`, and WebView2 154. Two things to know:

- Gradle needs JDK 17. If `JAVA_HOME` points at Android Studio's bundled JBR 21, point it
  at a JDK 17 for the Gradle run.
- adb doesn't have to be on PATH: the host also looks in `%LOCALAPPDATA%\Android\Sdk`.

## 2. Hardware test results (review §"Windows manual test plan")

- **Pairing.** USB: works ("Pair with a code" + the USB preset; QR pairing over the cable
  works too, since Perrito tries 127.0.0.1). Wi-Fi: QR link pairs in under 2.5 s over the
  default-route address; mDNS discovery works on Fire OS. The QR lists the LAN address
  first, then any WSL/Hyper-V `vEthernet` addresses; link-local ones are left out. Round
  trip: a median ~12 ms over USB and Wi-Fi alike on the Fire 7 (next-steps item 6). An
  earlier "~105 ms over Wi-Fi" included Perrito's own main-thread time.
- **Firewall.** The first bind of each exe path (debug and release builds are different
  paths) brings up the Defender prompt. Answering it creates TCP and UDP rules for that exe
  only; on a network Windows classes as Public, the default answer lets nothing in. Until
  it's answered, the tablet falls back to the USB tunnel. Finding 6 stays open by request.
- **Console windows (release build):** none in 100 s of adb polling or from a Command key.
- **Hotkeys and text:** after the fixes, Notepad and a classic edit control get exactly what
  was sent (`a\tb\nc`, CRLF, `|+?!&é€`); `ctrl+plus` sends Ctrl + the =/+ key.
  Before: Windows 11 Notepad dropped and repeated characters (it needs ≥10 ms between
  them), and Tab/newline came out twice in Win32 apps.
- **Launch:** `code` starts VS Code with no console flash; `cmd` gets a working console;
  OBS by full path starts in its folder (no locale error); a quoted "Copy as path" path works.
- **Auto-switch:** a rule typed `charmap.exe` matches; taskbar, desktop, Start/Search and
  Task View keep the profile; another app or closing the app switches back. A UAC prompt
  keeps the profile too (verified 2026-10-13, see "Still open").
- **Python plugins** (soundboard, audio, windows, screenshot, clipboard, sysmon, media): all
  run under `python.exe` directly (no `py.exe`), all actions and live values work; an app the
  windows plugin launched survives a plugin restart; the clipboard is restored after a paste;
  sysmon's network rates match Windows' counters.
- **Stream Deck plugins:** Elgato's open-source samples (hello-world prebuilt, counter built
  from source) install, run under the system Node, draw their keys, show their property
  inspector, survive remove/reinstall while running, and Restart no longer aborts the app.
- **Quit:** with 7 Python plugins, 2 Stream Deck plugins and a sound playing, Quit takes
  ~4 s and leaves no process behind.
- **MCP:** `perritodeck-app mcp` (GUI-subsystem exe) works as a stdio bridge. Added to
  Claude Code with the command from Settings → AI assistants (in Git Bash), `claude mcp get`
  shows Connected and a `list_tablets` call answers. A user-scope entry starts the bridge in
  every assistant session, so remove a test entry afterwards. Since next-steps item 1,
  assistants start `perritodeck-mcp` from the data folder instead; `perritodeck-app mcp`
  still works.
- **Diagnostics:** Copy diagnostics and the logs folder work; the report also says whether
  Windows classes the network Public or Private.

## 3. What the bring-up fixed (highlights; see `git log` and `docs/DECISIONS.md`)

- Stream Deck plugin **Restart aborted the app** (bare `tokio::spawn` on the UI thread).
  Plugin commands that wait now run off the UI thread.
- **Text action** paced on Windows; enigo no longer types Tab/newline twice; extended keys
  get the extended flag; hotkeys hold the key 50 ms, press letters by virtual key, add the
  Shift/AltGr a symbol needs, accept `Ctrl + +` and punctuation names; Win+L locks.
- **Launch:** console programs get a working window, PATH `.cmd` shims run hidden, URIs and
  elevated programs go through ShellExecute (UAC), quotes are stripped; Command waits for the
  process and a timeout ends what it started; OpenPath of a missing file no longer shows a
  blocking dialog.
- **Plugin processes:** jobs allow explicit breakaway only, so programs a plugin opens for
  the user (browser, an app from a Focus key) outlive it while its own children don't; Python
  runs directly; plugins start suspended until they're in their job; removal waits for the
  whole tree. SDK 1.3.0: `open_for_user`, `start_for_user`.
- **USB:** lost or failed `adb reverse` is re-applied; adb calls time out; Settings says why
  a tablet isn't connected.
- **Pairing:** Perrito tries 127.0.0.1 first (QR and code pairing, reconnects); addresses are
  read fresh; mDNS is IPv4 only; inverted QR codes scan; plain-language connection errors
  (the tablet used to show "SSL handshake aborted: ssl=0x…: I/O error during system call"
  when the computer's app wasn't running with a cable in).
- **Orientation:** Perrito turns to the grid's shape (landscape for the starter 5×3), matching
  the desktop editor; key images are sized for that orientation.
- **Auto-switch:** shell windows (ShellHost, widgets, Snipping Tool, hung-app stand-ins, error
  boxes…) keep the last app; Store apps are named; `.exe` in rules matches.
- **MCP:** the Claude Code command works in Git Bash, PowerShell and cmd and in every folder
  (`--scope user`); the bridge follows a new token or port.
- Plugins: PowerShell values passed in environment variables (typographic quotes broke
  scripts), readable PowerShell errors, sysmon network rates, media key presses no longer wait
  for a poll, Spotify sign-in port on Windows, the clipboard plugin in-process.
- App: the Vite dev server doesn't watch `src-tauri`; "Restart now" keeps the window when
  started at login; a second launch no longer touches the running app's log.
- **Editor shows what Stream Deck plugins draw** (it showed blank "Untitled" keys while the
  tablet showed the plugin's image), refreshed as the plugin redraws.
- Second round: text actions capped at 2,000 characters (the editor field stops there);
  Record on Linux keeps shifted punctuation typeable; code pairing also tries the cable;
  Node.js installed while the app runs is found; Quit waits for Stream Deck plugins too;
  a scan can't restart a plugin being removed; programs opened by the gitlab, sysmon and
  screenshot plugins outlive them; wireless-adb tablets get their own advice; the deck stays
  full screen after the lock screen or notification shade; tray menu access keys; every
  first-party plugin got a patch version for the SDK change.

## 4. Still open

- ~~UAC prompt during auto-switch~~: verified 2026-10-13 (next-steps item 4); the profile
  stays through the prompt and follows the elevated app afterwards.
- **Firewall (finding 6)**: each exe path gets its own prompt; on a Public network the
  default answer lets nothing in. Detection is done (next-steps item 2): the Tablets and Pair
  pages say when a tablet fell back to the cable or Windows Firewall has no rule for
  PerritoDeck. Installers add no rule (maintainer's decision).
- ~~Installers, autostart at login, uninstall~~: the NSIS installer was tested on
  2026-10-13 (next-steps item 5), including an upgrade with an assistant connected. A bug
  was fixed (an update turned "Start at login" off), and the release notes are in
  `docs/launch-checklist.md` §7. The MSI was built once, in CI on 2026-10-11, but has
  never been installed or tested, and it's stale since then (decision D5 in
  `docs/plan/phase-8.md`: test it or drop it from the first release).
- ~~Raw tokens for live values not reported yet, editor title clipping and image-only key
  labels, USB offered on a specific `--listen` address, a tight test limit~~: fixed
  2026-10-13 (next-steps item 6).
- ~~Node plugins can't start programs outside their job.~~ Fixed (next-steps item 3): the
  plugin protocol's `open` and Node SDK 1.1.0's `openForUser`/`startForUser`. Programs
  Stream Deck plugins start with `child_process` still end with them (documented in
  phase-7). A Node plugin doesn't exit by itself on `shutdown` (the host ends it after
  about 2 s).
- ~~The MCP bridge is the app exe, so a connected assistant locks it.~~ Fixed (next-steps
  item 1): assistants start `perritodeck-mcp` from `<data dir>/bin`; verified with Claude Code
  connected across a quit, rebuild and restart. The updated real-app e2e still needs a Linux
  run.
- **Linux** was checked after the bring-up, on 2026-10-12 on a headless Linux machine:
  host fmt, clippy and tests, the app's check and Playwright tests, plugins, protocol and
  the real-app e2e (its update for the separate MCP bridge, above, hasn't had a Linux run
  yet). It hasn't been compiled since the 2026-10-13 commits, and has never run on a real
  Linux desktop with a tablet. Reviewers flagged the Linux text paths (`open_for_user`'s
  `xdg-open`, X11 hotkeys with shifted symbols) as the risky parts.
- **macOS** was last built in CI on 2026-10-11, before the bring-up, and has never run on
  a real Mac.
- iOS: never built (needs a Mac).
- ~~Wi-Fi latency on the Fire 7~~: profiled 2026-10-13 (next-steps item 6). Key presses take
  the same ~12 ms on Wi-Fi as on the cable. Messages the computer starts can wait 0.3–0.9 s
  for the dozing Fire's Wi-Fi. A page switch holds Perrito's main thread for up to ~0.5 s;
  next-steps item 6 has the numbers and the suggested fix.
- Older Fire models run Fire OS 5 (API 22), which is why minSdk stays 22; only the Fire 7
  above has been tried on real hardware so far.

## 5. Testing on Windows without a person at the keyboard

None of these helpers is committed; recreate them as needed.

- **Drive the real app:** start it with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333`
  and attach Playwright with `chromium.connectOverCDP("http://127.0.0.1:9333")`; Tauri
  commands can be called with `window.__TAURI_INTERNALS__.invoke(...)`, e.g. `test_action`
  to run any action inside the GUI-subsystem app. Playwright emulates a light colour scheme
  while attached, so a dark-mode app flashes light on every check: call
  `page.emulateMedia({ colorScheme: null })` right after attaching.
- **Drive the tablet:** `adb -s <serial> exec-out screencap -p`, `uiautomator dump`, `input tap`;
  `svc power stayon usb` keeps it awake (set back with `svc power stayon false`). Quote `&`
  in URLs for `adb shell` (it goes through the tablet's `sh`); uiautomator reports unpanned
  coordinates while the keyboard is up.
- **Catch console flashes:** a `SetWinEventHook(EVENT_OBJECT_SHOW / EVENT_SYSTEM_FOREGROUND)`
  logger sees every window shown, even for 50 ms.
- **Native file dialog:** `WM_SETTEXT` on the File name edit (id 1148) and `BM_CLICK` on Open
  (id 1).
- **NSIS wizards** can't be read by .NET UI Automation. Use Win32 child windows, and click by
  posting WM_COMMAND with the button's id to the dialog (a posted BM_CLICK is ignored on
  the Finish page).
- **Start something outside a packaged app:** `Win32_Process.Create` through WMI starts a
  program outside the caller's MSIX package (see the first gotcha below).
- **Accessibility checks:** use Narrator, Inspect, Accessibility Insights or MSAA. PowerShell's
  `System.Windows.Automation` shows every Win32 popup menu as an empty pane.

## 6. Gotchas learned

- **Starting PerritoDeck from inside a packaged (MSIX) Windows app**, such as a terminal
  built into another packaged desktop app: everything it starts runs inside that package.
  AppData and HKCU writes go to `%LOCALAPPDATA%\Packages\<package>\LocalCache`, and inside
  the package that copy hides the real one. A PerritoDeck started there keeps its data where
  a normal start won't look; diagnostics say "really at …" and the log warns. An installer
  started there installs into that folder too, and Windows Firewall then names the
  redirected path. To test for real, start programs from Explorer or through WMI, which run
  outside the package; the same goes for reading the real AppData or registry.
- Some shells set `NoDefaultCurrentDirectoryInExePath=1`: run `.\gradlew.bat`, not `gradlew.bat`.
- Windows PowerShell 5.1 strips inner double quotes from native programs' arguments, so
  `perritodeck-host action '{"type":...}'` needs cmd, PowerShell 7.3+ or bash.
- `npx tauri dev` builds with `--no-default-features`, separately from a plain `cargo build`.
- With adb reverse in place and no host running, the tablet's connection is accepted by adb on
  the tablet and then reset during TLS; Perrito now says "PerritoDeck isn't running on the
  computer this tablet is plugged into".
- Windows 11 Notepad loses `KEYEVENTF_UNICODE` input that arrives faster than ~10 ms apart.
- A refused TCP connect to a closed localhost port takes ~2 s on Windows (SYN retries); tests
  with tight timeouts need room for it.
- `SO_REUSEADDR` on Windows lets two sockets bind the same port.

Earlier notes (2026-10-11) are in git history. Items from them that are still open:
Companion and Stream Deck plugins with the real apps beyond the samples, the service plugins
(OBS, Hue, Key Light, …) against real services, the first iOS build, and Phase 6 (packaging,
v0.1.0) when the maintainer says go.
