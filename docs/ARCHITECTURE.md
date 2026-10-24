# Architecture

How PerritoDeck fits together, for anyone reading or changing the code. Why things are
the way they are is in [DECISIONS.md](DECISIONS.md), the threat model in
[SECURITY.md](SECURITY.md), and what's done and what's next in [plan/](plan/).

## The big picture

PerritoDeck has three kinds of parts:

- **The host**, on the computer (Windows, macOS, Linux). It owns everything: the paired
  tablets, the profiles, the plugins and the settings. It draws every key image, runs
  every action and keeps live values up to date. It comes as a tray app with an editor
  (the desktop app) and as a command-line program for headless machines; both run the
  same Rust library.
- **The tablet apps**, Perrito for Android and Perrito for iPhone and iPad. They're
  surfaces: they show the grid the host sends and report touches. They keep no profiles
  and run no actions, so the same deck looks the same on every tablet, and a new
  feature usually needs no tablet update.
- **Plugins**: separate programs the host starts and talks to over stdin/stdout. They
  add actions and live values without touching the host.

```
 Computer                                                      Tablet
┌───────────────────────────────────────────────────────┐      ┌───────────────────────────┐
│ Desktop app (Tauri 2 + Svelte 5) or perritodeck-host  │      │ Perrito                   │
│  └─ perritodeck-host library                          │      │  Android: Kotlin, Compose │
│      ├─ core: TLS WebSocket server, pairing, mDNS     ├─wss──┤  iOS: SwiftUI (written,   │
│      ├─ deck: profiles on each tablet, pages, toggles │ JSON │       not yet built)      │
│      ├─ render: key images at each tablet's size      │      │  grid, sliders, haptics   │
│      ├─ actions: hotkeys, text, media, open, launch   │      └───────────────────────────┘
│      ├─ integrations: OBS, Home Assistant             │ ──► OBS, Home Assistant
│      ├─ companion: Bitfocus Companion surfaces        │ ──► Companion (TCP 16622)
│      ├─ plugins: plugin processes (JSON over stdio)   │ ──► Python, Node.js or exe
│      ├─ sdcompat: Stream Deck plugins (.sdPlugin)     │ ──► Node.js or native
│      └─ mcp: AI assistants (127.0.0.1 only)           │ ◄── perritodeck-mcp (stdio)
└───────────────────────────────────────────────────────┘
      protocol/schema + protocol/fixtures: the one source of truth, tested
      by the Rust, Kotlin and Swift code
```

## Repository layout

| Path | What |
|---|---|
| `host/` | The Rust workspace: thirteen crates, listed below |
| `host/app/` | The desktop app: Tauri 2 with a Svelte 5 UI. Its Rust side (`src-tauri`) is its own Cargo workspace, so the headless host builds without the WebView and tray libraries |
| `android/` | Perrito for Android: Kotlin, Jetpack Compose, OkHttp, CameraX and ZXing for the QR scanner. minSdk 22 (Android 5.1, for older Fire tablets) |
| `ios/` | Perrito for iPhone and iPad: SwiftUI and the `PerritoKit` package, no third-party dependencies. Written but not yet built ([ios/README.md](../ios/README.md)) |
| `protocol/` | The wire protocol: JSON Schema, golden fixtures and a validator ([protocol/README.md](../protocol/README.md)) |
| `plugins/` | The plugin protocol, the Python and Node.js SDKs, the 24 first-party plugins and the catalogue builder. MIT-licensed, unlike the rest (Apache-2.0) |
| `website/` | perritodeck.dev: plain HTML, CSS and a little JavaScript, no build step; the user guide, and the plugin catalogue page |
| `fastlane/` | Store text and images for F-Droid, IzzyOnDroid and Google Play |
| `packaging/` | Draft Flatpak and AUR recipes |
| `scripts/` | Release versioning, third-party notices, the Android emulator test, screenshots |
| `docs/` | This file, decisions, plans, the security model, distribution and release notes |

## The host crates

