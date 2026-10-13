# Handoff: Windows bring-up on real hardware (local session, 2026-10-12)

> **Next:** the prioritised task list is in [`next-steps.md`](next-steps.md).

Everything is on `main`. This session took the ~40 commits that had never been compiled for
Windows (CI is paused by billing) and ran them on a Windows 11 Pro PC (build 26200) with an
Amazon Fire 7 (2019, KFMUWI, Fire OS 7.3.1.7 = Android 9 / API 28) on USB and Wi-Fi.
The Windows review (`docs/windows-review-2026-10.md`) has the per-finding status.

**Short version:** the host, the desktop app and Perrito all build and pass their tests on
Windows; every item of the review's manual test plan was run on the real machine and tablet
(except a UAC prompt, which needs a person); everything that broke was fixed, reviewed and
re-verified on the hardware. What's left is listed under "Still open".

## 1. Builds and tests on Windows

| Check | Result |
|---|---|
| `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings` (host) | clean on the first try; the never-compiled Windows code built as written |
| `cargo test` (host) | all pass (229 at the start of the session, more now); the plugin-removal test now runs on Windows too |
| `npm ci`, `npm run check`, `npx playwright test` (host/app) | 0 errors/warnings; all Playwright tests pass (use `CI=1` if another checkout's dev server may be on port 5174) |
| `npx tauri dev`, `npx tauri build --no-bundle` | both build and run; no installers were built (NSIS/WiX tools not downloaded) |
| `.\gradlew.bat testDebugUnitTest assembleDebug` (android, JDK 17) | all unit tests pass; AGP installs Platform 36 and Build-Tools 35 on first build |
| `py -3 plugins\test_all.py` | all plugin tests pass on Windows (obs, screenshot, windows and spotify needed Windows fixes) |

Toolchain on this PC: Rust 1.97.0 (pinned), MSVC 2022 Build Tools 14.44, Node 26, JDK 17
(Zulu; `JAVA_HOME` pointed at Android Studio's JBR 21, so the Gradle runs set it to 17),
Python 3.14 via `py -3`, Android SDK with adb 37 (not on PATH; the host finds it in
`%LOCALAPPDATA%\Android\Sdk`), WebView2 154.

## 2. Hardware test results (review §"Windows manual test plan")

- **Pairing.** USB: works ("Pair with a code" + the USB preset; QR pairing over the cable now
  works too, since Perrito tries 127.0.0.1). Wi-Fi: QR link pairs in under 2.5 s over the
  default-route address; mDNS discovery works on Fire OS. The QR lists `192.168.x` first,
  then the WSL/Hyper-V `vEthernet` addresses; link-local ones are now left out. Round trip:
  6–18 ms over USB, ~105 ms over Wi-Fi on the Fire 7.
- **Firewall.** The network here is classed **Public**. The first bind of each exe path
  (debug and release builds are different paths) brings up the Defender prompt; it was
  answered with Public allowed, creating TCP+UDP Allow rules for that exe only. Until it's
  answered the tablet falls back to the USB tunnel. Finding 6 stays open by request.
- **Console windows (release build):** none in 100 s of adb polling or from a Command key.
- **Hotkeys and text:** after the fixes, Notepad and a classic edit control get exactly what
  was sent (`a\tb\nc`, CRLF, `|+?!&é€`); `ctrl+plus` sends Ctrl + the =/+ key.
  Before: Windows 11 Notepad dropped and repeated characters (it needs ≥10 ms between
  them), and Tab/newline came out twice in Win32 apps.
- **Launch:** `code` starts VS Code with no console flash; `cmd` gets a working console;
  OBS by full path starts in its folder (no locale error); a quoted "Copy as path" path works.
- **Auto-switch:** a rule typed `charmap.exe` matches; taskbar, desktop, Start/Search and
  Task View keep the profile; another app or closing the app switches back. Not tried: a UAC
  prompt (needs a person to answer it).
- **Python plugins** (soundboard, audio, windows, screenshot, clipboard, sysmon, media): all
  run under `python.exe` directly (no `py.exe`), all actions and live values work; an app the
  windows plugin launched survives a plugin restart; the clipboard is restored after a paste;
  sysmon's network rates match Windows' counters.
- **Stream Deck plugins:** Elgato's open-source samples (hello-world prebuilt, counter built
  from source) install, run under the system Node, draw their keys, show their property
  inspector, survive remove/reinstall while running, and Restart no longer aborts the app.
- **Quit:** with 7 Python plugins, 2 Stream Deck plugins and a sound playing, Quit takes
  ~4 s and leaves no process behind.
- **MCP:** `perritodeck-app mcp` (GUI-subsystem exe) works as a stdio bridge; added to Claude
  Code with the command from Settings → AI assistants (in Git Bash), `claude mcp get` showed
  Connected and a `list_tablets` call through Claude Code answered. The user-scope entry was
  removed again afterwards so other sessions don't start the bridge. Since next-steps item 1,
  assistants start `perritodeck-mcp` from the data folder instead; `perritodeck-app mcp`
  still works.
- **Diagnostics:** Copy diagnostics and the logs folder work; the report now also says
  whether Windows classes the network Public or Private.

## 3. Fixed in this session (highlights; see `git log` and `docs/DECISIONS.md`)

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
  `docs/launch-checklist.md` §7. MSI not built.
- A live value with no data yet shows its raw token in small type on the key
  (e.g. `(plugin:dev.perritodeck.sysmon:disk)`). Deliberate ("unknown names are left as
  written", so typos show), but a declared value that hasn't reported yet could show "–".
- Editor nits: a two-line plugin title over an image is clipped at the key's bottom edge
  (hello-world's "Dynamic"); a key whose plugin draws only an image still reads "Untitled"
  to a screen reader.
- With the CLI host on one specific `--listen` address, the USB forwarder still runs and
  Settings still offers USB, although adb reverse then points at a closed port.
- `actions` test `a_command_that_starts_a_program_finishes_when_it_exits` has a 1.5 s limit
  that a heavily loaded machine overshot once (1.73 s).
- ~~Node plugins can't start programs outside their job.~~ Fixed (next-steps item 3): the
  plugin protocol's `open` and Node SDK 1.1.0's `openForUser`/`startForUser`. Programs
  Stream Deck plugins start with `child_process` still end with them (documented in
  phase-7). A Node plugin doesn't exit by itself on `shutdown` (the host ends it after
  about 2 s).
- ~~The MCP bridge is the app exe, so a connected assistant locks it.~~ Fixed (next-steps
  item 1): assistants start `perritodeck-mcp` from `<data dir>/bin`; verified with Claude Code
  connected across a quit, rebuild and restart. The updated real-app e2e still needs a Linux
  run.
- Linux and macOS have not been compiled since these changes (only Windows here); the weekly
  CI or a local build should check them first. Reviewers flagged the Linux text paths
  (`open_for_user`'s `xdg-open`, X11 hotkeys with shifted symbols) as the risky parts.
- iOS: never built (needs a Mac).
- Wi-Fi latency on the Fire 7 is ~105 ms vs 6–18 ms on USB; worth a look on other tablets.
- More Kindles are on the way from the maintainer for a hardware matrix and beta testers;
  older Fire models run Fire OS 5 (API 22), which is why minSdk stays 22.

## 5. Testing on Windows without a person at the keyboard

What worked in this session (none of it is committed; recreate as needed):

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
- **Accessibility checks:** use Narrator, Inspect, Accessibility Insights or MSAA. PowerShell's
  `System.Windows.Automation` shows every Win32 popup menu as an empty pane.

## 6. Gotchas learned

- **Running from the Claude desktop app (Code tab) on Windows:** everything it starts runs
  inside its MSIX package. AppData and HKCU writes go to
  `%LOCALAPPDATA%\Packages\Claude_<id>\LocalCache`, and inside the package that copy hides
  the real one. A PerritoDeck started there keeps its data where a normal start won't
  look; diagnostics now say "really at …" and the log warns. An installer started there
  installs into that folder too, and Windows Firewall then names the redirected path. To
  test for real, start programs through Explorer with a `.cmd` launcher in
  `C:\Users\Public`. To read the real AppData or registry, run a `.cmd` through WMI
  (`Win32_Process.Create`), which starts outside the package.
- NSIS wizards can't be read by .NET UI Automation. Use Win32 child windows, and click by
  posting WM_COMMAND with the button's id to the dialog (a posted BM_CLICK is ignored on
  the Finish page).
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

The earlier sections of this file (cloud session, 2026-10-11) are in git history; their "next
up" items that are still open: Companion and Stream Deck plugins with the real apps beyond the
samples, the new service plugins (OBS, Hue, Key Light, …) against real services, the first iOS
build, and Phase 6 (packaging, v0.1.0) when the maintainer says go.
