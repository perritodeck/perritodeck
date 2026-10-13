# Launch checklist: making the repository public

Everything to do, in order, to take `perritodeck/perritodeck` from private to public and
ship the first release. Steps marked **(maintainer)** change repository settings, secrets,
tags or anything visible to the world; only the maintainer does those.

Status as of 2026-10-13: steps 1–2 re-checked by a sweep of the whole history (results
below), and the ticked boxes are done. Everything else is open; `docs/next-steps.md` item 8
has the current state and the decisions it needs.

## 1. History and personal data

A sweep on 2026-10-13 of every commit on every ref (224: 220 on `main` plus the four
`ci/e2e-*` branches; there are no tags in this repository or on GitHub) found:

- **Authors and committers:** only `dmang-dev <dev@perritodeck.dev>` (219) and
  `github-actions[bot]` (5).
- **Trailers:** `Co-Authored-By: Claude … <noreply@anthropic.com>` on 219 commits, and a
  `Claude-Session: https://claude.ai/code/session_…` link on 81 of them (2026-10-11 to
  2026-10-13). They aren't secret, but they're public once the repo is; they stay (see
  the decision below). No commit so far has a `Signed-off-by` line: sign-off is required
  from 2026-10-13 on, and the earlier commits predate the rule (`CONTRIBUTING.md`).
- **Names and emails in content:** only upstream ones that belong there (the enigo authors
  in `host/vendor/enigo/Cargo.toml`, the Tabler Icons copyright line), `dev@`, noreply
  addresses and `example.com` placeholders.
- **Home paths:** only placeholders (`/home/me`, `/Users/me`, `C:\Users\me` and similar) and
  CI runner paths (`/home/runner/...`). The CI emulator logs (`logcat.txt`) contain only
  emulator data (SSID `AndroidWifi`, emulator accounts).
- **Images:** none of the 72 tracked images has text or EXIF metadata.
- **Commit dates** carry a `-0700` UTC offset (211 of 224 commits). That hints at a time
  zone, not an identity, and isn't worth a rewrite on its own. To stop it, commit with
  `TZ=UTC` set (git records the offset from `TZ`), e.g. `export TZ=UTC` in the shell used
  for this repo; that isn't being done so far.

Decision (maintainer, 2026-10-13): **no history rewrite.** Nothing personal needs one, and
the history is kept as it is, trailers and time zones included. The session, machine and
billing details in the docs were trimmed in place instead (`docs/next-steps.md` item 8).

- [ ] Re-run the scan right before flipping visibility, since commits keep landing:
  ```sh
  git log --all --format='%an <%ae> | %cn <%ce>' | sort -u
  git log -p --all | grep -oiE '[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}' | sort | uniq -c | sort -rn
  git log -p --all | grep -oiE '(/Users/|/home/|C:\\+Users\\+)[A-Za-z0-9._-]+' | sort | uniq -c
  ```
- [ ] **CI screenshot branches** (`ci/e2e-api22`, `ci/e2e-api34`, `ci/e2e-fire7`,
  `ci/e2e-firehd10`) hold emulator screenshots, `logcat.txt`, `host.log` and `gradle.txt`.
  They're clean, but they're noise in a public repo. E2E no longer pushes them: its
  screenshots and logs are workflow artifacts on each run's summary page. Delete the
  branches before launch **(maintainer)**:
  `git push origin --delete ci/e2e-api22 ci/e2e-api34 ci/e2e-fire7 ci/e2e-firehd10`.
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
  are in `.gitleaksignore`, so a re-run doesn't report them again.
- [ ] Re-run before launch: `gitleaks git --log-opts="--all" .` (about 146 commits have
  landed since the 2026-10-11 run). gitleaks isn't installed locally: install it or run
  it in a workflow **(maintainer)**. A pattern sweep on 2026-10-13 (`ghp_`,
  `github_pat_`, `AKIA`, `-----BEGIN`, `xoxb-`, `AIza`, `sk-`, `glpat-` over every added
  line) found nothing; the `password=` and `api_key=` hits are test values, placeholders or
  environment-variable reads.
- [ ] Check that no signing material is in the tree (`*.jks`, `*.keystore`, `*.p12`, `*.p8`,
  `*.pfx`, `.env` and `keystore.properties` are all in `.gitignore` now, and none is
  tracked) and that the new Android release key is backed up twice, offline
  (`docs/next-steps.md` item 11).