| Crate | What it does |
|---|---|
| `perritodeck-protocol` | The wire types (serde), decoded and re-encoded against every fixture in its tests |
| `perritodeck-core` | The WebSocket server over TLS (rustls) with the host's self-signed certificate (rcgen), the `hello`/`pair` handshake, pairing codes and the paired-device list, heartbeats, mDNS advertisement (`_perritodeck._tcp`) and the pairing link shown as a QR code |
| `perritodeck-profile` | Profiles as TOML files: pages, keys at `[row, column]`, toggles with states, sliders, validation, the commented starter profile. Edits from the app change the file in place and keep comments (`toml_edit`) |
| `perritodeck-deck` | Runs profiles on the tablets: which profile each tablet shows, its current page and Back history, key presses to actions, toggle states shared by every tablet showing them, live values in titles, and reloading a profile when its file changes |
| `perritodeck-render` | Draws key images (background, icon, title in the bundled Inter font) at each tablet's real pixel size with tiny-skia and ab_glyph, so every client shows exactly the same thing; GIF icons animate |
| `perritodeck-actions` | What a key does: hotkeys and typed text (through `enigo`, patched in `host/vendor/enigo`), media keys, opening URLs and files, launching programs, commands, several actions in a row, a random one, slider levels. On Windows, plugin processes run in job objects so everything they start ends with them |
| `perritodeck-integrations` | Built-in OBS Studio (obs-websocket 5) and Home Assistant, set up in `integrations.toml` |
| `perritodeck-companion` | A Bitfocus Companion Satellite API client: a tablet showing a profile with `companion = true` appears in Companion as a surface |
| `perritodeck-plugins` | Starts plugins, restarts them with a growing delay if they crash, sends them key presses and settings, and passes on the live values they publish ([plugins/PROTOCOL.md](../plugins/PROTOCOL.md)) |
| `perritodeck-sdcompat` | Runs Stream Deck plugins (`.sdPlugin`): Node.js plugins (Node 22 or newer) and native ones on Windows and macOS. They connect back over a WebSocket on 127.0.0.1, and their property inspectors are served to the app over HTTP on 127.0.0.1. Written from Elgato's public documentation; experimental |
| `perritodeck-mcp` | A Model Context Protocol server, written from the specification: Streamable HTTP on 127.0.0.1:46711 with a bearer token, stdio, and `perritodeck-mcp`, a small bridge program assistants start, which relays stdio to the running host |
| `perritodeck-host` | The running host as a library (`Host::start` loads everything and wires server events to the deck), shared by the desktop app and the `perritodeck-host` command line. Also here: USB tablets (`adb reverse`), switching profiles by the app in front, live values such as CPU and memory, the online plugin catalogue, the opt-in update check, diagnostics, logging and the app's settings |
| `perritodeck-fakedeck` | A command-line client that pairs, watches and presses keys like a tablet, for testing without one; the host's end-to-end tests use it too |

## How a key press travels

1. The tablet sends `keyDown` with the key's index (row-major), then `keyUp`.
2. `core` checks the device is paired and connected, answers each with `keyAck` (the
   tablet times the round trip from it), and hands the press to `deck`.
3. `deck` finds the key on that tablet's current page. A page key changes the page; a
   toggle moves to its next state; anything else runs its action.
4. The action runs in `actions`, or goes to the integration, plugin or Stream Deck
   plugin it names. A failure or warning comes back to the key as `showAlert` (⚠);
   commands and multi-actions, which can take a while, also confirm success with
   `showOk` (✓). Stream Deck plugins send either themselves.
