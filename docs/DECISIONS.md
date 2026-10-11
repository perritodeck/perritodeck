# Decisions

One line per decision, newest last. Format: `YYYY-MM-DD — decision — reason`.

- 2026-10-11 — Project name PerritoDeck; launcher name "Perrito"; ids `dev.perritodeck.*` — bare "Perrito" is unsearchable and its GitHub name is taken; `perritodeck.dev` and the `perritodeck` GitHub org are owned.
- 2026-10-11 — Host in Rust (Tauri 2 for UI later) — small binaries, good cross-platform input/audio crates, same stack as OpenDeck/TilePad so contributors can move between them.
- 2026-10-11 — Android client in Kotlin + Compose, no Google Play Services — F-Droid requires it; old tablets (incl. Fire OS) lack GMS anyway.
- 2026-10-11 — Protocol is JSON over WebSocket with Elgato-style message names — easy to debug, works in browsers, makes an Elgato-SDK shim mostly a rename.
- 2026-10-11 — Golden fixtures instead of codegen for cross-language types in Phase 1 — no extra toolchain; both test suites prove they agree on the wire format.
- 2026-10-11 — Host renders key images; client only displays — mirrors Stream Deck and Companion Satellite, keeps the client trivial and lets one client speak both protocols.
- 2026-10-11 — Phase 1 host binds to 127.0.0.1 by default; LAN requires an explicit flag plus a session token — never allow unauthenticated remote control, even before real pairing exists.
- 2026-10-11 — Default port 46710 — in the dynamic range, no well-known service found on it, and not used by Companion (16622/16623), Macro Deck (8193/8194) or OpenDeck (57116+).
- 2026-10-11 — License: Apache-2.0 for the project, MIT for everything under `plugins/` — permissive for adoption with a patent grant; MIT SDKs let plugin authors use any license. We will not copy code from GPL projects (OpenDeck, StreamController, Stream-Pi).
- 2026-10-11 — Android minSdk 22 (provisional, overrides spec's 26) — test Kindle's Fire OS version unknown; Fire OS 5 is API 22, and old tablets are the core audience. Revisit once known.
- 2026-10-11 — Build independently of TilePad — keep the protocol documented and open so a bridge is easy later.
- 2026-10-11 — A `fakedeck` CLI client ships alongside the host — lets CI and contributors test the host with no Android device.
