# PerritoDeck: review of the Windows-only code paths

Read-only review of HEAD `cd63368`, plus a look at the uncommitted logging and MCP work in the tree. Nothing was built or run. Each finding below was checked against the code, and against Microsoft docs or upstream source where noted. Findings that couldn't be confirmed are marked **PLAUSIBLE: verify** or moved to the test plan.

Severity scale: **crash**, **broken** (feature broken), **degraded** (works badly or slowly), **cosmetic**.

## Status (2026-10-11, cloud session)

Fixes were written and tested on Linux only: the `x86_64-pc-windows-gnu` target couldn't be
downloaded there, so **none of the Windows-only code has been compiled yet**. Signatures
were checked against the `windows` 0.61.3, `windows-sys` 0.61.2 and `tokio` 1.53.2 sources.
Run `cargo clippy --all-targets -- -D warnings` and `cargo test` in `host\` first, then the
test plan below.

| # | Finding | Status |
|---|---|---|
| 1 | adb console window every 4 s | **Fixed**: `CREATE_NO_WINDOW` and null stdin (`usb.rs`, `adb_command`) |
| 2 | "command" actions open a console | **Fixed**: `CREATE_NO_WINDOW` in `run_command`; Launch unchanged |
| 3 | Shifted/AltGr hotkeys type nothing | **Fixed**: `& 0x00FF` in vendored enigo, noted in `vendor/enigo/PERRITODECK.md` |
| 4 | Launch: PATHEXT, working folder | **Fixed**: PATHEXT retry (Launch and Command), Launch starts in the program's folder, `ShellExecuteW` fallback on NotFound / `ERROR_BAD_EXE_FORMAT` (`actions/src/launch.rs`) |
| 5 | Store `python.exe` stub restart loop | **Fixed**: exit 9009 within 5 s → "needs Python 3"; `has_python()` needs a successful `--version` (`py -3` first, as before) |
| 6 | Defender Firewall | **Open** (not attempted, by request) |
| 7 | `open` through PowerShell in the CLI | **Fixed**: `shellexecute-on-windows` feature (adds `dunce`) |
| 8 | Auto-switch flicker, Store apps | **Fixed**: NULL foreground and shell windows keep the last app; `ApplicationFrameHost` resolved through its child windows' process |
| 9 | Pairing slow with WSL/Hyper-V | **Fixed**: default-route address first (`discovery.rs`) |
| 10 | Media plugin polling, timeout, error text | **Fixed differently**: the host only sends `watch` for values with arguments, so the plugin can't know whether `status`/`title` is shown; on Windows it polls every 10 s after 30 s with no media key press (an action wakes it at once). Timeout 8 s; key errors are one line of at most 80 characters, and a timeout says "the player didn't answer in 8 s". Long-lived PowerShell not done |
| 11 | Screenshot DPI | **Fixed**: per-monitor-v2 first, `SetProcessDPIAware` as the fallback; screenshot errors shortened the same way |
| 12 | Soundboard latency | **Fixed**: in-process winmm MCI (`plugins/soundboard/winmm.py`), PowerShell as the fallback |
| 13 | Plugin process trees, Quit | **Fixed**: one job object per plugin process with `KILL_ON_JOB_CLOSE` (`actions/src/job.rs`, both plugin kinds); app stops plugins on `RunEvent::Exit`, CLI on Ctrl+C |
| 14 | Tab/newline twice | Open (verify first) |
| 15 | UIPI / elevated windows | **Documented** in the guide (Troubleshooting → Buttons do nothing); no elevation hint in the app yet |
| 16 | `write_private` ACL, Roaming | Open |
| 17 | Plugin `main` outside its folder | **Fixed**: leading `/` or `\` and any `:` refused on every platform |
| 18 | `platformVersion` | **Fixed**: `10.0.<build>` (sysinfo) on Windows, the macOS version on macOS |
| 19 | Tray left-click | **Open** (not attempted, by request) |
| 20 | MCP stdio bridge note | Open |

Things to check on Windows that these fixes add: the job-object assignment (look for "couldn't
put the process in a job object" in a detailed log), that `code` and `obs64` start from Launch
keys, that MCI plays MP3 and WAV at the set volume, and that clicking the taskbar no longer
flips the tablet to the default profile.

## Status after the Windows run (2026-10-12, local session on Windows 11 + Fire 7)

Everything compiled on Windows on the first try; the test plan below was run on a real PC and
tablet. "Verified" means seen working on the hardware, not just in tests.

| # | Finding | Status on Windows |
|---|---|---|
| 1 | adb console window every 4 s | **Verified fixed**: release build, 100 s of polling, no window |
| 2 | "command" actions open a console | **Verified fixed** (starter "Run a command" key) |
| 3 | Shifted/AltGr hotkeys type nothing | **Verified fixed**: `ctrl+plus` sends Ctrl + the =/+ key; symbols now also bring the Shift/AltGr their layout needs, letters/digits go by virtual key |
| 4 | Launch: PATHEXT, working folder | **Verified fixed**: `code`, OBS by full path (no locale error). Also new: console programs get a working window, PATH `.cmd` shims run hidden (no flash), URIs and elevation (740) go to ShellExecute, "Copy as path" quotes are stripped |
| 5 | Store `python.exe` stub restart loop | **Replaced**: the host asks `py -3`/`python` for the interpreter path and runs it directly; none → "needs Python 3". Store stub itself not tried (Python is installed here) |
| 6 | Defender Firewall | Seen: one prompt per exe path (debug and release differ); this network is Public; until answered, the tablet falls back to USB. Diagnostics now show Public/Private. **Detection done** (next-steps item 2): the app reads the firewall's rules and hears from tablets that fell back, and says what to change. Installers add no rule (maintainer's decision, 2026-10-12) |
| 7 | `open` through PowerShell in the CLI | Fixed; not timed |
| 8 | Auto-switch flicker, Store apps | **Verified fixed** for taskbar, desktop, Start/Search, Task View; more shell hosts added (ShellHost, widgets, Snipping Tool, hung-app stand-ins). **UAC prompt verified** (2026-10-13): the profile stays through the secure desktop and switches to the elevated app's profile afterwards |
| 9 | Pairing slow with WSL/Hyper-V | **Verified fixed**: default-route address first; link-local addresses now left out |
| 10 | Media plugin polling, timeout, error text | Fixed differently (as above); key presses no longer wait for a poll; status value verified |
| 11 | Screenshot DPI | Fixed; window capture verified (one monitor here, mixed DPI not tried) |
| 12 | Soundboard latency | Fixed (winmm); play/stop verified |
| 13 | Plugin process trees, Quit | **Verified fixed**: Quit with 9 plugins running leaves no process. Follow-up fix: programs a plugin opens for the user (browser, Focus-key apps) now outlive it (explicit breakaway) |
| 14 | Tab/newline twice | **Confirmed and fixed**: Win32 apps got both the key and the character; enigo now sends only the key, CRLF as one Return |
| 15 | UIPI / elevated windows | Documented; not tried |
| 16 | `write_private` ACL, Roaming | Open |
| 17 | Plugin `main` outside its folder | Fixed; not re-tested |
| 18 | `platformVersion` | Fixed; not checked against a plugin |
| 19 | Tray left-click | Open (by request) |
| 20 | MCP stdio bridge note | **Verified**: the GUI-subsystem exe works as a stdio bridge with Claude Code; the Claude Code command now quotes Windows paths and uses `--scope user`. **Exe lock fixed**: assistants now start `perritodeck-mcp.exe` from the data folder; with Claude Code connected, the app was quit, rebuilt and restarted (next-steps item 1) |

New problems found by running it (all fixed unless marked):

- Stream Deck plugin **Restart aborted the app** (`tokio::spawn` on the UI thread). Fixed.
- Windows 11 Notepad **dropped and repeated characters** from the text action (needs ≥10 ms
  between Unicode keystrokes). Fixed by pacing.
- Launch of a console program from the installed app opened an empty window that closed at
  once; `code` (a `.cmd` shim) flashed a Terminal window. Fixed.
- The job objects of #13 also killed programs a plugin started for the user. Fixed.
- `adb reverse` lost after an adb server restart, or failed once and never retried. Fixed.
- Pairing over USB from a QR code failed (Perrito never tried 127.0.0.1). Fixed.
- The tablet showed raw TLS exception text ("SSL handshake aborted … I/O error during system
  call") when the computer's app wasn't running with a cable in. Fixed (plain words).
- The tablet stayed portrait with the starter 5×3 grid while the editor drew it landscape.
  Fixed: Perrito turns to the grid's shape.
- Stream Deck plugin keys were blank in the editor (their drawing only reached tablets).
  Fixed: the editor shows the plugin's drawing and refreshes as it redraws.
- The deck lost full-screen after the lock screen or the notification shade (Android 9).
  Fixed.
- Checked and not a problem: the tray menu is a standard Win32 menu that screen readers read
  (PowerShell's .NET UI Automation client just can't see popup menus); it now has access keys.
- Plugin tests (obs, screenshot, windows) assumed Linux; the Spotify sign-in server could
  share its port on Windows (`SO_REUSEADDR`). Fixed.

---

## 0. Before anything else: CI has not built Windows for 41 commits

Every Actions job since 2026-10-11 16:19Z failed before starting, with the message "recent account payments have failed or your spending limit needs to be increased". The last green Windows Host run was at `9e698c8`. Since then about 19.6k lines have landed in `host/` that have **never been compiled for Windows**, including:

- the whole plugin runtime (`crates/plugins`)
- the Stream Deck host (`crates/sdcompat`)
- `crates/mcp`
- `usb.rs`
- the new `creation_flags` calls

So the first thing to do on the laptop is a local `cargo clippy --all-targets -- -D warnings` and `cargo test`.

---

## Top findings (fix or check first)

### 1. Console window flashes every 4 s in the installed app when adb is installed: **broken (very visible)**

**Status: fixed.**

`host/crates/host/src/usb.rs:67` and `:71` run `adb devices` every 4 s, and `adb reverse` for each new tablet, through `std::process::Command` with no creation flags.

The release app is a GUI-subsystem process (`app/src-tauri/src/main.rs:2`) with no console. Windows gives every console program it starts a new console window. With Windows Terminal set as the default terminal (the Windows 11 default), each one is a Terminal window. The result is a window popping up every 4 seconds for as long as the app runs.

`tauri dev` (a debug build with a console) and the CLI host share their console, so neither shows it. **You only see this in an installed or `tauri build` app.**

Fix:

```rust
let mut c = Command::new(&adb);
#[cfg(windows)]
{
    use std::os::windows::process::CommandExt;
    c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}
