# PerritoDeck host

Rust workspace for the desktop host. During Phase 1 it's a headless command-line app; the Tauri UI arrives in Phase 3.

| Crate | What |
|---|---|
| `crates/protocol` | Wire types (serde), tested against `protocol/fixtures` |
| `crates/core` | WebSocket server, handshake, device registry, demo layout |
| `crates/actions` | What keys do: hotkeys, text, media keys, URLs, apps, commands, multi-actions (Phase 2 prototype) |
| `crates/host` | `perritodeck-host` binary |
| `crates/fakedeck` | `perritodeck-fakedeck` — a CLI "tablet" for testing without Android, plus end-to-end tests |

## Run it

```sh
cd host
cargo run -p perritodeck-host                 # Wi-Fi, emulator and USB tablets (TLS + pairing)
cargo run -p perritodeck-host -- --pair       # open a pairing window straight away
```

The first run creates the host's certificate and, since nothing is paired yet, shows a QR code and a 6-digit code. Scan the QR with the tablet's camera app (it opens Perrito), or tap **Pair with a code** in Perrito. Type `pair` in the host's window for a new code, `devices` to list paired tablets, `revoke <id>` to unpair one.

- **Wi-Fi:** the tablet finds the host automatically, or enter its IP
- **Android emulator:** `10.0.2.2`
- **USB:** `adb reverse tcp:46710 tcp:46710`, then `127.0.0.1` on the tablet

`perritodeck-host devices`, `revoke <id>` and `paths` work without starting the server. `--insecure` runs plain `ws://` with no pairing on loopback, for development.

The demo layout makes key 1 a live clock and key 2 a press counter; every press is logged.

## Try an action

Actions aren't wired to keys until Phase 2 profiles land, but you can fire one directly (it waits 3 s so you can focus a window):

```sh
cargo run -p perritodeck-host -- action '{"type":"hotkey","keys":"Ctrl+Shift+Esc"}'
cargo run -p perritodeck-host -- action '{"type":"text","text":"hello from perrito"}'
cargo run -p perritodeck-host -- action '{"type":"media","key":"playPause"}'
cargo run -p perritodeck-host -- action '{"type":"openUrl","url":"https://perritodeck.dev"}'
```

On macOS the first hotkey or text action asks for Accessibility permission for your terminal.

## Test without a tablet

```sh
cargo run -p perritodeck-fakedeck -- pair 123456   # pair using the code the host shows
cargo run -p perritodeck-fakedeck                  # draw the grid, show live updates
cargo run -p perritodeck-fakedeck -- press 1 1 1   # press the counter three times
cargo run -p perritodeck-fakedeck -- bench -n 500  # round-trip latency stats
```

## Develop

```sh
cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test
```
