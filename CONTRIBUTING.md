# Contributing to PerritoDeck

Thanks for helping. Bug reports, plugin ideas, docs fixes and code are all welcome.

- **Questions and ideas:** open a [discussion or issue](https://github.com/perritodeck/perritodeck/issues/new/choose).
- **Bugs:** use the bug report form. In the desktop app, **Settings → Copy diagnostics** puts
  everything we need on your clipboard, and **Open logs folder** shows the log files.
- **Security problems:** please don't open an issue. See [SECURITY.md](SECURITY.md).

Before you start on something big, open an issue so we can agree on the approach. Design
decisions are recorded in [docs/DECISIONS.md](docs/DECISIONS.md) and the roadmap is in
[docs/plan/](docs/plan/); [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) explains how the
pieces fit.

## Ground rules

- **Licences.** Code is Apache-2.0; everything under `plugins/` (SDKs and first-party
  plugins) is MIT so plugin authors can use any licence. By contributing you agree your
  work is released under the licence of the folder you change.
- **No code from GPL projects.** Don't copy or translate code from OpenDeck,
  StreamController, Stream-Pi or any other GPL/AGPL/LGPL project, even a few lines.
  Reading their docs or behaviour is fine; write it yourself. New dependencies must be
  permissively licensed (`cd host && cargo deny check licenses bans sources` checks
  Rust crates; see [host/deny.toml](host/deny.toml)).
- **Android stays F-Droid clean.** No Google Play Services, Firebase, proprietary SDKs,
  analytics or crash reporters.
- **Privacy.** No telemetry, accounts or cloud services. Anything that talks to the
  internet must be opt-in.
- **No personal data in the repo.** Don't commit real names, emails, home paths, tokens or
  device logs that contain them. Use `/home/me/...` or `C:\Users\me\...` in examples.

## Sign your commits off (DCO)

We use the [Developer Certificate of Origin](https://developercertificate.org/) instead of
a CLA. Every commit needs a `Signed-off-by` line, which certifies you wrote the change or
have the right to submit it under the project's licence. `git commit -s` adds it:

```sh
git commit -s -m "fix(host): ..."
```

The name and email in the sign-off are whatever you commit with; a GitHub noreply address
and a handle are fine. Forgot it? `git commit --amend -s` fixes the last commit, and
`git rebase --signoff main` fixes every commit on your branch.

Sign-off is required from 2026-10-13 on. Commits made before then predate the rule and
don't carry one; they aren't rewritten.

## Building and testing

| Area | Path | Check it with |
|---|---|---|
| Wire protocol (JSON schema + golden fixtures) | `protocol/` | `python protocol/tools/validate.py protocol` |
| Host workspace (Rust, toolchain pinned in `host/rust-toolchain.toml`) | `host/` | `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` |
| Command-line host | `host/crates/host` | `cargo run -p perritodeck-host -- --pair` |
| Test client with no tablet | `host/crates/fakedeck` | `cargo run -p perritodeck-fakedeck -- pair <code>`, then `watch` |
| Desktop tray app (Tauri 2 + Svelte 5) | `host/app` | `npm ci`, `npm run check`, `npx playwright test`, `npx tauri dev` |
| Android app (Kotlin + Compose, JDK 17) | `android/` | `./gradlew testDebugUnitTest assembleDebug` (`.\gradlew.bat` on Windows) |
| Plugin runtime (host side) | `host/crates/plugins` | `cargo test -p perritodeck-plugins` |
| Bitfocus Companion client | `host/crates/companion` | `cargo test -p perritodeck-companion` |
| Stream Deck plugin host | `host/crates/sdcompat` | `cargo test -p perritodeck-sdcompat` (needs Node 22+) |
| Plugin SDKs and first-party plugins | `plugins/` | `python3 plugins/test_all.py` |
| iOS/iPadOS app (needs a Mac) | `ios/` | `cd ios/PerritoKit && swift test`; `cd ios && xcodegen`, then Xcode |
| Website | `website/` | static; open `index.html`, and see `docs/website.md`. `cd plugins && python3 -m unittest test_build_catalog` checks the pages and stylesheets for anything the CSP would block, and the policy `<meta>` tags at the top of every page; `cd scripts && python3 -m unittest test_publish_site` tests the publish script |
| Licences | repo root | `cd host && cargo deny check licenses bans sources`; `python3 scripts/third-party-notices.py --check` |

Only one host can run at a time: the command-line host and the desktop app both listen
on port 46710. `perritodeck-host paths` prints where settings and logs live.

**Linux builds** need `libxkbcommon-dev` for the host, and for the desktop app also
`libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev`
(Debian/Ubuntu names).

## Pull requests

- One topic per pull request. Keep unrelated formatting changes out.
- Commit messages follow `type(scope): summary` (`feat`, `fix`, `docs`, `test`, `build`,
  `ci`, `refactor`), as in the existing history.
- Run the checks for every area you touched. CI runs the main ones; Linux and macOS host
  tests and the slower emulator runs happen weekly, so test those locally if you changed
  platform code.
- Changes to the wire protocol need a schema update and golden fixtures in `protocol/`,
  and must keep older tablets working.
- Changes to host networking, pairing or actions: go through the checklist in
  [docs/SECURITY.md](docs/SECURITY.md#review-checklist-every-pr-touching-host-networking-or-actions).
- User-visible changes: update the guide in `website/guide/` where it applies.
- Adding a dependency: say why in the PR, and regenerate the notices with
  `python3 scripts/third-party-notices.py`.

## Plugins

Third-party plugins don't need to live in this repository. Write one with the
[Python or Node.js SDK](plugins/README.md) against [plugins/PROTOCOL.md](plugins/PROTOCOL.md)
and publish it anywhere. To propose a first-party plugin (bundled with the app and listed
in the online catalogue), open a plugin request first. First-party plugins:

- are MIT-licensed and use only the language's standard library,
- come with tests that run against a fake server (`python3 plugins/test_all.py`),
- never log tokens or passwords, and mark secret settings as `password`,
- bump their version in `perritodeck-plugin.json` whenever their code changes.

## Code of conduct

Everyone taking part is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
