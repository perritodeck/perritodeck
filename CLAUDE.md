# PerritoDeck — notes for Claude

An open-source Stream Deck alternative. **Perrito** is the Android tablet app (`android/`).
The host runs on Windows, macOS and Linux (`host/`). The domain is perritodeck.dev, and the
GitHub repo is `perritodeck/perritodeck`, public since 2026-10-24 (`docs/launch-checklist.md`
has what's left before the first release).

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
| Website | `website/` | static; open `index.html`; notes, deploy steps and the cutover runbook in `docs/website.md`. Published to `perritodeck/perritodeck.github.io` by `scripts/publish_site.py` (docs/website.md), by the Site publish workflow on every push to `main` that touches `website/` or `plugins/` (its token, `SITE_PUBLISH_TOKEN`, expires 2027-10-26). `py -3 -m unittest test_build_catalog` in `plugins/` checks the pages and stylesheets for anything the CSP would block, and the policy `<meta>` tags at the top of every page; `py -3 -m unittest test_publish_site` in `scripts/` tests the publish script. Both run git: on Windows, check that `git` is Git for Windows (`(Get-Command git).Source`) |
| Licences | repo root | `py -3 scripts/third-party-notices.py --check` (rerun without `--check` after a dependency change); `cargo deny check licenses bans sources` in `host/` |

The host keeps its data in the per-user config folder: `%APPDATA%\perritodeck\PerritoDeck\config`
on Windows. `perritodeck-host paths` prints the exact locations. Only one host can run at a
time, because both the CLI and the app listen on port 46710.

## CI

Every push to `main` and every pull request starts one workflow, CI
(`.github/workflows/ci.yml`). Its first job lists the changed files with git and calls the
workflows whose checks read any of them (reusable: `on: workflow_call`); its last job,
**CI gate**, always runs and fails if any of those failed or was cancelled, or was skipped
though the first job asked for it. It's the one check the `main` ruleset should require
(`docs/launch-checklist.md` §5). Those lists are `NEEDS` in `ci.yml`: when a check starts
reading a file outside its list, add the path there.
A push run checks only what that push changed, so nothing cancels it (a newer commit on a
pull request does cancel the older run). On push and pull request the called workflows' jobs
and artifacts (the debug APK, the E2E screenshots) belong to the CI run.

The repo is public, so Actions are free on standard runners, macOS included; never use larger
runners, which are always billed. Since 2026-10-25 every platform runs on every push and pull
request that needs it: Host tests, Plugins, the app crate (`host/app/src-tauri`: fmt, clippy,
tests) and the desktop bundles on Windows, Linux and macOS, the desktop UI tests and the
real-app test, Android (with the reproducible-build check), Protocol, Licences, all four
emulators, and iOS (`ios.yml`, when `ios/` or `protocol/` changes). Weekly (Mondays) Host,
Plugins, Desktop app, E2E, iOS and Licences run again (advisories appear without any change
here, and `cargo deny check advisories` is a job of its own), and so does Site check. Actions → CI → Run
workflow runs every called workflow. Site publish
builds `website/` when a push or pull request touches it or `plugins/`, and publishes from
`main` once `SITE_PUBLISH_TOKEN` exists. Host CI sets `PERRITODECK_REQUIRE_NODE=1`, so the
Stream Deck plugin tests fail instead of skipping when Node 22 is missing. Linux can also be
checked locally in WSL (`wsl -d Ubuntu`): `scripts/linux-check.sh` runs the host and app
checks on a commit in about a minute. E2E screenshots and logs are workflow artifacts
(`e2e-<run name>`: `api22`, `api34`, `fire7`, `firehd10`), kept for 30 days: on the CI run's
summary page for a push or pull request, on the E2E run's for a weekly or manual one.
