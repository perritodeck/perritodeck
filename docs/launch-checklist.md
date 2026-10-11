# Launch checklist: making the repository public

Everything to do, in order, to take `perritodeck/perritodeck` from private to public and
ship the first release. Steps marked **(maintainer)** change repository settings, secrets,
tags or anything visible to the world; only the maintainer does those.

Status as of 2026-10-11: steps 1–2 checked by a scan (results below); nothing else done.

## 1. History and personal data

A scan on 2026-10-11 of every commit in the local clone (78, including the unpushed
`main` commit and the local-only `v0.1.0` tag) and in a fresh mirror of GitHub (77,
including newer `ci/e2e-*` commits than the local clone had) found:

- **Authors and committers:** only `dmang-dev <dev@perritodeck.dev>` and
  `github-actions[bot]`. Trailers: only `Co-Authored-By: Claude ... <noreply@anthropic.com>`.
- **Names and emails in content:** only upstream ones that belong there: the enigo authors in
  `host/vendor/enigo/Cargo.toml` and the Tabler Icons copyright line.
- **Home paths:** only placeholders (`/home/me`, `C:\Users\me`, `/home/you`) and CI runner
  paths (`/home/runner/...`). The CI emulator logs (`logcat.txt`) contain only emulator
  data (SSID `AndroidWifi`, emulator accounts).
- **Images:** no PNG text/EXIF metadata.
- **Commit dates** carry a `-0700` UTC offset (73 commits). That hints at a time zone,
  not an identity, and isn't worth a rewrite. To stop it from now on, commit with
  `TZ=UTC` set (git records the offset from `TZ`), e.g. `export TZ=UTC` in the shell
  used for this repo.

Decision: **no history rewrite needed.**

- [ ] Re-run the scan right before flipping visibility, since commits keep landing:
  ```sh
  git log --all --format='%an <%ae> | %cn <%ce>' | sort -u
  git log -p --all | grep -oiE '[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}' | sort | uniq -c | sort -rn
  git log -p --all | grep -oiE '(/Users/|/home/|C:\\+Users\\+)[A-Za-z0-9._-]+' | sort | uniq -c
  ```
- [ ] **CI screenshot branches** (`ci/e2e-api22`, `ci/e2e-api34`, `ci/e2e-fire7`,
  `ci/e2e-firehd10`) hold emulator screenshots, `logcat.txt`, `host.log` and `gradle.txt`.
  They're clean today, but they're noise in a public repo and each push adds binary
  history. Consider deleting them before launch **(maintainer)** and uploading E2E
  screenshots as workflow artifacts instead (needs a change to `.github/workflows/e2e.yml`).
