# Architecture

PerritoDeck has two halves:

- **Host** — a desktop app (Windows, macOS, Linux) that owns all configuration, renders every key image, runs actions and plugins.
- **Client** — an Android app that is a "dumb surface": it displays what the host sends and reports touches. It caches the last layout so reconnects are instant, but holds no logic.

```
┌──────────────── Host (Rust) ─────────────────┐        ┌──── Android client (Kotlin) ────┐
│ Tauri UI (Phase 3)                           │        │ Connect screen                  │
│   │                                          │        │ Deck screen (Compose grid)      │
│ perritodeck-core                             │  WS    │   │                             │
│   device registry · profiles · renderer ─────┼───────►│ protocol client (OkHttp WS)     │
│   action dispatch · plugin manager ◄─────────┼────────┤   keyDown/keyUp/dial/touch      │
│ perritodeck-protocol (serde types)           │ JSON   │ protocol types (kotlinx.serial.)│
└──────────────────────────────────────────────┘        └─────────────────────────────────┘
           ▲                                                        ▲
           └──────── protocol/schema + protocol/fixtures ───────────┘
                     (single source of truth, tested by both sides)
```

## Transports
1. LAN WebSocket (Phase 1 manual IP; Phase 2 mDNS `_perritodeck._tcp` + QR pairing).
2. USB via `adb reverse tcp:46710 tcp:46710` — the client connects to `127.0.0.1` on the tablet.
3. Emulator — the host's loopback is `10.0.2.2` from inside the Android emulator.

## Repository layout
```
/host       Rust workspace: protocol, core, host (bin), fakedeck (bin)
/android    Android client
/protocol   JSON Schema, golden fixtures, protocol docs
/plugins    Plugin SDKs and first-party plugins (Phase 5)
/website    perritodeck.dev (Phase 6)
/docs       Architecture, decisions, plans, security
```

See `docs/plan/` for the phase plans and `docs/DECISIONS.md` for why things are the way they are.
