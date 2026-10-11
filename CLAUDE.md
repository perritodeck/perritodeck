# PerritoDeck — notes for Claude

An open-source Stream Deck alternative. **Perrito** is the Android tablet app (`android/`).
The host runs on Windows, macOS and Linux (`host/`). The domain is perritodeck.dev, and the
GitHub repo is `perritodeck/perritodeck`, which is private for now.

Read first: `SPEC.md` (the original brief), `docs/plan/README.md` (phases and status),
`docs/DECISIONS.md` (append to it whenever a decision is made), and `docs/HANDOFF.md`
(where the work stands).

## Rules

- **No personal names or emails anywhere in the repo, commits or docs.** Commit as
  `dmang-dev <dev@perritodeck.dev>`; the git config is set in this clone. Use "PerritoDeck
  contributors" or "dmang-dev" for attribution. (The author list in `host/vendor/enigo` is
  upstream's and stays.)
- Licenses: Apache-2.0 for the project and MIT for `plugins/`. Never copy code from GPL
  projects (OpenDeck, StreamController, Stream-Pi).
- Android must stay F-Droid friendly: no Play Services, no proprietary SDKs, no
  analytics. minSdk 22 is provisional; it depends on the Kindle test.
- Ask before tagging releases, pushing tags, force-pushing or setting repo secrets.
  Releases and signing are on hold until the maintainer says go.
- Commit message trailer:
  `Co-Authored-By: Claude <noreply@anthropic.com>`

## Layout and commands

| Area | Path | Check it with |
|---|---|---|
| Wire protocol (JSON schema + golden fixtures) | `protocol/` | `python protocol/tools/validate.py` |
| Host workspace (Rust 1.97, pinned in `host/rust-toolchain.toml`) | `host/` | `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` |
| Command-line host | `host/crates/host` (`src/main.rs`; runtime in `src/lib.rs`) | `cargo run -p perritodeck-host -- --pair` |
| Test client with no tablet | `host/crates/fakedeck` | `cargo run -p perritodeck-fakedeck -- pair <code>` then `watch` |
| Desktop tray app (Tauri 2 + Svelte 5, its own Cargo workspace) | `host/app` | `npm ci`, `npm run check`, `npx playwright test`, `npx tauri dev` |
| Android app (Kotlin + Compose, JDK 17) | `android/` | `.\gradlew.bat testDebugUnitTest assembleDebug` |
| Website | `website/` | static; open `index.html` |

The host keeps its data in the per-user config folder: `%APPDATA%\perritodeck\PerritoDeck\config`
on Windows. `perritodeck-host paths` prints the exact locations. Only one host can run at a
time, because both the CLI and the app listen on port 46710.

## CI

The workflows in `.github/workflows` are `host`, `protocol`, `android`, `e2e` (real host +
API 22/34 emulators plus Fire-sized API 22 runs), `desktop-app` (bundles for all three OSes) and `release` (triggered by
a tag, not yet exercised). E2E screenshots are pushed to the `ci/e2e-<run name>` branches (`api22`, `api34`,
`fire7`, `firehd10`).
