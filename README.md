# PerritoDeck

**Turn any Android tablet or phone into a Stream Deck–style control surface for Windows, macOS and Linux.** Free and open source.

> El gato has a deck. Now the perrito does too.

PerritoDeck has two parts:

- **Perrito** (Android) — a full-screen grid of live-updating buttons, dials and touch strips. Works on old tablets, including Fire OS.
- **PerritoDeck Host** (desktop) — configures your layouts, renders the buttons and runs the actions: hotkeys, launching apps, media control, OBS, Home Assistant, MQTT, webhooks and plugins.

**Status:** pre-alpha. See [`docs/plan/`](docs/plan/) for the roadmap.

## Repository layout

| Path | What |
|---|---|
| [`host/`](host/) | Desktop host (Rust) |
| [`android/`](android/) | Android client (Kotlin + Jetpack Compose) |
| [`protocol/`](protocol/) | Wire protocol schema and golden fixtures |
| [`plugins/`](plugins/) | Plugin SDKs and first-party plugins |
| [`website/`](website/) | Source for [perritodeck.dev](https://perritodeck.dev) |
| [`docs/`](docs/) | Architecture, decisions, plans, prior art |

## Principles

- No telemetry, no accounts, no cloud. Your tablet talks only to your computer.
- F-Droid–clean: no Google Play Services or proprietary libraries.
- Not affiliated with Elgato or Corsair.
