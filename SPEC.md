# Claude Code Prompt — PerritoDeck

> Paste this into Claude Code at the root of an empty repo (or save it as `SPEC.md` and tell Claude Code to read it).

---

## Role & goal

You are building **PerritoDeck**, a free/open-source (GPL-3.0 or Apache-2.0 — ask me before choosing) alternative to the Elgato Stream Deck. It turns an **Android tablet or phone** into a grid of customizable, live-updating buttons, dials and touch strips that control a **desktop host app** running on Windows, macOS and Linux.

### Naming & identifiers (use consistently everywhere)
- Display name: **Perrito** on the Android launcher/app stores' short name; **PerritoDeck** as the project and full product name.
- GitHub org: `perritodeck` (monorepo `perritodeck/perritodeck`).
- Website: `perritodeck.dev` (with `perritodeck.app` redirecting, if registered).
- Android `applicationId`: `dev.perritodeck.client` (never change after first release). Desktop bundle id: `dev.perritodeck.host`.
- Rust crates / npm scope: `perritodeck-*` / `@perritodeck/*`. Plugin ids: reverse-DNS, e.g. `dev.perritodeck.obs`.
- Never use Elgato names, logos or assets in branding.

Work in phases. At the start of each phase, write a short plan in `docs/plan/phase-N.md`, then implement, test, and stop for my review before moving on. Keep a running `docs/ARCHITECTURE.md` and `docs/DECISIONS.md` (one line per decision with the reason).

## Prior art to study first (do not copy code with incompatible licenses)

- **Bitfocus Companion** (MIT) — huge module library; its **Satellite API** lets remote surfaces act as button panels.
- **Macro Deck** (Apache-2.0) — Windows host + Android client; closest existing equivalent.
- **OpenDeck** and **StreamController** (Linux) — OpenDeck runs Elgato-SDK plugins.
- Elgato Stream Deck SDK docs — the plugin protocol (JSON over WebSocket, `keyDown`/`keyUp`/`dialRotate`/`setImage`/`setTitle`/`setState` events, manifest.json, property inspector HTML).

Summarize what each does well/badly in `docs/prior-art.md` before writing code.

## Concepts to model (mirror Stream Deck's mental model)

- **Device/Surface** — a connected tablet; reports its grid size, screen DPI, capabilities (keys, dials, touch strip, haptics).
- **Profile** — a full layout; can auto-switch when a given app gains focus on the host.
- **Page** — profiles have multiple pages; swipe or page-nav keys.
- **Folder** — a key that opens a sub-grid with a back key.
- **Key** — has 1..n **states** (e.g. mic muted/unmuted), each with icon, title, title position/font/color, background.
- **Action** — what a key does: built-in or from a plugin. Events: press, release, long-press, double-tap; dials: rotate, press, touch.
- **Multi-action** — ordered list of actions with delays; **toggle multi-action** alternates between two lists.
- **Feedback** — host/plugins can push new icons, titles, states, progress bars, or live images (clock, CPU graph, now-playing art) to any key at any time.

## Architecture

```
/host      Desktop host (Rust core + Tauri 2 UI)
/android   Android client (Kotlin, Jetpack Compose)
/protocol  Shared protocol spec (JSON Schema or protobuf) + generated types
/plugins   Built-in plugins + plugin SDK (TypeScript/Node + any-language via WebSocket)
/website   Static site (Astro or Hugo) for docs + downloads
/docs
```

### Host app (Rust + Tauri 2)
- Tray/menu-bar app, starts at login, minimal RAM. Config editor UI is a Tauri webview: drag actions from a sidebar onto a grid that mirrors the connected tablet's exact layout.
- Core responsibilities: device registry, profile storage (JSON/TOML in the OS config dir, human-editable and git-friendly), action dispatch, plugin process management, rendering key images.
- **Rendering on host** (like Stream Deck): host composites icon + title into a PNG/WebP per key and pushes it; client just displays. Also support a lightweight "vector mode" (icon id + text) to save bandwidth.
- Platform-specific actions behind a trait, with per-OS implementations:
  - Keystrokes/hotkeys & text typing: Windows `SendInput`; macOS CGEvent (needs Accessibility permission — guide the user through it); Linux X11 XTest **and** Wayland via `uinput`/libei/portal (document limitations).
  - Launch app / open file / open URL / run shell command / scripts.
  - Media keys, system volume (WASAPI / CoreAudio / PipeWire-PulseAudio), per-app volume where possible.
  - Focused-app detection for profile auto-switch.
- Built-in integrations (as first-party plugins): OBS (obs-websocket v5), Home Assistant (WebSocket API), MQTT, HTTP request/webhook, Discord (where API allows), Spotify/MPRIS/SMTC now-playing, Twitch, timers/counters, clock, system stats.
- Import/export profiles as a single `.perritoprofile` zip. Stretch: importer for Elgato `.streamDeckProfile` files.

