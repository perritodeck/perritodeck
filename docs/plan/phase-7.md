# Phase 7 — Stretch (pulled forward: Companion and Stream Deck plugins)

**Goal:** be useful on day one to people who already run Bitfocus Companion or own Stream
Deck plugins, before packaging (Phase 6).

## Companion Satellite mode

Bitfocus Companion (MIT) drives broadcast and AV gear with thousands of modules. Its
**Satellite API** lets a remote panel register as a surface: Companion draws the buttons
and receives the presses. PerritoDeck speaks it as a client, so a tablet can be a
Companion surface without anything installed on the Companion side.

- **Turn it on:** `[companion]` in `integrations.toml` (`host`, `port`, default 16622),
  and a profile with `companion = true` (the app's "+ Profile" has a checkbox for it).
- **Per tablet:** each tablet showing a Companion profile is registered as its own
  surface (`ADD-DEVICE`, id `perritodeck-<tablet id>`, named "PerritoDeck <tablet
  name>"), sized to the tablet's grid, with key images at the tablet's key size. It's
  removed when the tablet disconnects or switches to another profile, so auto-switching
  works with Companion profiles too.
- **Drawing:** Companion's `KEY-STATE` images go to the tablet as they are (PNG when
  Companion offers it, from API 1.12; raw RGB from older versions is converted). Its
  text and colours are kept too, for tablets that show text only.
- **Presses:** key down and key up both go to Companion (`KEY-PRESS PRESSED=true/false`),
  so Companion's press and release actions work. Pausing keys on the computer also
  pauses Companion keys.
- **Connection:** pings every 2 s, reconnects with backoff (1 s up to 15 s), re-registers
  every tablet after a reconnect. While Companion is unreachable the tablet's first key
  says "Companion offline"; Settings and the Buttons page show the connection state.
- **Code:** `host/crates/companion` (wire format and client, tested against a fake
  Companion), `host/crates/host/src/companion.rs` (bridge to the deck), and the deck's
  `RemoteSurfaces` hook. Written from Companion's published Satellite API documentation.

Not done (Companion):
- The WebSocket transport (port 16623); TCP covers every Companion since 2.x.
- Advanced layouts (`LAYOUT_MANIFEST`), so no Companion rotary encoders or LED rings yet;
  PerritoDeck sliders don't map to anything in Companion.
- Button subscriptions (`ADD-SUB`): showing single Companion buttons on an ordinary
  PerritoDeck page.
- Tested only against a fake Companion. A check against a real Companion 3.x/4.x is the
  next step.

## Stream Deck plugins

Many people own Stream Deck plugins (`.sdPlugin`), and most current ones are Node.js
programs written against Elgato's public SDK. PerritoDeck runs them next to its own
plugins, so their actions can go on a tablet key.

- **Add one:** Plugins → "Stream Deck plugins" → "Add a plugin file…" (a
  `.streamDeckPlugin`, which is a zip) or "From a folder…" (a `.sdPlugin` folder). They
  live in `<settings folder>/sdplugins/`.
- **Use it:** the key editor lists each plugin's key actions under "<name> (Stream Deck
  plugin)". In a profile: `action = { type = "sdPlugin", plugin = "<plugin UUID>",
  action = "<action UUID>", settings = { … } }`.
- **How it runs** (from Elgato's published plugin documentation, no code from other
  projects): the host listens on a WebSocket on 127.0.0.1, starts the plugin with
  `-port -pluginUUID -registerEvent -info` (Node.js plugins with `node`; native plugins
  on Windows and macOS), and the plugin registers. Supported events: `deviceDidConnect`,
  `deviceDidDisconnect`, `willAppear`, `willDisappear`, `keyDown`, `keyUp`,
  `didReceiveSettings`, `didReceiveGlobalSettings`. Supported commands: `setTitle`,
  `setImage` (PNG, JPEG, GIF, BMP, WebP and SVG, as data URLs or plugin files),
  `setState`, `showOk`, `showAlert`, `setSettings`, `getSettings`, `setGlobalSettings`,
  `getGlobalSettings`, `openUrl` (web addresses only) and `logMessage`.
- **Keys:** one action instance (context) per profile key, shared by every tablet that
  shows it, like PerritoDeck's toggles. The plugin's image fills the key; its title (or
  the profile's) is drawn over it. The manifest's state images are the defaults. Settings
  a plugin saves are kept in `sdplugins/settings.json`.
- **Settings pages (property inspectors):** the key editor's "Open its settings" shows
  the action's own HTML page in a frame. The host serves it from the plugin folder on
  127.0.0.1 (URLs carry a per-start secret), adds the call to
  `connectElgatoStreamDeckSocket`, and the page talks to the plugin over the same socket
  (`registerPropertyInspector`, `setSettings` → the plugin gets `didReceiveSettings`,
  `sendToPlugin` / `sendToPropertyInspector`, global settings, `propertyInspectorDidAppear`
  and `…Disappear`). The socket accepts the settings server's origin and no other.
  Tested with a fixture page, over WebSocket and in the real app on Linux (WebKitGTK).
- **Supervision:** a plugin that exits is restarted with backoff and given up after five
  crashes; the Plugins page shows the state ("Needs Node.js", "Can't run here: …").
- **Programs a plugin starts (Windows):** each plugin runs in a job object with
  kill-on-close and only explicit breakaway, so everything it leaves behind ends with it
  when it stops, restarts or is removed. `openUrl` is opened by the host, so a browser it
  opens stays. But a program the plugin starts itself with `child_process` ends with the
  plugin, because libuv can't ask for breakaway. Elgato's SDK has no other way to ask
  for one, and we keep the job as it is: dropping it for these plugins would leave their
  helpers running after Quit. PerritoDeck's own Node SDK has `openForUser` and
  `startForUser` for this (the plugin protocol's `open`).
- **Tests:** `host/crates/sdcompat/tests` runs a small Node.js fixture plugin against a
  real deck (needs Node 22+ for its built-in WebSocket; skipped otherwise).

Not done (Stream Deck plugins):
- Dials, touch strips (`Encoder` actions are hidden), multi-actions, `switchToProfile`,
  `sendToPropertyInspector`, `titleParametersDidChange`, application monitoring.
- The old HTML/JavaScript plugins (they need a browser engine) and native plugins on
  Linux.
- Not yet tried with real published plugins.
- Before shipping: review how the app may name Elgato's product (nominative "made for
  the Elgato Stream Deck" only; no logos) and each plugin's own license terms.

## iPhone and iPad app

`ios/`: Perrito for iOS/iPadOS 16+, at parity with the Android app: Bonjour discovery,
pairing by code or QR (in-app scanner or the Camera app via `perritodeck://`), the same
certificate pinning, credentials in the Keychain, the deck with host-drawn keys,
sliders, multi-touch, haptics and ✓/⚠ flashes, kept awake while shown. `PerritoKit`
(a Swift package with everything but the UI) has XCTest tests against the protocol
fixtures; the SwiftUI app is generated with XcodeGen (`ios/project.yml`).

**It has never been compiled**: the cloud session has no Swift toolchain and the
maintainer's laptop runs Windows. First build: a rented Mac (Scaleway Mac mini M2, about
€4 for the 24-hour minimum) linked to a Claude session, or the `iOS` workflow (manual and
weekly only, because macOS minutes cost ten times Linux). The protocol now has
`platform: "ios"`.

Not done (iOS): USB connection (iOS has no `adb reverse`), App Store/TestFlight (needs the
$99/yr Apple Developer Program), localization, a real app icon.
