# Phase 3 — Desktop app & editor

## Status (2026-10-11)

Done in `host/app/` (with the host runtime as the `perritodeck-host` library):
- **Tray app:** Open, Pair a tablet, Pause keys, Quit; closing the window keeps the host running; single instance; start at login (off by default, starts hidden in the tray).
- **Editor:** the profile drawn as the tablet shows it; a key inspector for title, colours, image and action (hotkey recorder, text, media, websites, files, programs, commands, OBS, Home Assistant, page navigation); drag and drop (and Alt+arrows) to move or swap keys, onto a page tab to send them to that page; copy/paste keys; add, rename, reorder and remove pages; undo/redo across everything; "Try it" runs an action from the editor. Saves go to the TOML file in place, keeping comments, and reach the tablets straight away.
- **Profiles:** create, copy, rename, delete, export and import (`.perritoprofile`, a zip with the images). Each tablet shows its chosen profile.
- **Switching:** rules for showing a profile while a given app is in front (Windows, macOS, Linux X11).
- **Tablets:** status, rename, choose profile, unpair. **Pairing** with QR, code and fingerprint. **Settings:** pause, start at login, addresses, folders.
- **macOS:** Accessibility permission check with a button to the right System Settings page.
- **Tests:** Playwright against a mocked host (23 tests), Rust tests for the host API, and a CI job driving the real Linux app over WebDriver (pair, edit, see it on the tablet, undo). Bundles build on all three OSes.

Not done yet: toggle (multi-state) and multi-step keys are edited in the file; built-in icon set; auto-switch on Wayland; update check; settings for port and Wi-Fi access; per-device latency in the Tablets view.

**Goal:** a normal desktop app that someone who has never seen a terminal can install, pair a tablet with, and lay out buttons in by dragging, with every edit appearing on the tablet as they make it.

## The app shell (Tauri 2)

- The Phase 2 headless host becomes a library (`perritodeck-core` + `actions` + `render`) embedded in a Tauri 2 app at `host/app/`. The CLI binary stays for headless and server use.
- **Tray / menu-bar icon** with: open editor, pair a tablet, connected devices, pause all keys, quit.
- **Start at login** (`tauri-plugin-autostart`), off by default and offered on first run.
- **Single instance** (`tauri-plugin-single-instance`): launching it again focuses the open window.
- Closing the editor window keeps the host running in the tray.
- **Updates:** check GitHub Releases and tell the user (no auto-install until signing is in place).

## Editor UI (Svelte 5 + TypeScript, Vite)

Svelte is what OpenDeck and TilePad use, so contributors can move between projects. It compiles to small bundles, which keeps the app light.

```
┌──────────────────────────────────────────────────────────────────────┐
│ [Kindle Fire HD ▾]  Profile: Streaming ▾   Page 1 of 3  ◀ ▶  [+ page]│
├──────────────┬───────────────────────────────────────┬───────────────┤
│ Actions      │   ┌──┐┌──┐┌──┐┌──┐┌──┐┌──┐┌──┐┌──┐     │ Key 3         │
│  Keyboard    │   │  ││  ││▓▓││  ││  ││  ││  ││  │     │ Title [ Mute ]│
│   Hotkey     │   └──┘└──┘└──┘└──┘└──┘└──┘└──┘└──┘     │ Icon  [🎤 ▾]  │
│   Text       │   ┌──┐┌──┐┌──┐ …  grid mirrors the     │ Colours ■ ■   │
│  Media       │   │  ││  ││  │    selected tablet's     │ Action        │
│  Open        │   └──┘└──┘└──┘    exact shape           │  Hotkey       │
│  Navigation  │                                        │  [Ctrl+Shift+M]│
│  Multi       │                                        │ States  1 | 2 │
│  Plugins (5) │                                        │ [Test action] │
└──────────────┴───────────────────────────────────────┴───────────────┘
```

**Editing:**
- Drag an action onto a key, drag keys to swap them, and drag a key onto a folder to move it inside.
- The **key inspector** shows title, font size, title position, colours, icon (searchable Tabler set plus "choose image…"), states and action settings.
- Action settings forms are **generated from each action's JSON Schema**. The same schemas validate profiles and, in Phase 5, describe plugin actions. Hotkeys are recorded by pressing them, so nobody has to type `Ctrl+Shift+M`.
- **Test action** runs the key's action from the editor.
- **Undo/redo** covers every edit, and Ctrl/Cmd+C/V copies keys.

**Live preview and storage:**
- Every edit is saved to the profile file and pushed to the tablet within about 100 ms.
- The file watcher from Phase 2 keeps hand edits working; the editor reloads if the file changes underneath it.

**Accessibility:**
- Every drag has a keyboard equivalent (select key, press Enter, choose action).
- Full screen-reader labels and visible focus.

## Profiles, pages and auto-switch

- Create, rename, duplicate and delete profiles; reorder pages; nested folders with an automatic Back key.
- Each device has its own active profile; a profile can be shared by several devices with different grid sizes. Keys keep their row and column, and anything outside the grid goes to an overflow page.
- **Auto-switch:** rules of the form "when *app* is focused, switch *device* to *profile*". Focused-app detection per OS:
  - Windows: `GetForegroundWindow` → process name.
  - macOS: `NSWorkspace` frontmost application.
  - Linux X11: `_NET_ACTIVE_WINDOW`.
  - Linux Wayland: GNOME Shell extension / KDE KWin script / Hyprland and Sway IPC, as StreamController does.
- **Import/export** a profile as `.perritoprofile` (a zip of the TOML plus its images).

## Devices and pairing

- **Devices view:** paired tablets with name, model, last seen, connection type (Wi-Fi/USB) and latency. Actions: rename, choose profile, revoke.
- **Pair a tablet** shows the QR code and the 6-digit code from Phase 2, with a live "waiting…" state that flips to the device's name when it pairs.
- **Settings:** port, allow Wi-Fi connections (on/off), start at login, theme, diagnostics (open logs folder, copy debug info).

## Platform details

- **macOS:** check the Accessibility permission on start, with a one-click "Open System Settings" and a clear explanation of why it's needed. Universal binary.
- **Windows:** a firewall prompt on first LAN listen is expected; explain it beforehand.
- **Linux:** tray via the StatusNotifierItem/AppIndicator protocol (GNOME needs the AppIndicator extension; detect and explain). Wayland input path chosen in Phase 2.

## Testing

- **UI:** Vitest component tests, plus Playwright tests against the editor running in a browser with Tauri's `invoke` mocked. Drag and drop, keyboard editing and undo get covered.
- **Rust:** command handlers and the profile store tested directly, including migration from Phase 2 profile files.
- **CI:** builds the Tauri app on Windows, macOS and Linux for every PR. Bundles are uploaded as artifacts but not signed until Phase 6.

## Out of scope
Plugins and the plugin store (Phase 5), dials and touch strip editing (Phase 4), signed and notarized installers (Phase 6).

## Open questions
1. **Editing from the tablet:** a long-press "edit mode" on the tablet for quick title and icon changes would be handy for couch use. Phase 3 or later?
2. **Theme:** follow the OS light/dark setting only, or also offer a custom accent colour?
3. **Telemetry-free crash reports:** offer a "copy crash report" button that the user pastes into a GitHub issue themselves?