### Android client (Kotlin + Compose)
- Full-screen, immersive, keep-screen-on, landscape/portrait, configurable grid (auto-suggest based on screen size: phone 3×5, 10" tablet 5×8, etc.).
- Supports keys, virtual dials (circular drag gesture), touch strip (horizontal swipe zone), haptic feedback, long-press, multi-touch (several keys at once).
- Burn-in protection option (dim/shift after idle), brightness control from host, "kiosk/dedicated device" mode with screen pinning.
- Works as a dumb surface — no logic on device except caching the last layout and images so reconnects are instant.
- Optional mode: act as a **Bitfocus Companion Satellite** client, so it's useful on day one with Companion users.
- **F-Droid constraints from day one:** no Google Play Services, Firebase, proprietary analytics or crash reporters; all deps FOSS; reproducible builds; no network calls except to the paired host. Use build flavors only if a Play-specific feature is ever needed (prefer not).
- minSdk 26 (old tablets are a core audience — repurposing e-waste is part of the pitch).

### Connection & protocol
- **Transports:** (1) LAN WebSocket, (2) USB via `adb reverse` (zero-config when plugged in, low latency, keeps tablet charged), (3) optional Wi-Fi Direct later.
- **Discovery:** mDNS/DNS-SD (`_perritodeck._tcp`) + manual IP entry fallback.
- **Pairing:** host shows a QR code containing host id, address, port, and a one-time secret; client scans it. Afterwards: mutual auth with stored keys; TLS with pinned self-signed cert (or Noise protocol). Never allow unauthenticated control — this app can type keystrokes and run commands.
- **Messages:** versioned (`protocolVersion`), with capability negotiation. Client→host: `hello`, `keyDown/keyUp`, `dialRotate`, `dialPress`, `touchStrip`, `heartbeat`. Host→client: `layout`, `setImage`, `setTitle`, `setState`, `setBrightness`, `showAlert/showOk`, `switchPage`.
- Target <30 ms press-to-action latency on LAN; measure and log it.

### Plugin system
- Plugins are separate processes speaking JSON over a local WebSocket (language-agnostic), each with a `manifest.json` (id, name, version, actions, icons, settings UI as HTML "property inspector").
- Provide a TypeScript SDK + template repo + a Python example.
- **Phase-later stretch:** an Elgato-SDK compatibility layer (study OpenDeck's approach) so many existing Stream Deck plugins run unmodified. Check licensing/terms before shipping; never use Elgato trademarks or assets.
- Sandbox/permissions: plugins declare needed capabilities (network, shell, keystrokes); host shows them on install.
- Optional local **MCP server** exposing actions so AI assistants can trigger or create buttons (Elgato added MCP support in 2026; parity is worth having).

## Phases

1. **Skeleton & protocol** — monorepo, CI (GitHub Actions: Rust tests, Android build, lint), protocol schema, host that accepts a connection and logs key presses, Android app that shows a static grid and sends presses over LAN.
2. **Core loop** — pairing with QR, mDNS, host-rendered key images, built-in actions: hotkey, open app/URL, run command, text, media keys, multi-action, folders, pages, states.
3. **Config editor UI** — drag-and-drop editor in Tauri, icon library (bundle a FOSS icon set like Material Symbols/Tabler; allow custom images/GIFs), profiles + auto-switch.
4. **USB transport, dials, touch strip, haptics, live feedback** (clock, CPU, now-playing).
5. **Plugin SDK + first-party plugins** (OBS, Home Assistant, MQTT, HTTP, Spotify/MPRIS).
6. **Packaging & distribution** (see below).
7. **Stretch:** Companion Satellite mode, Elgato plugin compatibility, iOS client, support driving real Stream Deck/clone hardware via HID (like Companion/StreamController), multi-device per host, cloud-free profile sync via file export.

## Distribution requirements (design for these from the start)

- **Android:** Google Play (AAB, signing via Play App Signing, privacy policy page, Data Safety form — "no data collected"), **F-Droid** (metadata in `fastlane/metadata/android/` for descriptions/screenshots, reproducible build, no anti-features), **IzzyOnDroid** repo, **Obtainium**-friendly GitHub Releases with signed APKs, optionally Amazon Appstore, Samsung Galaxy Store, Huawei AppGallery. Keep `applicationId` stable forever (`dev.perritodeck.client`).
- **Desktop:** GitHub Releases with Windows MSI/MSIX (+ winget manifest), macOS signed & notarized .dmg (universal binary) + Homebrew cask, Linux AppImage + Flatpak (Flathub) + .deb/.rpm + AUR.
- **Website:** static site with docs, plugin directory, download page auto-detecting OS, privacy policy, press kit.
- Changelog via Conventional Commits + release-please. Localization-ready strings (Weblate-compatible) from day one.

## Engineering rules

- Tests for protocol encoding, action dispatch, and profile migration. Profile files carry a `schemaVersion` with migrations.
- No telemetry by default. If ever added: opt-in, documented, self-hostable.
- Security review checklist in `docs/SECURITY.md` (auth, replay protection, plugin permissions, command injection).
- Accessibility: TalkBack labels on keys, large-text, high-contrast themes.
- Ask me before: choosing the license, adding any dependency with a non-FOSS license, or any network service outside the user's LAN.

Start with Phase 0: read this spec, write `docs/prior-art.md` and `docs/plan/phase-1.md`, list open questions, and wait for my go-ahead.