- [ ] Review repository and organisation secrets and variables: only what `docs/releasing.md`
  lists.
  Secrets are not exposed to workflows triggered from forks, but check that no workflow
  uses `pull_request_target` with a checkout of the PR head (none does today).

## 3. Files in the repository

- [x] `LICENSE` (Apache-2.0), `plugins/LICENSE` (MIT), `THIRD_PARTY_NOTICES.md`
  (`scripts/third-party-notices.py`), `host/deny.toml`.
- [x] `README.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`,
  `.github/ISSUE_TEMPLATE/`, `.github/PULL_REQUEST_TEMPLATE.md`, `.github/CODEOWNERS`,
  `.github/FUNDING.yml` (commented out).
- [x] Final screenshots in `docs/assets/`, referenced from the README.
- [x] `docs/SECURITY.md` "Reporting" section points at `SECURITY.md` and
  dev@perritodeck.dev.
- [ ] Make sure dev@perritodeck.dev actually receives mail (it's the CoC, security and
  store contact).
- [x] Session-oriented docs (`docs/HANDOFF.md`, `CLAUDE.md`, `docs/next-steps.md` and
  others) trimmed in place into neutral contributor notes (decided 2026-10-13).
- [x] The copyright holder is "PerritoDeck contributors" everywhere, `plugins/LICENSE`
  included (decided 2026-10-13).
- [x] Add `publish = false` to the `perritodeck-*` crates.
- [x] Set `wildcards = "deny"` in `host/deny.toml` (cargo-deny 0.20.2 passes licences, bans
  and sources on both workspaces, 2026-10-13).
- [ ] Decide on RUSTSEC-2026-0192 (`ttf-parser`, behind `ab_glyph`, is unmaintained): replace
  it or accept it in `deny.toml` with a reason. Then add `cargo deny check licenses bans sources` and
  `python3 scripts/third-party-notices.py --check` to a push workflow. Today only the
  Release workflow runs the notices check (its licences job), so a release with stale
  notices fails at tag time; nothing runs `cargo deny`.
- [x] The plugin catalogue's zips and the desktop app's bundled plugins carry
  `plugins/LICENSE`.
- [x] The release, the command-line archives and the desktop installers carry
  `third-party-licenses.txt` (`release.yml` puts it in the app's resource folder with
  `tauri build --config`).
- [ ] **Release blocker:** the third-party notices in the APK, or a licences screen in
  Perrito. The APK carries no licence texts today. Generate a text asset at build time
  (not Play Services' oss-licenses, which F-Droid forbids).
- [ ] Optional: a link to the licences next to Version in the desktop app's Settings (the
  installers already carry the file).

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
  maintainer to bypass while the team is one person), block force pushes and deletion,
  require linear history. **Require no status checks yet.** Every workflow is
  path-filtered, so a required check whose workflow didn't run stays pending forever and
  blocks the PR. The fix is a single always-running gate job to require instead of the
  individual checks, but **that gate doesn't exist yet** (`docs/next-steps.md` item 16):
  one workflow that uses a paths filter (e.g. `dorny/paths-filter`) to decide which jobs
  run, plus a final `if: always()` job that fails if any job it needed failed. A gate
  can't `needs:` jobs in the other workflows, because `needs` works only inside one
  workflow. Once it exists, require only that final job.
  `bootstrap-gradle-wrapper.yml` pushes straight to `main` and will need to open a PR
  instead.
- [ ] **Tag ruleset** for `v*`: only the maintainer can create, nobody can update or
  delete.
- [ ] Recommended: a `release` environment for the three Android signing secrets, limited to
  `v*` tags with a required reviewer, and the `environment: release` line in `release.yml`
  turned on at the same time (`docs/releasing.md`, "Once the repository is public").
- [ ] Actions → General: "Require approval for first-time contributors" for fork PRs, and
  check that "Workflow permissions" is read-only (not yet confirmed). Every workflow file
  already asks for read-only `permissions`, except `bootstrap-gradle-wrapper.yml` and the
  Release workflow's publish job.
- [ ] Optional: [DCO app](https://github.com/apps/dco) to enforce sign-offs (required
  from 2026-10-13; the app checks the commits of pull requests, not older history).
