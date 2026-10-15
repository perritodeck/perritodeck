# PerritoDeck — notes for Claude

An open-source Stream Deck alternative. **Perrito** is the Android tablet app (`android/`).
The host runs on Windows, macOS and Linux (`host/`). The domain is perritodeck.dev, and the
GitHub repo is `perritodeck/perritodeck`, which is private until launch
(`docs/launch-checklist.md`).

Read first: `SPEC.md` (the original brief), `docs/plan/README.md` (phases and status),
`docs/DECISIONS.md` (append to it whenever a decision is made), `docs/next-steps.md`
(where the work stands), `docs/HANDOFF.md` (Windows notes and testing without a person at
the keyboard), and `plugins/PROTOCOL.md` for plugins.

## Rules

- **No personal names or emails anywhere in the repo, commits or docs.** The maintainer's
  commits use `dmang-dev <dev@perritodeck.dev>`; anyone else commits under their own
  identity (a handle and a noreply address are fine) and never in someone else's name. Use
  "PerritoDeck contributors" or "dmang-dev" for attribution; the copyright holder is
  "PerritoDeck contributors". (The author list in `host/vendor/enigo` is upstream's and
  stays.)
- Licenses: Apache-2.0 for the project and MIT for `plugins/`. Never copy code from GPL
  projects (OpenDeck, StreamController, Stream-Pi).
- Android must stay F-Droid friendly: no Play Services, no proprietary SDKs, no
  analytics. minSdk 22 is provisional (older Fire tablets run Fire OS 5, API 22); raising
  it is its own decision.
- Ask before tagging releases, pushing tags, force-pushing or setting repo secrets.
  Releases wait until the maintainer says go. The Android signing pipeline is in place but
  signs nothing until the maintainer generates the release key and sets its secrets; until
  then a tag build fails.
- Commit with `git commit -s` (DCO, see `CONTRIBUTING.md`) under your own identity, so the
  commit ends with your `Signed-off-by:` line (never someone else's) and this trailer:
  `Co-Authored-By: Claude <noreply@anthropic.com>`

## Layout and commands

| Area | Path | Check it with |
|---|---|---|
| Wire protocol (JSON schema + golden fixtures) | `protocol/` | `python protocol/tools/validate.py protocol` |
| Host workspace (Rust 1.97, pinned in `host/rust-toolchain.toml`) | `host/` | `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` |
| Command-line host | `host/crates/host` (`src/main.rs`; runtime in `src/lib.rs`) | `cargo run -p perritodeck-host -- --pair` |
| Test client with no tablet | `host/crates/fakedeck` | `cargo run -p perritodeck-fakedeck -- pair <code>` then `watch` |
| Desktop tray app (Tauri 2 + Svelte 5, its own Cargo workspace) | `host/app` | `npm ci`, `npm run check`, `npx playwright test`, `npx tauri dev` |
| Android app (Kotlin + Compose, JDK 17) | `android/` | `.\gradlew.bat testDebugUnitTest assembleDebug` |
| Plugin runtime (host side) | `host/crates/plugins` | `cargo test -p perritodeck-plugins` |
| Bitfocus Companion client (Satellite API) | `host/crates/companion` | `cargo test -p perritodeck-companion` |
| Stream Deck plugin host (`.sdPlugin`) | `host/crates/sdcompat` | `cargo test -p perritodeck-sdcompat` (needs Node 22+) |
| Plugin SDKs and first-party plugins (Python, Node) | `plugins/` | `python3 plugins/test_all.py`; `python3 plugins/build_catalog.py` builds the online catalogue |
| iOS/iPadOS app (SwiftUI + `PerritoKit` package, needs a Mac) | `ios/` | `cd ios/PerritoKit && swift test`; `cd ios && xcodegen` then Xcode |
| Website | `website/` | static; open `index.html`. `py -3 -m unittest test_build_catalog` in `plugins/` checks the pages and stylesheets for anything the CSP in `website/_headers` would block |
| Licences | repo root | `py -3 scripts/third-party-notices.py --check` (rerun without `--check` after a dependency change); `cargo deny check licenses bans sources` in `host/` |

The host keeps its data in the per-user config folder: `%APPDATA%\perritodeck\PerritoDeck\config`
on Windows. `perritodeck-host paths` prints the exact locations. Only one host can run at a
time, because both the CLI and the app listen on port 46710.

## CI

While the repo is private, Actions minutes are billed (macOS at ten times Linux), so pushes
run only Host tests on Windows, the desktop UI tests, Android, Protocol, Plugins and two
emulators (API 22, Fire 7). Linux/macOS host tests, desktop bundles, the real-app test and
the other emulators run weekly (Mondays) or by hand (Actions → workflow → Run workflow).
Test Linux locally before pushing, and batch pushes. Since 2026-10-11 no run starts at all (a
billing block the maintainer is leaving until the repo goes public, when Actions are free),
so until the flip every check runs locally: Linux in WSL (`wsl -d Ubuntu`), and the emulator
tests not at all. E2E screenshots and logs are workflow
artifacts (`e2e-<run name>`: `api22`, `api34`, `fire7`, `firehd10`) on each run's summary
page, kept for 30 days.
