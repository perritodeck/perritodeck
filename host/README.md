# PerritoDeck host

Rust workspace for the host: the `perritodeck-host` library that runs everything, the `perritodeck-host` command-line program for headless machines, and `perritodeck-fakedeck`, a tablet stand-in for testing. The desktop tray app in [`app/`](app/) is a separate Cargo workspace over the same library. What each of the thirteen crates in `crates/` does is in [docs/ARCHITECTURE.md](../docs/ARCHITECTURE.md#the-host-crates).

## Build requirements

- Rust (the pinned version in `rust-toolchain.toml` installs automatically via rustup)
- Linux only: `libxkbcommon-dev` (Debian/Ubuntu) or `libxkbcommon-devel` (Fedora), used for Wayland input

## Run it

```sh
cd host
cargo run -p perritodeck-host                 # Wi-Fi, emulator and USB tablets (TLS + pairing)
cargo run -p perritodeck-host -- --pair       # open a pairing window straight away
```

The first run creates the host's certificate and, since nothing is paired yet, shows a QR code and a 6-digit code. Scan the QR with the tablet's camera app (it opens Perrito), or tap **Pair with a code** in Perrito. Type `pair` in the host's window for a new code, `devices` to list paired tablets, `revoke <id>` to unpair one.

- **Wi-Fi:** the tablet finds the host automatically, or enter its IP
- **Android emulator:** `10.0.2.2`
- **USB:** with Android's `adb` installed, the host runs `adb reverse` for each tablet on a cable by itself, and Perrito reaches it at `127.0.0.1`

`perritodeck-host devices`, `revoke <id>` and `paths` work without starting the server. `--insecure` runs plain `ws://` with no pairing on loopback, for development.

## Your profile

The first run creates `profiles/default.toml` in the host's settings folder (`perritodeck-host paths` shows where) with commented examples of every kind of key: hotkeys, media keys, typing, URLs, commands, a toggle and a second page. Edit and save it; the tablet updates within a second. If an edit has a mistake, the host says what and keeps showing the last good version. Use `--profile <file>` to run a different one.

On Linux Wayland desktops, the host asks for permission to control the keyboard each time it starts (the desktop shows a prompt). X11, Windows and macOS don't ask, though macOS needs Accessibility permission once.

## Try an action

You can run one action directly, without a key or a tablet (it waits 3 s so you can focus a window):

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