- [ ] Public repos get free standard GitHub-hosted runners, macOS included. Revisit the
  CI cost rules in `CLAUDE.md` (weekly-only Linux/macOS jobs, two emulators per push)
  and consider running more on every push.

## 6. Website and plugin catalogue (perritodeck.dev)

The steps are in [`website/README.md`](../website/README.md): "Deploy" for the setup,
"Launch day" for the day the release goes out. `docs/next-steps.md` item 12 has the
decisions to make first.

- [ ] Deploy `website/` as `website/README.md` "Deploy" describes: Cloudflare Pages with
  Git integration, whose build command `python3 plugins/build_catalog.py website/plugins`
  publishes the catalogue on every deploy (`website/plugins/` is git-ignored). It works
  while the repo is private. If the site is ever served without that build step (GitHub
  Pages, say), unzip the release's `perritodeck-X.Y.Z-plugin-catalogue.zip` into
  `website/plugins/` before each deploy.
- [ ] Custom domain `perritodeck.dev`, with `www` and the `pages.dev` address redirecting to
  it, HTTPS enforced, HSTS ("The domain" in `website/README.md`). Set up mail for `dev@`
  before the nameservers move.
- [ ] Check `https://perritodeck.dev/plugins/index.json` loads and that the desktop app's
  Plugins → "Look online" installs a plugin from it.
- [ ] **Launch day**, after step 7 publishes the release: work through "Launch day" in
  `website/README.md`. It removes the temporary `noindex` header and the "Not released
  yet" notice and other pre-release lines, checks the status lines, and lists the checks
  after the deploy. The home page's Download links already point at `download.html`.
- [x] Self-host the two web fonts instead of loading them from Google Fonts, in keeping
  with "nothing leaves your desk": done (`website/fonts/`); no page, the plugin catalogue
  included, makes a third-party request.

## 7. First release **(maintainer)**

Follow [docs/releasing.md](releasing.md). In short:

- [ ] Android signing: the maintainer's new release key, three secrets
  (`PERRITO_KEYSTORE_BASE64`, `PERRITO_KEYSTORE_PASSWORD`, `PERRITO_KEY_PASSWORD`) and two
  variables (`PERRITO_KEY_ALIAS`, and `PERRITO_CERT_SHA256`, recommended for the first
  release), and the certificate's fingerprint in `website/download.html`
  (`#android-certificate`). A tag build fails without the key. `docs/next-steps.md`
  item 11 has the steps. Apple and Windows signing are optional.
- [ ] `python3 scripts/release-version.py X.Y.Z`, changelog in
  `fastlane/metadata/android/en-US/changelogs/<versionCode>.txt`.
- [x] The full licence texts for the release: `release.yml` runs
  `scripts/third-party-notices.py --check --full` and puts `third-party-licenses.txt` in
  the release, the command-line archives and the desktop installers.
- [ ] **Release blocker:** the third-party notices in the APK, or a licences screen in
  Perrito (§3). Of the apps in the release, only the APK lacks them.
- [ ] Decide whether the first public release is `v0.1.0` on current `main` or `v0.2.0`.
  There are no tags in this repository or on GitHub. A stale local `v0.1.0` tag (on
  `9f47509`, before most features) is left in one other clone and was never pushed;
  delete it there. Every version field is still 0.1.0 / versionCode 1. Details in
  `docs/next-steps.md` item 13.
- [ ] Windows installer, from the NSIS test on 2026-10-13 (`docs/next-steps.md` item 5):
  - Unsigned, a downloaded installer will get SmartScreen's "Windows protected your PC" with
    "Unknown publisher" (More info → Run anyway). This wasn't seen in testing, because a file
    built locally has no download mark. Sign it, or say so on the download page.
  - An upgrade runs the old version's uninstall wizard, which has a "Delete the application
    data" checkbox (unticked). Consider a custom NSIS template, or making that page skip
    data deletion during upgrades.
  - The running-app message reads "PerritoDeck is running! Click OK to kill it"; friendlier
    wording would need custom NSIS language strings.
  - Uninstall leaves `HKCU\Software\PerritoDeck` (install path and language) behind.
  - The login entry's path isn't quoted (upstream `auto-launch`); it still works.
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
