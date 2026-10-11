# PerritoDeck host

Rust workspace for the desktop host. During Phase 1 it's a headless command-line app; the Tauri UI arrives in Phase 3.

| Crate | What |
|---|---|
| `crates/protocol` | Wire types (serde), tested against `protocol/fixtures` |
| `crates/core` | WebSocket server, handshake, device registry, demo layout |
| `crates/host` | `perritodeck-host` binary |
| `crates/fakedeck` | `perritodeck-fakedeck` — a CLI "tablet" for testing without Android, plus end-to-end tests |

## Run it

```sh
cd host
cargo run -p perritodeck-host                      # loopback: emulator + USB tablets
cargo run -p perritodeck-host -- --listen 0.0.0.0  # LAN tablets (prints a session token)
```

Connecting a tablet:

- **Android emulator:** `ws://10.0.2.2:46710`
- **USB:** `adb reverse tcp:46710 tcp:46710`, then `ws://127.0.0.1:46710` on the tablet
- **Wi-Fi:** start with `--listen 0.0.0.0`, enter the printed address and token

The demo layout makes key 1 a live clock and key 2 a press counter; every press is logged.

## Test without a tablet

```sh
cargo run -p perritodeck-fakedeck                  # draw the grid, show live updates
cargo run -p perritodeck-fakedeck -- press 1 1 1   # press the counter three times
cargo run -p perritodeck-fakedeck -- bench -n 500  # round-trip latency stats
```

## Develop

```sh
cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test
```