```

Apply it to both calls. Also consider `.stdin(Stdio::null())`.

### 2. "command" actions open a console window from the app: **broken/cosmetic**

**Status: fixed.**

`host/crates/actions/src/lib.rs:489` (`run_command`) captures output in the background but sets no `CREATE_NO_WINDOW`. Pressing the starter profile's "Run a command" key (`program = "hostname"`, `profile/src/starter.toml:131`) in the installed app flashes a console. `powershell` or `cmd /c` commands flash too.

- **Fix:** set `creation_flags(0x0800_0000)` in `run_command`. tokio's `Command` has an inherent `creation_flags` on Windows.
- **Don't** add it to `Launch` (`:333`). A launched console program should get a window.

### 3. Hotkeys whose key needs Shift or AltGr do nothing: **broken**

**Status: fixed** (vendored enigo).

In vendored enigo, `vendor/enigo/src/keycodes.rs:1063-1089`:

```rust
let vk = VkKeyScanExW(utf16_surrogates[0], layout);   // high byte = shift state
...
VIRTUAL_KEY(vk as u16)                                // keeps the high byte
```

`VkKeyScanExW` puts the virtual-key code in the low byte and the Shift/Ctrl/Alt state in the high byte (per the MS docs, and enigo's own comment says so). For `'+'` on a US layout it returns `0x01BB`.

`queue_key` (`win/win_impl.rs:428-470`) then:

1. calls `MapVirtualKeyExW(0x1BB, MAPVK_VK_TO_VSC_EX)`, which returns 0 (logged as "no mapping");
2. sends the key with `KEYEVENTF_SCANCODE` and scan code 0.

Nothing is typed. What breaks:

- `ctrl+plus`, `ctrl+shift+?` and the like on a US layout;
- on AZERTY, every digit (`ctrl+1`, because digits are shifted there);
- on German and other layouts, `/`, `-` and the other shifted punctuation.

Letters are fine, because the parser lowercases them (`hotkey.rs:132`). Upstream enigo `main` has the same code.

Fix (and note it in `vendor/enigo/PERRITODECK.md`):

```rust
VIRTUAL_KEY((vk as u16) & 0x00FF)
```

That makes `ctrl+plus` send Ctrl plus the `=`/`+` key, which is what browsers and editors treat as zoom-in.

### 4. Launch on Windows: `.cmd` shims aren't found and the working directory is wrong (OBS fails): **broken**

**Status: fixed** (PATHEXT, working folder, ShellExecute fallback).

`actions/src/lib.rs:332-346`: `Command::new(program)` with no `current_dir`.

- **PATHEXT:** Rust's Windows program search only appends `.exe`. So `code` (`code.cmd`), `npm`, `az` and similar commands return NotFound, and the same applies to the `command` action.
- **Working directory:** the launched program inherits the host's working directory (`%LOCALAPPDATA%\PerritoDeck` for the app). OBS on Windows must start in its `bin\64bit` folder. Launched from elsewhere it stops with **"Failed to find locale/en-US.ini"**, a well-known OBS problem. Other portable apps behave the same way.

Fix: on Windows, when `program` contains a path separator, call `cmd.current_dir(Path::new(program).parent())`, which matches Explorer's default "Start in". Two options for names:

- try `program.cmd` and `program.bat` through the PATH yourself, or
- fall back to `ShellExecuteExW`. This also resolves App Paths entries (`obs64`, `chrome`) and `.lnk` shortcuts, which is what users will type.

### 5. Without Python, the Store's `python.exe` stub causes a restart loop instead of "needs Python 3": **broken (misleading)**

**Status: fixed.**

`plugins/src/lib.rs:516-528` tries `py -3`, then `python`. On a stock Windows 10/11 machine without Python, `py` is NotFound, but `python` resolves to the App Execution Alias `%LOCALAPPDATA%\Microsoft\WindowsApps\python.exe`. That spawns, prints "Python was not found; run without arguments to install from the Microsoft Store…", and exits with code **9009**.

The supervisor (`:583-591`) only reports `MissingRuntime` when the spawn returns `NotFound`. Here the plugin is restarted with backoff and ends as `Crashed { "it exited (exit code: 9009)" }`. The test helper `has_python()` (`plugins/src/tests.rs:42-54`) is fooled the same way.

Fix: on Windows, treat exit code 9009 within the first couple of seconds as `MissingRuntime { "Python 3" }`. Or skip any candidate that resolves under `\Microsoft\WindowsApps\`.

### 6. Windows Defender Firewall: the first LAN bind prompts, and on Public networks or Cancel, tablets can't connect: **broken (common setup)**

**Status: open.**

The host binds `0.0.0.0:46710` (`host/src/lib.rs:80/89/407`) and mDNS on UDP 5353. The first bind from each exe path brings up the Defender Firewall dialog:

- Only "Private networks" is ticked by default. If the laptop's Wi-Fi is classed as **Public**, which is common on new networks, inbound stays blocked.
- **Cancel** makes Windows add *Block* rules for both TCP and UDP for that exe.

Either way tablets silently can't connect, and nothing in the app explains why. The NSIS `currentUser` installer can't add a rule (that needs admin). The MSI (perMachine) could, through the WiX firewall extension.

Suggestions:

- Detect it: no connection within N seconds of showing a pairing QR code, or check the network profile through `INetworkListManager`.
- Show a help panel: "Allow PerritoDeck through Windows Defender Firewall", with the `netsh advfirewall firewall add rule … program="<exe>" profile=private` line.
- USB via adb binds through loopback and is not affected. "Wi-Fi off" (127.0.0.1) never prompts.

### 7. Opening URLs and paths behaves differently in the CLI and the app; the CLI goes through PowerShell: **degraded**

**Status: fixed.**

`OpenSystem::open` (`actions/src/system.rs:177`) calls `open::that_detached`. In `open` 5.4.4 without the `shellexecute-on-windows` feature, Windows runs `powershell.exe -NoProfile -Command <script>` (`open-5.4.4/src/windows.rs:66-88`). That means:

- about 0.5–1.5 s of delay per OpenUrl or OpenPath key;
- failures are silent, because spawning `powershell` "succeeds";
- it fails outright where PowerShell is blocked by policy.

The Tauri app is different: `tauri-plugin-opener` turns that feature on, and Cargo merges features, so the app uses `ShellExecuteExW`. That gives the same key two behaviours depending on which host runs it.

Fix, in `host/Cargo.toml`:

```toml
open = { version = "5", features = ["shellexecute-on-windows"] }
```

### 8. Auto-switch jumps back to the default profile during window switches, UAC prompts, the taskbar and Start; Store apps never match: **cosmetic/degraded**

**Status: fixed.**

- `focus.rs:40-46`: `GetForegroundWindow()` returns NULL while focus is changing, and while the UAC secure desktop or lock screen is up. That becomes `None`. `spawn_focus_watch` (`host/src/lib.rs:1631-1653`) treats `None` as a new app and runs `apply_auto_switch`, which falls back to the default profile, so the tablet flickers to default and back.
- Clicking the taskbar or desktop, or opening Start, Search or Alt+Tab, reports `explorer`, `StartMenuExperienceHost`, `SearchHost` or `ShellExperienceHost`. Each one also resets to the default profile, and so does clicking PerritoDeck's own tray icon (that's explorer).
- Store/UWP apps (Calculator, Settings, Store Spotify) all report `ApplicationFrameHost`.

Fixes:

- Ignore `None` (keep the last app).
- Skip a short list of shell hosts. For explorer, only skip when the window class is `Shell_TrayWnd`, `Shell_SecondaryTrayWnd`, `Progman` or `WorkerW`.
- For `ApplicationFrameHost`, look up the child `Windows.UI.Core.CoreWindow` window's PID.

### 9. Pairing is slow when WSL or Hyper-V is installed: **degraded**

**Status: fixed** (default-route address first; adapters aren't dropped).

`core/src/discovery.rs:60-72` lists every non-loopback IPv4 address, sorted ascending. WSL and Hyper-V "vEthernet" adapters (172.16–31.x) therefore come **before** a home 192.168.x Wi-Fi address.

Android probes the addresses one at a time (`DeckViewModel.kt:173`) with a **5 s** connect timeout each (`Tls.kt:102`). One or two virtual adapters add 5–10 s to every pairing, and to reconnect fallbacks too.

Fix: put the address of the default-route interface first. The trick is a UDP `connect` to 192.0.2.1, then `local_addr()`; no packet is sent. Optionally drop adapters whose name starts with `vEthernet`. Or have Android probe in parallel.

### 10. Media plugin starts a PowerShell every ~2 s for as long as it runs, and a timeout puts a several-KB command on the key: **degraded**

**Status: fixed differently** (idle slow-down instead of watch-based polling; see the status table).

- `plugins/media/plugin.py:366-386`: `poll()` starts on `on_start` and calls `windows_run("status")`, a fresh `powershell.exe` with WinRT loading, every `POLL_SECONDS = 2`, whether or not any key shows the value. That's a noticeable constant CPU and battery cost on a laptop.
- `TIMEOUT = 3` (`:30`) is tight for a cold PowerShell 5.1 start with WinRT and Defender scanning.
- `windows_control` does not catch `subprocess.TimeoutExpired`. The SDK sends `str(e)` (`sdk-python/perritodeck_plugin.py:204`), and that string contains the whole argument list, including the base64 `-EncodedCommand`, so the key alert and the log get several KB of text.

Fixes:

- Catch `TimeoutExpired` and raise `RuntimeError("Windows didn't answer in 3 s")`.
- Only poll while a `status`/`title` value is watched.
- Better: keep one long-lived `powershell -Command -` that reads commands on stdin.

`screenshot/plugin.py:233-243` has the same uncaught `TimeoutExpired` (8 s).

### 11. Screenshots are wrong on mixed-DPI setups: **broken (laptop with an external monitor)**

**Status: fixed.**

`screenshot/plugin.py:164-170` calls `SetProcessDPIAware()`, which is *system* DPI awareness. When the laptop panel (say 150 %) and the external monitor (100 %) use different scaling, a system-aware process gets DPI-virtualised coordinates on the monitor whose DPI differs. Two effects:

- the `VirtualScreen` capture is cropped or scaled on the other monitor;
- `window` mode mixes physical `DWMWA_EXTENDED_FRAME_BOUNDS` with virtualised `CopyFromScreen` coordinates.

Fix: try per-monitor-v2 first.

```csharp
[DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
// in PS: if (-not [PerritoShot]::SetProcessDpiAwarenessContext([IntPtr]-4)) { [void][PerritoShot]::SetProcessDPIAware() }
```

### 12. Soundboard: about 1 s from key press to sound: **degraded**

**Status: fixed.**

`soundboard/plugin.py:118-121`: each sound starts `powershell.exe -STA` and loads PresentationCore/WPF before playing. That costs roughly 0.5–1.5 s, more when cold, which feels broken for a soundboard.

Fix: play in-process through ctypes `winmm`:

- `mciSendStringW('open "<path>" type mpegvideo alias s1')`, then `setaudio s1 volume to N`, `play s1`, and `stop`/`close` to stop. This handles MP3 and WAV with volume, as in-process aliases instead of processes.
- `PlaySoundW(path, None, SND_FILENAME|SND_ASYNC)` is enough for WAV.

Keep PowerShell as a fallback.

---

## Medium and low findings

### 13. Plugin process trees aren't killed; orphans are left after Quit: **degraded**

**Status: fixed.**

- `py.exe` puts `python.exe` in a job with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | SILENT_BREAKAWAY_OK` (checked in CPython `PC/launcher2.c:2618`). Killing `py.exe` (`plugins/src/lib.rs:662`) therefore kills Python, but anything Python started **breaks away** and survives. Soundboard's PowerShell players keep playing after a plugin restart or removal.
- `sdcompat/src/lib.rs:808` kills only `node.exe` or the native plugin exe, not its children.
- Tauri "Quit" (`app.exit(0)`) never stops plugins: there is no `RunEvent::Exit` handler and state isn't dropped. Python plugins exit when stdin reaches EOF. Whether Stream Deck plugins exit depends on the plugin noticing the WebSocket close.

Fix: create one Job object per plugin with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, without breakaway, through `windows-sys`:

1. `CreateJobObjectW`
2. `SetInformationJobObject(JobObjectExtendedLimitInformation)`
3. `AssignProcessToJobObject(job, child.raw_handle())`

Closing the job kills the whole tree, and so does the host dying. Nested jobs are fine on Windows 8 and later. Also stop plugins on `RunEvent::Exit`.

### 14. `text` action: Tab and newline may come out twice: **PLAUSIBLE: verify**

**Status: open.**

`vendor/enigo/src/win/win_impl.rs:285-295`: for `'\n'` and `'\t'`, `text()` queues the key (Return or Tab) **and then also** `queue_char(c)`, a `KEYEVENTF_UNICODE` packet, because the arms don't `continue`. `'\r'` also gets a Unicode packet. Some apps may insert a second tab or line break, or treat `U+000A` as Ctrl+Enter, which sends in some chat apps.

Fix: `continue` after the `'\n'` and `'\t'` arms, and skip `'\r'` entirely. Test: a Text key with `"a\tb\nc"` in Notepad and in a browser text box.

### 15. Hotkeys and text don't reach elevated windows (UIPI): **broken in that case; document it**

**Status: documented** in the user guide.

`SendInput` from a non-elevated host is silently dropped by elevated targets: Task Manager, admin terminals, and **OBS run as administrator** (which people do for game capture). Neither the return value nor `GetLastError` says so (per the `SendInput` docs). Same for the windows plugin's `PostMessageW(WM_CLOSE)`, though that one does report an error. This needs a user-guide note and possibly a hint when the foreground process is elevated (`OpenProcessToken`/`TokenElevation`).

### 16. `write_private` sets no ACL on Windows: **low**

**Status: open.**

`core/src/identity.rs:115-134`, `plugins/src/lib.rs:531`, `sdcompat/src/lib.rs:474`. On Windows these are plain `File::create`. That is acceptable by default because `%APPDATA%` inherits an owner+SYSTEM+Administrators ACL. It is *not* private if `PERRITODECK_DATA_DIR` points at a shared folder.

The data folder is also **Roaming** AppData (`ProjectDirs::config_dir`). With roaming profiles, the TLS key, pairing secrets, Python plugins and Stream Deck `node_modules` all roam. Consider `data_local_dir()` for plugins and logs.

### 17. Plugin `main` can point outside its folder on Windows: **low**

**Status: fixed** (checked the same way on every platform).

`plugins/src/manifest.rs:116-118` rejects `..` and `is_absolute()`. On Windows `\Windows\System32\x.exe` (root-relative) and `C:x.exe` (drive-relative) are *not* absolute. `dir.join()` then leaves the plugin folder. Fix: reuse `sdcompat::manifest::inside()`, which rejects `RootDir` and `Prefix` components.

### 18. Stream Deck `-info` `platformVersion` is `"windows"`: **cosmetic**

**Status: fixed.**

`sdcompat/src/lib.rs:524` sets `platformVersion: std::env::consts::OS`. Real Stream Deck sends a version string such as `"10.0.22631"`. Harmless unless a plugin parses it.

### 19. The tray menu opens on left-click: **cosmetic**

**Status: open.**

`app/src-tauri/src/lib.rs` uses `show_menu_on_left_click(true)`. On Windows the convention is that left-click (or double-click) opens the window and right-click opens the menu.

### 20. Uncommitted `perritodeck-app mcp` stdio bridge: **note**

**Status: open.**

`app/src-tauri/src/main.rs` (working tree) runs MCP stdio from a `windows_subsystem = "windows"` exe. With an MCP client that passes pipes, stdio works, because the std handles come in through STARTUPINFO. Run by hand from `cmd` or PowerShell, it prints nothing, `eprintln!` goes nowhere, and cmd doesn't wait for it. Also `mcp/src/config.rs` examples show `C:\Program Files\PerritoDeck\perritodeck-host.exe`, but the NSIS bundle (currentUser, `%LOCALAPPDATA%\PerritoDeck`) doesn't ship `perritodeck-host.exe`: there's no `externalBin`.

---

## Checked and fine

- **`win_audio.py`:** every vtable index matches the SDK headers:
  - IMMDeviceEnumerator 3/4
  - IMMDeviceCollection 3/4
  - IMMDevice 3/4/5
  - IPropertyStore GetValue 5
  - IAudioEndpointVolume 7/9/14/15
  - IAudioSessionManager2 5
  - IAudioSessionEnumerator 3/4
  - IAudioSessionControl2 GetProcessId 14
  - ISimpleAudioVolume 3–6
  - IPolicyConfig SetDefaultEndpoint 13

  The CLSIDs and IIDs are right. PROPVARIANT is 24 bytes on x64 and 16 on x86. GUID is 16 bytes. HRESULTs are signed (`c_long`), with `RPC_E_CHANGED_MODE` handled. MTA init and uninit happen per call. Floats go through prototypes. Generator cleanup releases the interfaces before `CoUninitialize` (CPython refcounting).
- **`windows/winapi.py`:** `PROCESSENTRY32W` is 568 bytes on x64 with the `ULONG_PTR` padding. All handles and HWNDs are `c_void_p`, so nothing is truncated to 32 bits. `INVALID_HANDLE_VALUE` is compared correctly. The `EnumWindows` callback is `WINFUNCTYPE`.
- **`sysmon/winapi.py`:** `MEMORYSTATUSEX` is 64 bytes, `SYSTEM_POWER_STATUS` is 12, and `GetTickCount64` returns `c_uint64`.
- **`screenshot/winapi.py`:** `SHGetKnownFolderPath` signature, `FOLDERID_Pictures` and `CoTaskMemFree` are correct.
- **`clipboard/winapi.py`:** `keybd_event` signature is correct.
- **PowerShell calls:** all use `-NoProfile -NonInteractive -EncodedCommand` (UTF-16LE base64), with `-STA` where WinForms or WPF need it (clipboard, screenshot, soundboard). Output is ASCII-escaped JSON or base64, so code pages don't matter, and `CREATE_NO_WINDOW` is set. `-ExecutionPolicy Bypass` isn't needed, because execution policy doesn't apply to `-EncodedCommand`.
- **Plugin stdio:** `PYTHONUTF8` and `PYTHONIOENCODING` are set and the SDK reconfigures UTF-8. CRLF from Python text-mode stdout is fine, because tokio `lines()` strips `\r\n`. The plugin's `CREATE_NO_WINDOW` gives Python, and through inheritance its PowerShell children, a hidden console, so nothing flashes.
- **Rust volume (COM):** `CoInitializeEx` MTA, a repeat call harmless, `windows` 0.61 signatures.
- **`focus.rs` handles:** `PROCESS_QUERY_LIMITED_INFORMATION` works on elevated targets, and the handle is closed.
- **Stream Deck manifests:** the UTF-8 BOM is stripped (`manifest.rs:87`). Code paths with backslashes or drive prefixes are rejected by `inside()`.
- **Zips:** `zip` 2.4.2 `enclosed_name` treats `\` as a separator on Windows and rejects prefixes and `..`.
- **Long paths:** Rust std adds the `\\?\` prefix to long absolute paths itself.
- **Other:** TOML BOM is handled (`toml_parser` 1.1.5). `default_data_dir()` is `%APPDATA%\perritodeck\PerritoDeck\config`. `find_adb` covers `%LOCALAPPDATA%\Android\Sdk`. `dragDropEnabled: false` keeps HTML5 drag and drop working in WebView2.

---

## Packaging and CI (brief)

- `tauri.conf.json`:
  - `targets: "all"` builds NSIS (currentUser) and MSI (perMachine). Installing both gives two copies and two autostart entries, and they fight over port 46710.
  - The autostart `Run` key isn't removed on uninstall.
  - Installers are unsigned, so expect a SmartScreen warning and "Publisher: Unknown" in the firewall prompt.
  - `webviewInstallMode: downloadBootstrapper` needs internet on Windows 10 machines that don't have WebView2.
- `host.yml` Windows job: fmt, clippy and test. It's correct, but blocked by billing (see section 0).

---

## Windows manual test plan (in priority order)

Collect throughout:

- `perritodeck-host paths`;
- the log folder (`<data>\logs\perritodeck.log` once the in-progress `logging.rs` lands), or run the CLI host in a terminal;
- screenshots of any dialog.

Results of the 2026-10-12 run are marked **[done]** / **[not done]** under each step.

1. **Build and test** (CI hasn't since `9e698c8`): `cd host; cargo clippy --all-targets -- -D warnings; cargo test`, then `cd app; npm ci; npx tauri build --debug`. Save the full error output.
   **[done]** Clean on the first try (fmt, clippy, all tests); `tauri dev` and `tauri build --no-bundle` work. Note: `--debug` builds a console-subsystem app, so use a release build for the console-window checks.
2. **CLI host basics:** `cargo run -p perritodeck-host -- --pair`.
   - Firewall prompt: note which boxes are ticked and whether Wi-Fi is Public or Private (Settings → Network → Properties).
   - Pair the tablet by QR. Time it, and note every `a=` address in the QR text, especially vEthernet 172.x and 169.254.x.
   - Check that mDNS discovery works from the tablet.
   **[done]** with the desktop app instead of the CLI host: prompt per exe path, answered with Public allowed (this network is Public); QR pairing under 2.5 s, addresses `192.168.x` first then two `vEthernet` 172.x (169.254.x now left out); mDNS discovery works on Fire OS.
3. **Actions via CLI.** Use **cmd.exe** or PowerShell 7.3+. Windows PowerShell 5.1 strips the inner quotes.
   - `perritodeck-host action "{\"type\":\"hotkey\",\"keys\":\"ctrl+plus\"}"` in a browser: expect zoom (finding 3, now fixed; before the fix nothing happened).
   - Hotkeys `ctrl+shift+esc` and `win+d`, then media play/pause and the volume keys.
   - A text action with `"a\tb\nc"` in Notepad (finding 14).
   - openUrl: time it (finding 7).
   - launch `notepad`, then `code` (finding 4), then OBS by full path (expect "Failed to find locale").
   - command `hostname`.
   **[done]** `ctrl+plus` → Ctrl + =/+ (fixed), shifted symbols, `a\tb\nc` in Notepad and a Win32 edit (fixed: pacing, no doubled Tab/newline), Launch `code`, `cmd`, OBS by path and a quoted path from the release app (fixed), command `hostname`. **[not done]** `ctrl+shift+esc`, `win+d`, media and volume keys (they'd disrupt the desktop in use), openUrl timing.
4. **Installed app (release build), not `tauri dev`:** install the NSIS setup.
   - With adb installed and on PATH, watch for console windows every 4 s (finding 1).
   - Press the starter "Run a command" key (finding 2).
   - Tray: open, pause, quit. After Quit, look in Task Manager for leftover `python.exe`, `py.exe`, `node.exe` or `powershell.exe` (finding 13).
   - Autostart on, log out and back in: the app should start hidden in the tray.
   - "Restart now" after changing the port: does single-instance let the new process start?
   - "Open data folder": does it open *inside* the folder, or the parent with the folder selected?
   - "Open profile file" when `.toml` has no associated app.
   **[done]** with `tauri build --no-bundle` (no installer): no console windows; "Run a command" quiet; tray Quit leaves no plugin process (7 Python + 2 Stream Deck plugins, a sound playing). **[done 2026-10-13]** NSIS install, first run, autostart (entry and hidden start), upgrade over the running app, uninstall (next-steps item 5; an update removing "Start at login" was fixed). **[not done]** "Restart now" after a port change, Open data folder/profile file.
5. **Auto-switch:** add a rule for an app.
   - Click the taskbar, press Win, Alt+Tab, and trigger a UAC prompt (for example, open an admin terminal). Watch the tablet for flicker to the default profile (finding 8).
   - Try Calculator (ApplicationFrameHost).
   **[done]** rule `charmap.exe`: taskbar, desktop, Start/Search and Task View keep the profile; another app and closing the app switch back.
   **[done]** UAC prompt (2026-10-13, answered by a person): with Character Map in front, the tablet kept its rule's profile through the taskbar, a jump list, the PerritoDeck window, and the secure desktop (no foreground window for about 1.5 s). It switched to the default profile 0.2 s after "Administrator: Command Prompt" came to the front. **[not done]** Calculator.
6. **Plugins (Python):** first check whether Python is installed (`py -0p`).
   - If it is, temporarily rename or hide it (or test on a clean account) to see the 9009 loop (finding 5).
   - Then exercise each plugin:
     - **audio:** volume, mute, mic mute, switch the default device, app volume for Spotify or a browser.
     - **windows:** list, focus, minimize, close, show desktop.
     - **sysmon:** CPU, RAM, network, battery, NVIDIA GPU if present.
     - **clipboard:** copy, paste, slots, and the restore delay.
     - **screenshot:** screen, window and region with the laptop panel at 150 % plus an external monitor at 100 % (finding 11).
     - **soundboard:** latency, MP3 volume, stop/toggle.
     - **media (Now playing):** check Task Manager CPU while idle (finding 10).
   - Note each plugin's status in the app and any stderr lines in the log.
   **[done]** Python 3.14 installed (Store stub case not tried). All seven run (as `python.exe`, no `py.exe`), with their actions and live values: audio volume and mic mute (put back), windows focus/launch/quit (the launched app survives a plugin restart), sysmon (network rates match Windows' counters), clipboard paste (clipboard restored), screenshot window, soundboard play/stop, media status. **[not done]** mixed-DPI screenshots, idle CPU of media, region capture.
7. **USB:** tablet on a cable with adb. Check the log for `adb reverse` and for connecting with Wi-Fi off.
   **[done]** `adb reverse` within 0.3 s of start; pairing and connecting with Wi-Fi off; falls back to USB when Wi-Fi is blocked.
8. **Stream Deck plugins:** install one Node `.streamDeckPlugin` (needs `node` on PATH) and one native `.exe` plugin if available. Open a settings page, remove and reinstall while running (file locks), and restart.
   **[done]** Elgato's open-source hello-world and counter samples: install, keys, settings page, remove/reinstall while running, Restart (fixed; it aborted the app). **[not done]** a native `.exe` plugin.
9. **Elevated target:** run OBS as admin and send a hotkey to confirm UIPI blocking (finding 15), so the user guide can say so.
   **[not done]**
10. **Kindle/Fire over Wi-Fi:** if time allows, repeat step 2 with the Fire tablet on the Windows host.
   **[done]** (see step 2): the Fire 7 is the tablet used throughout.
