# PerritoDeck protocol v1

How the tablet apps (Android and iOS) and the desktop host talk. The machine-readable source of truth is [`schema/v1.schema.json`](schema/v1.schema.json); [`fixtures/`](fixtures/) holds one golden example per message, and the Rust, Kotlin and Swift test suites decode and re-encode every fixture to prove they agree.

## Transport

- WebSocket over **TLS** (`wss://`), **text frames, one JSON message per frame**. Default port **46710**.
- The host's certificate is self-signed; clients **pin its SHA-256 fingerprint** (base64url, no padding), learned when pairing. That fingerprint is also the host's id.
- WebSocket upgrades carrying an `Origin` header are refused, so web pages can't talk to the host.
- `perritodeck-host --insecure` (loopback only) speaks plain `ws://` with no pairing, for development.

Reaching the host:
  - Android emulator: `10.0.2.2:46710`
  - USB: `adb reverse tcp:46710 tcp:46710`, then `127.0.0.1:46710` on the tablet
  - Wi-Fi: found automatically over mDNS (`_perritodeck._tcp`), or by IP address

## Pairing

1. The host opens a pairing window: a 6-digit code valid for 5 minutes, single use, 5 wrong guesses and it's gone. It shows the code and a QR link `perritodeck://pair?v=1&n=<name>&a=<ip:port>…&fp=<fingerprint>&c=<code>`.
2. The client connects over TLS. From a QR link it pins `fp` straight away; with a typed code it shows the first 6 bytes of the certificate fingerprint (e.g. `3FA9 07C2 D41E`) for the user to compare with the host's screen.
3. The client sends `pair {code, deviceId, deviceName}` as its first message. The host replies `paired {deviceSecret, hostId, hostName}` and closes, or `error pairingFailed`.
4. From then on the client connects with the pinned fingerprint and sends `hello` including `deviceSecret`. The host stores only a SHA-256 of the secret. Unknown or revoked devices get `error notPaired`.

## Envelope

```json
{ "v": 1, "type": "keyDown", "id": 42, "payload": { "keyIndex": 3, "clientTimestampMs": 1760169600123 } }
```

| Field | Meaning |
|---|---|
| `v` | Protocol major version. A peer that receives a different major sends `error` `unsupportedVersion` and closes. |
| `type` | Message type (below). **Unknown types are ignored**, so minor additions never break older peers. |
| `id` | Optional correlation id. Replies echo it (`keyAck` echoes the `keyDown`'s id). |
| `payload` | Type-specific object. Unknown fields are rejected by the schema but decoders should tolerate them in practice. |

Names follow the Elgato Stream Deck SDK where a counterpart exists (`keyDown`, `keyUp`, `setTitle`, `setImage`).

## Session flow

```
client                                   host
  │ ── hello {deviceId, screen, caps, deviceSecret} ──►│  check the device is paired
  │ ◄── welcome {heartbeatIntervalMs} ───────────│
  │ ◄── layout {cols, rows, keys[]} ─────────────│
  │ ── keyDown {keyIndex} id=1 ─────────────────►│  log + dispatch
  │ ◄── keyAck {hostProcessingUs} id=1 ──────────│  client measures round trip
  │ ── keyUp {keyIndex} id=2 ───────────────────►│
  │ ◄── keyAck id=2 ─────────────────────────────│
  │ ── heartbeat {seq, sentAtMs} ───────────────►│  every heartbeatIntervalMs
  │ ◄── heartbeatAck {seq, sentAtMs} ────────────│
  │ ◄── setTitle / setImage (any time) ──────────│
```

- The first client message **must** be `hello` or `pair`; anything else gets `error` `protocolViolation` and a close.
- The host closes a connection that sends nothing for 3 × `heartbeatIntervalMs`, however much the host is sending to it, so a client sends `heartbeat` every interval even while messages keep arriving.
- A new `hello` with the same `deviceId` replaces the older connection.

## Messages

| Type | Direction | Payload |
|---|---|---|
| `hello` | C → H | `deviceId`, `deviceName`, `appVersion`, `platform` (`android`, `ios`, `web` or `other`), `osVersion`, `screen {widthPx, heightPx, densityDpi}`, `capabilities[]`, `requestedGrid?`, `deviceSecret?`, `networkFailed?` (true over a USB cable when the host's network addresses didn't answer; `token?` is deprecated) |
| `pair` | C → H | `code`, `deviceId`, `deviceName` — first message instead of `hello` |
| `paired` | H → C | `deviceSecret`, `hostId` (certificate fingerprint), `hostName`; then the host closes |
| `welcome` | H → C | `hostName`, `hostVersion`, `protocolVersion`, `heartbeatIntervalMs` |
| `layout` | H → C | `cols`, `rows`, `keys[] {index, title, bgColor, textColor, control?, value?}`; `control` is `"key"` (default) or `"slider"` (only to clients with the `sliders` capability), `value` 0–100 for sliders |
| `keyDown` / `keyUp` | C → H | `keyIndex` (row-major), `clientTimestampMs` |
| `keyAck` | H → C | `keyIndex`, `hostProcessingUs` |
| `heartbeat` / `heartbeatAck` | C → H / H → C | `seq`, `sentAtMs` |
| `setTitle` | H → C | `keyIndex`, `title` |
| `setImage` | H → C | `keyIndex`, `image` (PNG/WebP data URL, or `null`). |
| `showOk` / `showAlert` | H → C | `keyIndex`, `message?` — flash ✓ or ⚠ on the key |
| `sliderChange` | C → H | `keyIndex`, `value` (0–100). At most ~20 a second while dragging, then once on release; no reply |
| `setValue` | H → C | `keyIndex`, `value` — move a slider to match a change made elsewhere |
| `error` | both | `code` (`unauthorized`, `unsupportedVersion`, `badMessage`, `protocolViolation`, `internal`, `notPaired`, `pairingFailed`), `message` |

## Changing the protocol

1. Edit the schema.
2. Add or update a fixture in `fixtures/`.
3. Run `python3 protocol/tools/validate.py protocol` (needs `pip install jsonschema`; on Windows, `py -3` instead of `python3`).
4. Update the types in all three apps. Each test suite decodes and re-encodes every fixture, so it fails until its types match:

   | App | Types | Tests |
   |---|---|---|
   | Host (Rust) | `host/crates/protocol/src/lib.rs` | `cargo test -p perritodeck-protocol` in `host/` |
   | Android (Kotlin) | `android/app/src/main/java/dev/perritodeck/client/protocol/Protocol.kt` | `.\gradlew.bat testDebugUnitTest` (or `./gradlew`) in `android/` |
   | iOS (Swift) | `ios/PerritoKit/Sources/PerritoKit/Protocol.swift` | `swift test` in `ios/PerritoKit` (needs a Mac; CI runs it in the iOS workflow) |

   New fixtures are picked up by themselves: each suite reads every `.json` file in `fixtures/` (`host/crates/protocol/tests/fixtures.rs`, `android/app/src/test/java/dev/perritodeck/client/protocol/ProtocolFixturesTest.kt`, `ios/PerritoKit/Tests/PerritoKitTests/ProtocolFixturesTests.swift`).

Adding a message type or optional field is a minor change. Removing or renaming anything bumps `v`.