5. When something changes what a key shows (a toggle, a live value, a plugin's image),
   `render` draws the key again and the host sends `setImage` or `setTitle` to every
   tablet showing it.

Sliders work the same way: the tablet sends `sliderChange` while the slider is dragged
(at most about 20 a second), and the host sends `setValue` when the level changes
somewhere else.

## The protocol

WebSocket over TLS, one JSON message per text frame, on port 46710. The schema in
`protocol/schema/v1.schema.json` is the source of truth, and `protocol/fixtures/` holds a
golden example of every message. The Rust, Kotlin and Swift test suites each decode and
re-encode every fixture, so the three implementations can't drift apart unnoticed.

- **Tablet to host:** `pair` or `hello` (always the first message), `keyDown`, `keyUp`,
  `sliderChange`, `heartbeat` and `error`.
- **Host to tablet:** `paired`, `welcome`, `layout`, `keyAck`, `setTitle`, `setImage`,
  `setValue`, `showOk`, `showAlert`, `heartbeatAck` and `error`.

Unknown message types are ignored, so adding one is a minor change; removing or renaming
anything would bump the protocol's major version (`v`).

Pairing: the host shows a six-digit code and a QR link carrying its certificate
fingerprint. The tablet pins that fingerprint (from the QR, or after the person compares
it by eye for a typed code), redeems the code with `pair`, and gets a per-device secret
that it sends in every later `hello`. The host keeps only a hash of the secret.
[protocol/README.md](../protocol/README.md) has the details.

How a tablet reaches the host:

- **Wi-Fi:** found over mDNS, or typed in by address.
- **USB:** when Android's `adb` is installed, the host runs `adb reverse` for every
  tablet on a cable, so Perrito reaches the computer at 127.0.0.1. Perrito knows both
  addresses and moves between them until one answers. When it ends up on the cable
  because the network addresses didn't answer, it says so in `hello`, and the host can
  point at a firewall that keeps Wi-Fi out.
- **Android emulator:** the computer is at 10.0.2.2.

## Ports and what listens

| Port | Who | Where |
|---|---|---|
| 46710 (TCP) | The host's TLS WebSocket for tablets | All addresses by default; loopback only when Wi-Fi is turned off in Settings |
| 5353 (UDP) | mDNS advertisement | Local network |
| 46711 (TCP) | The MCP server, only while it's turned on (off by default) | 127.0.0.1 |
| Two free ports each start | Stream Deck plugins: a WebSocket, and HTTP for property inspectors | 127.0.0.1 |

The host also connects out to OBS, Home Assistant and Companion when they're set up. It
goes online only to fetch the plugin catalogue (when someone looks online) and to check
for updates (when that's turned on).

## The desktop app

`host/app` is Tauri 2 with a Svelte 5 front end. The Rust side (`src-tauri/src/lib.rs`)
is a thin layer of Tauri commands over the `perritodeck-host` library; it adds the tray,
start at login and the window. The pages are Buttons (the editor: the tablet-shaped grid,
the key inspector, pages, copy and paste, undo), Tablets, Pair a tablet, Switching (rules
for the app in front), Plugins and Settings. Closing the window leaves the host running
in the tray.

`npm run dev` runs the UI in a browser against a mock host (`src/lib/mock.ts`), which is
also what the Playwright tests use. The installers carry the 24 first-party plugins and
`perritodeck-mcp`, which the app copies into its data folder so an assistant using it
doesn't keep the app's own program locked.

Only one host runs at a time, the desktop app or the command line, because both listen
on port 46710.

## Plugins

A plugin is a folder with `perritodeck-plugin.json` (id, name, version, runtime, actions
with the fields the editor asks for, and values) and a program the host starts: Python,
Node.js, or any executable. Host and plugin exchange one JSON object per line over
stdin/stdout: the host sends `hello`, settings, `run` for key presses, `watch` for values
with arguments, and `shutdown`; the plugin sends values, results, log lines and requests
to open something for the user. [plugins/PROTOCOL.md](../plugins/PROTOCOL.md) is the
reference, and the SDKs in `plugins/sdk-python` and `plugins/sdk-node` (standard library
only, no dependencies) do the protocol work.

All 24 first-party plugins are Python. `plugins/build_catalog.py` builds the online
catalogue: `index.json`, a browsable page and one reproducible zip per plugin with its
SHA-256. The app's Plugins → **Look online** reads it from `https://perritodeck.dev/plugins/`
and refuses a zip whose hash doesn't match.

## AI assistants (MCP)

The host can serve the Model Context Protocol so assistants on the same computer can read
profiles, tablets and live values, and, with separate permissions that are off by
default, press keys (`press`), change keys and pages (`edit`) and run any action (`run`).
It listens on 127.0.0.1 only and every request needs the token from Settings. Assistants
that only start a command get `perritodeck-mcp`, which relays stdio to that endpoint and
reads the port and token from the host's settings. Several MCP protocol versions are
served, from `2024-11-05` to the stateless `2026-07-28`.

## Where the host keeps its data

One per-user folder: `%APPDATA%\perritodeck\PerritoDeck\config` on Windows, the usual
config folder elsewhere (`perritodeck-host paths` prints it). In it:

| File or folder | What |
|---|---|
| `host-cert.der`, `host-key.der` | The host's certificate and private key; tablets pin the certificate |
| `devices.json` | Paired tablets, with a hash of each one's secret |
| `profiles/` | One TOML file per profile (`default.toml` first) |
| `app-settings.json` | Which tablet shows which profile, switching rules, network, MCP and other app settings |
| `integrations.toml` | OBS, Home Assistant and Companion addresses and credentials |
| `plugins/`, `plugin-settings/`, `plugin-data/` | Installed plugins, their settings and their own files |
| `sdplugins/` | Installed Stream Deck plugins and their settings |
| `mcp-token`, `bin/` | The MCP token and the copy of `perritodeck-mcp` assistants start |
| `logs/` | `perritodeck.log`, redacted |

The host writes its private files (the key, the device list, tokens, plugin settings, the
logs) owner-only on Linux and macOS; an `integrations.toml` you create yourself keeps the
permissions you give it. On Windows the per-user profile's permissions apply.
[SECURITY.md](SECURITY.md) has the details.

## The website

`website/` is plain HTML, CSS and a little JavaScript, with no build step and no
third-party requests: the home page, features, Stream Deck and Companion compatibility,
FAQ, download, privacy, and the user guide in `website/guide/`, which changes in the same
commits as the features it describes. A strict Content-Security-Policy, in a `<meta>` tag
at the top of every page, allows only the site's own files. GitHub Pages serves the site
from the public repository `perritodeck/perritodeck.github.io`, which holds only what
`scripts/publish_site.py` builds from `website/` and `plugins/` here. The plugin catalogue
in `website/plugins/` isn't committed: `plugins/build_catalog.py` generates it at each
publish (see [website.md](website.md)).

## Testing

- **Protocol:** `protocol/tools/validate.py` checks every fixture against the schema;
  the Rust, Kotlin and Swift suites decode and re-encode them.
- **Host:** `cargo test` in `host/`, including end-to-end tests that run a real host and
  connect `fakedeck` to it over TLS.
- **Desktop app:** `npm run check` and Playwright tests against the mock host; a CI job
  also drives the real app on Linux.
- **Android:** unit tests, and an emulator test (`scripts/e2e-android.sh`) that pairs a
  debug build with a real host and presses keys, on API 22 and API 34 emulators and at
  the screen sizes of two Fire tablets.
- **Plugins:** `plugins/test_all.py` runs every plugin's and SDK's tests; plugins that
  talk to services run against fakes of them.

[CONTRIBUTING.md](../CONTRIBUTING.md) lists the exact commands for every part.