- If something personal is ever found: rewrite with
  [`git filter-repo`](https://github.com/newren/git-filter-repo) **before** going public
  (afterwards, forks and caches keep the old objects):
  ```sh
  git clone --mirror https://github.com/perritodeck/perritodeck perritodeck-mirror
  cd perritodeck-mirror
  # names/emails: a mailmap with "dmang-dev <dev@perritodeck.dev> <old@example.com>"
  git filter-repo --mailmap ../mailmap.txt
  # text in files: lines like "Old Name==>PerritoDeck contributors"
  git filter-repo --replace-text ../replacements.txt
  # whole files: git filter-repo --invert-paths --path path/to/file
  git push --force --mirror   # (maintainer) then everyone re-clones; re-create tags/releases
  ```

## 2. Secrets

- [x] [gitleaks](https://github.com/gitleaks/gitleaks) 8.21 over all history: four
  findings, none real: `protocol/fixtures/hello-paired.json` and
  `protocol/fixtures/paired.json` (example `deviceSecret`, commit `ca97afa`),
  `plugins/truenas/test_plugin.py` (fake API key, commit `24f1632`), and an emulator
  network key in `logcat.txt` on `ci/e2e-api34` (commit `7f02fcb`). Their fingerprints
  are in `.gitleaksignore`, so a re-run is clean.
- [ ] Re-run before launch: `gitleaks git --log-opts="--all" .`
- [ ] Check that no signing material is in the tree (`*.jks`, `*.keystore`, `*.p12`, `*.p8`
  are ignored or absent) and that the Android release key is backed up offline.
- [ ] Review repository and organisation secrets: only what `docs/releasing.md` lists.
  Secrets are not exposed to workflows triggered from forks, but check that no workflow
  uses `pull_request_target` with a checkout of the PR head (none does today).

## 3. Files in the repository

- [x] `LICENSE` (Apache-2.0), `plugins/LICENSE` (MIT), `THIRD_PARTY_NOTICES.md`
  (`scripts/third-party-notices.py`), `host/deny.toml`.
- [x] `README.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`,
  `.github/ISSUE_TEMPLATE/`, `.github/PULL_REQUEST_TEMPLATE.md`, `.github/CODEOWNERS`,
  `.github/FUNDING.yml` (commented out).
- [ ] Final screenshots in `docs/assets/`, referenced from the README.
- [ ] `docs/SECURITY.md` "Reporting" section points at `SECURITY.md` and
  dev@perritodeck.dev.
- [ ] Make sure dev@perritodeck.dev actually receives mail (it's the CoC, security and
  store contact).
- [ ] Decide what to do with session-oriented docs (`docs/HANDOFF.md`, `CLAUDE.md`):
  keep, trim, or move. They're harmless but read oddly to newcomers.
- [ ] Add `publish = false` to the `perritodeck-*` crates and set `wildcards = "deny"` in
  `host/deny.toml`; add `cargo deny check licenses bans sources` and
  `python3 scripts/third-party-notices.py --check` to CI.

## 4. Flip visibility **(maintainer)**

- [ ] Settings → General → Danger Zone → **Change visibility → Public**.
- [ ] Settings → General → Features: Issues on; Discussions on (optional, for Q&A);
  Wiki off; Projects as preferred.
- [ ] Description, website `https://perritodeck.dev`, topics (`stream-deck`, `macro-pad`,
  `android`, `tauri`, `rust`, `obs`, `home-assistant`, `fdroid`), social preview image.

## 5. Protect the repository **(maintainer)**

- [ ] **Private vulnerability reporting:** Settings → Code security → Private
  vulnerability reporting → Enable (`SECURITY.md` links to it).
- [ ] Dependabot alerts and security updates on; secret scanning and push protection on
  (free for public repos).
- [ ] **Branch protection / ruleset for `main`:** require a pull request (allow the
  maintainer to bypass while the team is one person), require status checks (Host tests
  on Windows, desktop UI tests, Android, Protocol, Plugins), block force pushes and
  deletion, require linear history.
- [ ] **Tag ruleset** for `v*`: only the maintainer can create, nobody can update or
  delete.
- [ ] Actions → General: "Require approval for first-time contributors" for fork PRs;
  workflow permissions default to read-only.
- [ ] Optional: [DCO app](https://github.com/apps/dco) to enforce sign-offs.
- [ ] Public repos get free standard GitHub-hosted runners, macOS included. Revisit the
  CI cost rules in `CLAUDE.md` (weekly-only Linux/macOS jobs, two emulators per push)
  and consider running more on every push.

## 6. Website and plugin catalogue (perritodeck.dev)

- [ ] Deploy `website/` (Cloudflare Pages as in `website/README.md`, or GitHub Pages now
  that the repo is public). With Cloudflare, a build command can publish the catalogue
  on every deploy: `python3 plugins/build_catalog.py website/plugins`
  (`website/plugins/` is git-ignored). Otherwise unzip the release's
  `perritodeck-X.Y.Z-plugin-catalogue.zip` into `website/plugins/` before each deploy.
- [ ] Custom domain `perritodeck.dev` + `www`, HTTPS enforced, HSTS.
- [ ] Check `https://perritodeck.dev/plugins/index.json` loads and that the desktop app's
  Plugins → "Look online" installs a plugin from it.
- [ ] Remove the "Not released yet" notice and point the home page's Download links at
  `download.html` (after step 7 publishes the release).
- [ ] Consider self-hosting the two web fonts instead of loading them from Google Fonts,
  in keeping with "nothing leaves your desk" (and GDPR rulings on Google Fonts).

## 7. First release **(maintainer)**

Follow [docs/releasing.md](releasing.md). In short:

- [ ] Signing secrets (Android at least; Apple and Windows optional).
- [ ] `python3 scripts/release-version.py X.Y.Z`, changelog in
  `fastlane/metadata/android/en-US/changelogs/<versionCode>.txt`.
- [ ] Generate the full licence texts for the release bundles:
  `python3 scripts/third-party-notices.py --full third-party-licenses.txt` (and wire
  this into `release.yml` so every asset carries it).
- [ ] Note: a local `v0.1.0` tag already exists (on `9f47509`, before most features). Decide
  whether the first public release is `v0.1.0` re-tagged on current `main` (delete the old
  tag first, only if it was never pushed) or `v0.2.0`.
- [ ] Tag, let the Release workflow build the draft, test installers on every OS and the
  APK on the Fire 7, publish.

## 8. Stores and package managers

See [docs/distribution.md](distribution.md) for each channel's requirements.

- [ ] **IzzyOnDroid:** request inclusion (needs the public repo and a release APK).
- [ ] **F-Droid:** merge request to `fdroiddata` from the draft in `docs/fdroid/`.
- [ ] **Google Play:** start the 12-tester, 14-day closed test early; data safety form;
  upload the `play-bundle` artifact.
- [ ] **Desktop:** winget (`wingetcreate new`), Flathub and AUR from `packaging/`,
  Homebrew cask once notarised.
- [ ] Amazon Appstore (optional, for Fire tablet owners).

## 9. Announcement

- [ ] Short demo video or GIF (tablet + editor), screenshots from `docs/assets/`.
- [ ] Blog-style post on the website or a pinned GitHub Discussion: what it is, what works,
  what doesn't yet, how to help.
- [ ] Post to r/streamdeck, r/selfhosted, r/homeassistant, r/obs, r/fdroid, Hacker News
  (Show HN), the Fediverse; tell the Home Assistant and OBS forums.
- [ ] Label a handful of `good first issue`s before announcing.
- [ ] Watch issues closely for the first week; be ready with a patch release.
