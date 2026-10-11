# Phase 2 — Core loop

**Goal:** a tablet pairs with the host once, then reconnects securely and automatically. Keys show host-rendered icons and do real things: hotkeys, typing, opening apps and URLs, running commands, media keys, multi-actions, folders, pages and toggle states. Layouts live in a hand-editable profile file until the Phase 3 editor exists.

## 1. Pairing & transport security

Phase 1's shared session token goes away. Every connection, including loopback, must be from a paired device. Loopback isn't safe to trust blindly: any web page in a local browser can open `ws://127.0.0.1`.

**Host identity.** On first run the host generates a self-signed TLS certificate (ECDSA P-256, `rcgen`) and stores it in the OS config dir. Its SHA-256 fingerprint is the host's identity. The host serves `wss://` only.

**Pairing flow:**
1. The user clicks "Pair a tablet" (Phase 2: `perritodeck-host pair` prints it in the terminal; Phase 3: shown in the UI). The host shows a QR code and a 6-digit code, both valid for 5 minutes and single use.
2. The QR encodes `perritodeck://pair?v=1&n=<host name>&a=<ip:port>[&a=…]&fp=<base64url fingerprint>&c=<code>`.
3. The client connects to `wss://`, **pins the certificate fingerprint** from the QR, and sends `pair {code, deviceId, deviceName}`.
4. The host checks the code (constant time, max 5 attempts per code) and replies `paired {deviceSecret}`, a random 256-bit secret. The host stores only a hash of it; the client stores it in app-private storage with the pinned fingerprint.
5. From then on `hello` carries `deviceId` + `deviceSecret` over pinned TLS.

**Without a camera** (most Kindles have none on the back): the user types the host address and the 6-digit code. The client shows the first 8 characters of the fingerprint and the host shows the same, so the user can compare them (trust on first use).

**Also:**
- Reject WebSocket upgrades that carry an `Origin` header (browsers always send one; our clients never do).
- A "Revoke" command in the host CLI and, later, the UI.
- Remove `usesCleartextTraffic` from the Android manifest.

**Android:** QR scanning with CameraX + ZXing core (both Apache-2.0, no Play Services). If the device has no camera, the scan button is hidden.

## 2. Discovery

- The host advertises `_perritodeck._tcp` over mDNS (`mdns-sd` crate). TXT records: `v=1`, `id=<host id>`, `fp=<fingerprint prefix>`, `n=<name>`.
- The client lists hosts with `NsdManager` (framework API, no dependency). Paired hosts reconnect automatically; unpaired ones open the pairing screen.
- Manual address entry stays as a fallback for networks that block multicast (common on guest and mesh Wi-Fi).

## 3. Actions (`host/crates/actions`)

Every action implements one trait: `async fn run(&self, ctx: &ActionContext) -> Result<ActionOutcome>`. Results feed back to the key: `showOk`, `showAlert` or a state change.

| Action | Windows | macOS | Linux |
|---|---|---|---|
| Hotkey (e.g. `Ctrl+Shift+M`) | `enigo` (SendInput) | `enigo` (CGEvent) — needs Accessibility permission; host explains how to grant it | X11: `enigo`; Wayland: libei / RemoteDesktop portal (prompts once), documented limits |
| Type text | same | same | same |
| Media keys (play/pause, next, volume) | same | same | same, or MPRIS over D-Bus |
| Open URL / file | `open` crate | `open` crate | `open` crate |
| Launch app | path or shell-free argv | `open -a` | `.desktop` id or argv |
| Run command | argv array, never a shell string; optional working dir, timeout, "show output on key" | same | same |
| Multi-action | ordered steps with delays; toggle variant alternates between two step lists | | |
| Navigation | open folder, back, next/previous/go-to page, switch profile | | |
| Toggle state | flips a key between states (each state has its own icon, title and action) | | |

## 4. Profiles

- A TOML file per profile in the config dir (`profiles/default.toml`), carrying `schemaVersion` with migrations from day one.
- Structure: profile → pages → keys `{position, states[{title, icon, bg, fg}], action}`. Folders are nested pages.
- Hot reload: the host watches the file (`notify` crate) and pushes new layouts to connected devices within a second of saving.
- A starter profile is generated on first run with examples of every action type, commented, so the file teaches itself.

## 5. Host-rendered key images

- Rendering uses `tiny-skia` and `cosmic-text`, with a bundled font (Inter, OFL) and bundled icon set (Tabler Icons, MIT).
- Each key is rendered at the tablet's real pixel size, derived from `hello.screen` and the grid. It's sent as lossless WebP via `setImage`, and only when the image actually changes (hashed).
- Custom images: PNG/JPEG/WebP/SVG paths in the profile. Animated GIFs wait for Phase 4.
- The client draws the image to fill the key. Titles move into the image, and the client's own text rendering becomes the fallback for `vector mode`.

## 6. Protocol additions (v1, minor)

Protocol version stays at v1, because adding message types and optional fields is a minor change and older peers ignore unknown types. Every addition gets a schema entry and a fixture.

- `pair` / `paired` (new)
- `hello.deviceSecret` (new optional field; `hello.token` deprecated)
- `setImage` (implemented)
- `setState`, `showOk`, `showAlert`, `setBrightness`, `switchPage` (new)
- `layout.page {index, count, name}` (new optional field)

## 7. Testing

- **Host:**
  - Pairing state machine unit tests, including expiry, reuse and brute-force limits.
  - An action dispatcher with a fake input backend, so actions are testable headlessly in CI.
  - Golden-image tests for the renderer, with a pixel tolerance.
- **End to end:** fakedeck learns to pair and pin.
- **Android:**
  - Pairing URI parser tests.
  - Pinned-certificate tests against MockWebServer with a self-signed cert.

## Out of scope
Tauri UI and drag-and-drop editor (Phase 3), dials and touch strip (Phase 4), plugins (Phase 5).

## Open questions
1. **Wayland input:** the RemoteDesktop portal asks for permission on every host start on some desktops. Is that acceptable for v1, or should we ship a small `uinput` helper? `uinput` needs a udev rule, and that's a sudo step during install.
2. **Profile format:** TOML, which is human-friendly and supports comments, or JSON, which is the same format the Phase 3 editor and plugins use? Leaning TOML.
3. **Run command scope:** allow arbitrary commands from day one, or require the host to be started with `--allow-commands` until the Phase 3 UI can confirm each new command?

## Status (2026-10-11)

| Item | State |
|---|---|
| Pairing (6-digit code, QR link), TLS with pinned host cert, per-device secrets, revoke | Done — host, fakedeck and Android; 15 host end-to-end tests; emulator test pairs over TLS |
| `Origin` rejection, cleartext disabled on Android | Done |
| mDNS advertisement (host) and discovery (Android NsdManager) | Done; untested on a real network until the laptop/Kindle check |
| Actions crate (hotkey, text, media, open, launch, command, multi) | Prototype done, not yet wired to keys |
| Host-rendered key images (Inter, auto-fit titles, PNG icons) | Done for the demo layout; Android draws them |
| Profiles (TOML), navigation (folders/pages), states | Not started — waiting on open question 2 |
| In-app QR scanning | Replaced by the `perritodeck://pair` link any camera app can open |
