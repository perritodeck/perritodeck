# PerritoDeck protocol v1

How the Android client and the desktop host talk. The machine-readable source of truth is [`schema/v1.schema.json`](schema/v1.schema.json); [`fixtures/`](fixtures/) holds one golden example per message, and both the Rust and Kotlin test suites decode and re-encode every fixture to prove they agree.

## Transport

- WebSocket, **text frames, one JSON message per frame**.
- Default port **46710**. The host listens on `127.0.0.1` unless started with `--listen`.
  - Android emulator → host: `ws://10.0.2.2:46710`
  - USB: `adb reverse tcp:46710 tcp:46710`, then `ws://127.0.0.1:46710` on the tablet
  - LAN: `ws://<host-ip>:46710` plus the session token the host prints

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
  │ ── hello {deviceId, screen, caps, token?} ──►│  validate token (if not loopback)
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

- The first client message **must** be `hello`; anything else gets `error` `protocolViolation` and a close.
- The host closes a connection that sends nothing for 3 × `heartbeatIntervalMs`.
- A new `hello` with the same `deviceId` replaces the older connection.

## Messages (Phase 1)

| Type | Direction | Payload |
|---|---|---|
| `hello` | C → H | `deviceId`, `deviceName`, `appVersion`, `platform`, `osVersion`, `screen {widthPx, heightPx, densityDpi}`, `capabilities[]`, `requestedGrid?`, `token?` |
| `welcome` | H → C | `hostName`, `hostVersion`, `protocolVersion`, `heartbeatIntervalMs` |
| `layout` | H → C | `cols`, `rows`, `keys[] {index, title, bgColor, textColor}` |
| `keyDown` / `keyUp` | C → H | `keyIndex` (row-major), `clientTimestampMs` |
| `keyAck` | H → C | `keyIndex`, `hostProcessingUs` |
| `heartbeat` / `heartbeatAck` | C → H / H → C | `seq`, `sentAtMs` |
| `setTitle` | H → C | `keyIndex`, `title` |
| `setImage` | H → C | `keyIndex`, `image` (PNG/WebP data URL, or `null`). Defined now, implemented in Phase 2. |
| `error` | both | `code` (`unauthorized`, `unsupportedVersion`, `badMessage`, `protocolViolation`, `internal`), `message` |

## Changing the protocol

1. Edit the schema.
2. Add or update a fixture in `fixtures/`.
3. Run `python3 protocol/tools/validate.py protocol` (needs `pip install jsonschema`).
4. Update the Rust types (`host/crates/protocol`) and Kotlin types (`android/app/src/main/java/dev/perritodeck/client/protocol`); both test suites will fail until they match the fixtures.

Adding a message type or optional field is a minor change. Removing or renaming anything bumps `v`.
