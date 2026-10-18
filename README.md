<p align="center"><img src="website/paw.svg" alt="" width="72"></p>

# PerritoDeck

**Turn an old Android tablet or phone into a Stream Deck–style control surface for Windows, macOS and Linux.** Free and open source, with no account and no cloud.

> El gato has a deck. Now the perrito does too.

PerritoDeck has two parts:

- **PerritoDeck** on your computer: a tray app where you lay out buttons by dragging, and the host that draws them and runs their actions.
- **Perrito** on the tablet: a full-screen grid of live buttons and sliders. It runs on Android 5.1 and newer, including Amazon Fire tablets. An iPhone/iPad version is coming.

**Status: pre-alpha.** It works end to end, tested on Windows 11 with an Amazon Fire 7 and in CI on Android emulators, but there's no published release yet. Expect rough edges and breaking changes. The [roadmap](docs/plan/) shows what's done.

## Screenshots

<p align="center">
  <img src="docs/assets/site/devices-hero.png" alt="Perrito on a tablet showing a grid of streaming keys (scenes, mic level, a stream timer) and on a phone showing homelab keys (server status, CPU, uptime checks)" width="80%">
</p>

The desktop app, in light or dark to match your system:

<p>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/screenshot-buttons-dark.png">
    <img src="docs/assets/screenshot-buttons.png" alt="The Buttons page: the profile menu and page tabs at the top, the tablet-shaped grid of keys, and the selected key's settings on the right" width="49%">
  </picture>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/screenshot-pair-dark.png">
    <img src="docs/assets/screenshot-pair.png" alt="The Pair a tablet page: a QR code, a six-digit code and the computer's fingerprint" width="49%">
  </picture>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/screenshot-live-values-dark.png">
    <img src="docs/assets/screenshot-live-values.png" alt="A key's settings with the live value list open under the title, ready to insert the live OBS scene" width="49%">
  </picture>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/screenshot-plugins-dark.png">
    <img src="docs/assets/screenshot-plugins.png" alt="The Plugins page: installed plugins with their status, Restart and Remove buttons, and the plugins that come with PerritoDeck" width="49%">
  </picture>
</p>

## Features

- **Drag-and-drop editor** that mirrors your tablet's grid: pages, copy/paste, undo/redo, and a "Try it" button. Edits show up on the tablet as you make them.
- **Actions:** hotkeys (recorded by pressing them), typing text, media keys, opening websites, files and programs, running commands, multi-step keys and toggles.
- **Built-in integrations:** OBS Studio and Home Assistant.
- **Sliders** for system volume, Home Assistant light brightness, OBS audio sources, or any program.
- **Live keys:** CPU, memory, Home Assistant states, OBS scene and recording status, and values from plugins; toggles can follow them. Animated GIF icons.
- **Profiles** per tablet, switched automatically by the app in front (Windows, macOS and Linux on X11). Export and import as `.perritoprofile` files.
- **Plugins** in Python or Node.js, with 24 first-party ones bundled with the app (Twitch, Spotify, OBS, Philips Hue, Elgato Key Light, audio devices, a soundboard, timers, a system monitor, Proxmox, TrueNAS, GitLab, Wake-on-LAN, weather and more; [the full list](plugins/README.md)) and an online catalogue.
- **AI assistants (MCP):** assistants that speak the Model Context Protocol, such as Claude Desktop and Claude Code, can look at your profiles and live values and, with your permission, press keys and build pages. It's off until you turn it on, and works only on this computer.
- **Experimental:** run many Stream Deck plugins (`.sdPlugin`, Node.js), and act as a Bitfocus Companion Satellite surface.
- **Private by design:** pairing with a one-time code or QR, TLS with a pinned certificate, Wi-Fi or a USB cable. No telemetry. The app itself only goes online for the update check and the plugin catalogue, both opt-in.
- **Command-line host** for headless machines, and `fakedeck`, a test client that needs no tablet.

Not there yet: the iOS app (written, not yet built), dials and touch strips as separate controls, plugins drawing their own key images, and signed installers.

