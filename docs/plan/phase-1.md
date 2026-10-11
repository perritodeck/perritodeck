# Phase 1 — Skeleton & protocol

**Goal:** a tablet shows a grid that the host sends it, and pressing a key shows up in the host's log within 30 ms. Nothing executes actions yet, so nothing here can harm the host machine.

## Scope

### 1. Monorepo & tooling
- Layout per the spec: `/host`, `/android`, `/protocol`, `/plugins` (empty placeholder), `/website` (placeholder), `/docs`.
- Root `README.md`, `CONTRIBUTING.md` (Conventional Commits), `.editorconfig`, `.gitignore`, issue templates. `LICENSE` once decided (open question 1).
- GitHub Actions:
  - `host.yml` — `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` on Linux, Windows, macOS.
  - `android.yml` — `./gradlew lint testDebugUnitTest assembleDebug`, upload the debug APK as an artifact (so it can be sideloaded onto the Kindle without a local Android SDK).
  - `protocol.yml` — validate every fixture against the JSON Schema.

### 2. Protocol v1 (`/protocol`)
- **Transport:** WebSocket, text frames carrying JSON. One message per frame.
- **Envelope:** `{ "v": 1, "type": "<messageType>", "id"?: <u32>, "payload": { … } }`. Unknown `type` values are ignored (forward compatibility); unknown major `v` closes the connection with a clear error.
- **Phase 1 message set:**
  - Client → host: `hello` (deviceId, deviceName, appVersion, screen {widthPx, heightPx, dpi}, capabilities [`keys`, `haptics`, …], requestedGrid {cols, rows}), `keyDown` / `keyUp` (keyIndex, clientTimestampMs), `heartbeat`.
  - Host → client: `welcome` (hostName, hostVersion, protocolVersion, heartbeatIntervalMs), `layout` (cols, rows, keys[] {index, title, bgColor, textColor}), `setTitle` (keyIndex, title), `error` (code, message).
- Messages are named after their Elgato SDK counterparts wherever one exists (`keyDown`, `setTitle`, `setImage`, …) so a compatibility layer later is mostly a shim.
- **Source of truth:** `protocol/schema/v1.schema.json` (JSON Schema 2020-12).
- **Contract tests:** `protocol/fixtures/*.json` — one golden file per message. The Rust crate and the Kotlin client both have tests that decode every fixture and re-encode it identically. This is how the two languages stay in sync without a code generator in Phase 1.
- Images (`setImage`) are defined in the schema now but implemented in Phase 2.

### 3. Host (`/host`, Rust workspace)
- `perritodeck-protocol` — serde types for the envelope and messages, fixture round-trip tests.
- `perritodeck-core` — async WebSocket server (tokio + tokio-tungstenite), device registry keyed by `deviceId`, heartbeat timeout, press-latency logging.
- `perritodeck-host` — headless CLI binary for now (Tauri UI arrives in Phase 3). Serves a hard-coded demo layout (3×5 on phones, 5×8 on tablets, chosen from `hello.screen`) and logs every press with round-trip latency.
- `perritodeck-fakedeck` — a CLI client that connects, sends `hello`, prints the layout, and simulates presses. Used in integration tests and CI so the host is testable with no Android device (idea borrowed from StreamController's fake decks).
- **Security for Phase 1:** binds to `127.0.0.1` by default. That's enough for the Android emulator (reaches the host as `10.0.2.2`) and for USB via `adb reverse`. LAN binding requires `--listen 0.0.0.0` **and** prints a random session token that the client must send in `hello`; real QR pairing with per-device keys replaces this in Phase 2.
- Default port: **46710** (configurable with `--port`).

### 4. Android client (`/android`)
- Kotlin, Jetpack Compose, single `:app` module, `applicationId dev.perritodeck.client`, launcher label "Perrito".
- Screens: **Connect** (host + port + optional token, remembered) and **Deck** (full-screen immersive grid, keep-screen-on).
- Renders the `layout` message as a grid; key press → `keyDown`, release → `keyUp`, with haptic tick. Multi-touch so several keys can be held at once.
- Auto-reconnect with backoff; shows a "disconnected" overlay but keeps the last layout visible.
- Dependencies limited to AndroidX, Compose, kotlinx.serialization, kotlinx.coroutines and OkHttp (WebSocket) — all Apache-2.0, all F-Droid-safe. No Google Play Services.
- Tests: fixture round-trip tests (JVM unit tests), Compose UI test for the grid.

### 5. Docs
- `docs/ARCHITECTURE.md`, `docs/DECISIONS.md`, `docs/SECURITY.md` (threat model stub), `protocol/README.md`.

## Out of scope (later phases)
Pairing/QR, mDNS, actions, host-rendered images, Tauri UI, plugins, packaging.

## Definition of done
- `cargo test` passes, including a fakedeck ↔ host integration test.
- Android CI builds an APK; it connects to the host from the emulator (laptop) and from the Kindle (tomorrow), shows the demo grid, and presses appear in the host log.
- Median press-to-log latency on LAN reported in the PR description.

## Open questions for the maintainer
1. **License.** GPL-3.0 lets us borrow from OpenDeck, StreamController and Stream-Pi but means plugin authors and companies must stay GPL-compatible. Apache-2.0 (or MIT) is friendlier for adoption and lets us share code with TilePad and Companion, but we can't copy from the GPL projects. My lean: **Apache-2.0** for the host/client, **MIT** for the plugin SDKs.
2. **TilePad.** It's MIT, the same Rust + Tauri 2 stack, WebSocket plugins with HTML inspectors, and its Android client is still WIP. Build PerritoDeck independently, or reach out about collaborating (e.g. PerritoDeck becomes the Android client and protocol, interoperating with TilePad)? My lean: build independently but keep the protocol open enough that a TilePad bridge is easy.
3. **Which Kindle?** Fire OS 5 is Android 5.1 (API 22), Fire OS 7 is Android 9 (API 28), Fire OS 8 is Android 11 (API 30). The spec says minSdk 26, which would exclude Fire OS 5 tablets. Old Kindles are exactly the e-waste audience, so if yours is Fire OS 5 I'd drop minSdk to 22.
4. **GitHub repo.** The GitHub CLI isn't logged in here. If you create an empty `perritodeck/perritodeck` repo, I can attach it to this session and push, which also gets CI building Android APKs.