## Install

The first release isn't out yet. When it is, everything will be on the
[download page](https://perritodeck.dev/download.html) and on
[GitHub Releases](https://github.com/perritodeck/perritodeck/releases):

- **Windows**: the installer. The first release (v0.1.0) is Windows and Android only;
  Linux and macOS join a later release once they've been tried on real hardware, and Linux
  builds from source until then.
- **Perrito for Android** as an APK, then on IzzyOnDroid and F-Droid (not on Google Play for now)

Until then, build from source (below).

## Quick start

1. Start **PerritoDeck** on your computer. It lives in the tray (menu bar on a Mac).
2. Open **Perrito** on the tablet. It finds your computer on the local network.
3. Choose **Pair a tablet** in the tray menu and scan the QR code with the tablet, or type the 6-digit code.
4. Drag actions onto keys in the editor. They appear on the tablet straight away.

The [user guide](https://perritodeck.dev/guide/) ([source](website/guide/)) covers
[getting started](website/guide/getting-started.html), [buttons](website/guide/buttons.html),
[integrations](website/guide/integrations.html), [plugins](website/guide/plugins.html) and
[troubleshooting](website/guide/troubleshooting.html).

### From source

You need Rust (the version in `host/rust-toolchain.toml` installs itself), Node.js 22+
for the desktop app, and JDK 17 for Android.

```sh
# Command-line host: prints a pairing code and QR
cd host && cargo run -p perritodeck-host -- --pair

# No tablet? Pair the test client with that code and watch the layout
cargo run -p perritodeck-fakedeck -- pair <code>
cargo run -p perritodeck-fakedeck -- watch

# Desktop app
cd host/app && npm ci && npx tauri dev

# Android app (debug APK in android/app/build/outputs/apk/)
cd android && ./gradlew assembleDebug
```

[CONTRIBUTING.md](CONTRIBUTING.md) lists the test commands for every part.

## Repository layout

| Path | What |
|---|---|
| [`host/`](host/) | Desktop host in Rust: the command-line host and shared crates, and the tray app in [`host/app/`](host/app/) (Tauri 2 + Svelte 5) |
| [`android/`](android/) | Perrito for Android (Kotlin + Jetpack Compose) |
| [`ios/`](ios/) | Perrito for iPhone and iPad (SwiftUI), not yet built |
| [`protocol/`](protocol/) | Wire protocol: JSON schema and golden fixtures |
| [`plugins/`](plugins/) | Plugin SDKs and first-party plugins (MIT) |
| [`website/`](website/) | [perritodeck.dev](https://perritodeck.dev) and the user guide ([how it's built and deployed](docs/website.md)) |
| [`docs/`](docs/) | [Architecture](docs/ARCHITECTURE.md), [decisions](docs/DECISIONS.md), [roadmap](docs/plan/), [security model](docs/SECURITY.md) |

## Plugins

Plugins are ordinary programs the host talks to over stdin/stdout, so you can write one
in any language. The [Python and Node.js SDKs](plugins/README.md) do the protocol work for
you; [plugins/PROTOCOL.md](plugins/PROTOCOL.md) documents it. The SDKs are MIT-licensed,
so your plugin can use any licence.

## Contributing

Bug reports, plugin ideas and pull requests are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md)
(sign off every commit with `git commit -s` for the DCO, and never take code from GPL
projects) and the
[Code of Conduct](CODE_OF_CONDUCT.md). Please report security problems privately as
described in [SECURITY.md](SECURITY.md).

## Licence

PerritoDeck is licensed under the [Apache License 2.0](LICENSE). The plugin SDKs and
first-party plugins in [`plugins/`](plugins/) are under the [MIT License](plugins/LICENSE).
Third-party components and their licences are listed in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

Copyright PerritoDeck contributors. Not affiliated with Elgato, Corsair or Bitfocus.
"Stream Deck" is a trademark of Corsair Memory, Inc., used here only to describe compatibility.
