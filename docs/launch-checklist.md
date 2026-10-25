# Launch checklist: making the repository public

Everything to do, in order, to take `perritodeck/perritodeck` from private to public and
ship the first release. Steps marked **(maintainer)** change repository settings, secrets,
tags or anything visible to the world; only the maintainer does those.

Status as of 2026-10-24: the repository is public. Steps 1–2 were re-checked right before
the flip (results below), and the ticked boxes are done. Everything else is open;
`docs/next-steps.md` item 8 has the current state, and `docs/plan/phase-8.md` the
decisions (D1, D2, D4, D5, D9, D10 and D11 were made on 2026-10-15, and D3, D6, D7, D8
and D12 on 2026-10-17).

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

- [x] Re-run the scan right before flipping visibility, since commits keep landing. Done
  2026-10-24: every commit on every ref is by dmang-dev or a bot, and no ref's history has
  personal data (the upstream enigo author list is allowed). GitHub still serves 8 deleted
  commits by SHA (old E2E artifacts and Dependabot heads, nothing personal): accepted. The
  home-path search below didn't find Windows paths until `1b69e47` fixed it:
  ```sh
  git log --all --format='%an <%ae> | %cn <%ce>' | sort -u
  git log -p --all | grep -oiE '[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}' | sort | uniq -c | sort -rn
  git log -p --all | grep -oiE '(/Users/|/home/|C:[\]+Users[\]+|C:/Users/|/c/Users/|/mnt/c/Users/)[A-Za-z0-9._-]+' | sort | uniq -c
  ```
- [x] **CI screenshot branches** (`ci/e2e-api22`, `ci/e2e-api34`, `ci/e2e-fire7`,
  `ci/e2e-firehd10`), which held emulator screenshots and logs, were deleted on GitHub on
  2026-10-13. E2E no longer pushes them: its screenshots and logs are workflow artifacts
  on each run's summary page.
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
- [x] Re-run on 2026-10-13 with gitleaks 8.30.1 over every ref (236 commits): no leaks.
  Three new false positives, all in `.gitleaksignore` with a note: the made-up
  `deviceSecret` in `hello-network-failed.json` and the iOS test that checks a fixture's,
  and a hotkey example in `host/README.md`.
- [x] Re-run once more right before flipping visibility. Done 2026-10-24: clean over every
  ref (the 8 fixture fingerprints are in `.gitleaksignore`).
  `gitleaks git --log-opts="--all" .` (gitleaks is installed with winget,
  `Gitleaks.Gitleaks`). A pattern sweep on 2026-10-13 (`ghp_`,
  `github_pat_`, `AKIA`, `-----BEGIN`, `xoxb-`, `AIza`, `sk-`, `glpat-` over every added
  line) found nothing; the `password=` and `api_key=` hits are test values, placeholders or
  environment-variable reads.
- [ ] Check that no signing material is in the tree (`*.jks`, `*.keystore`, `*.p12`, `*.p8`,
  `*.pfx`, `.env` and `keystore.properties` are all in `.gitignore` now, and none is
  tracked) and that the new Android release key is backed up twice, offline
  (`docs/next-steps.md` item 11). The first half was checked on 2026-10-24: no signing
  material or `.env` was ever committed. The release key doesn't exist yet.
- [ ] Review repository and organisation secrets and variables: only what `docs/releasing.md`
  lists, plus `SITE_PUBLISH_TOKEN` in the `site` environment once it exists (§4, D12).
  Secrets are not exposed to workflows triggered from forks, but check that no workflow
  uses `pull_request_target` with a checkout of the PR head (none does today).
  Organisation-level secrets weren't checked on 2026-10-24: the token lacked `admin:org`.

## 3. Files in the repository

- [x] `LICENSE` (Apache-2.0), `plugins/LICENSE` (MIT), `THIRD_PARTY_NOTICES.md`
  (`scripts/third-party-notices.py`), `host/deny.toml`.
- [x] `README.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`,
  `.github/ISSUE_TEMPLATE/`, `.github/PULL_REQUEST_TEMPLATE.md`, `.github/CODEOWNERS`,
  `.github/FUNDING.yml` (commented out).
- [x] Final screenshots in `docs/assets/`, referenced from the README.
- [x] `docs/SECURITY.md` "Reporting" section points at `SECURITY.md` and
  dev@perritodeck.dev.
- [x] dev@perritodeck.dev receives mail (it's the CoC, security and store contact):
  ImprovMX forwards it to the maintainer's mailbox (D2 in `docs/plan/phase-8.md`, decided
  2026-10-15). Cloudflare DNS has the `MX` records, SPF, one DMARC record (`p=none`) and
  ImprovMX's two DKIM records. Replies as dev@ go through Gmail's "Send mail as", which
  fails DMARC alignment, so DMARC stays at `p=none`. Google removes "Send mail as" for
  custom domains in January 2027: before then, replies move to ImprovMX Premium's SMTP or
  a free SMTP relay, and DMARC can go to `p=reject` (`docs/next-steps.md` item 8).
- [x] Session-oriented docs (`docs/HANDOFF.md`, `CLAUDE.md`, `docs/next-steps.md` and
  others) trimmed in place into neutral contributor notes (decided 2026-10-13).
- [x] The copyright holder is "PerritoDeck contributors" everywhere, `plugins/LICENSE`
  included (decided 2026-10-13).
- [x] Add `publish = false` to the `perritodeck-*` crates.
- [x] Set `wildcards = "deny"` in `host/deny.toml` (cargo-deny 0.20.2 passes licences, bans
  and sources on both workspaces, 2026-10-13).
- [x] RUSTSEC-2026-0192 (`ttf-parser`, behind `ab_glyph`, is unmaintained; not a
  vulnerability): accepted in `host/deny.toml` with a reason, and the library is replaced
  in a later release (D6, 2026-10-17).
- [x] The advisories published since 2026-10-13 (2026-10-24). Every one with a fix is
  updated: `rustls` 0.23.46, `zip` 9.0.5, `webpki-roots` 1.0.11, `rustls-webpki` 0.103.16
  and `tokio` 1.53.3. The unmaintained `zvariant`, `zbus_names`, `zvariant_derive` and
  `proc-macro-error`, all Linux only with no upgrade yet, are accepted in `host/deny.toml`
  (D13). `cargo deny check` passes on both workspaces. `docs/next-steps.md` item 8 has the
  IDs.
- [x] `cargo deny check licenses bans sources` (both workspaces) and `python3
  scripts/third-party-notices.py --check` run in CI, with `cargo deny check advisories` as a
  job of its own (`.github/workflows/licences.yml`, 2026-10-24; not run on GitHub yet): on
  pushes and pull requests that change a Cargo manifest or lock file, `deny.toml`, the app's
  `package-lock.json` or the notices, and weekly. Advisories pass on both workspaces since the
  update above (2026-10-24).
- [x] The plugin catalogue's zips and the desktop app's bundled plugins carry
  `plugins/LICENSE`.
- [x] The release, the command-line archives and the desktop installers carry
  `third-party-licenses.txt` (`release.yml` puts it in the app's resource folder with
  `tauri build --config`).
- [x] The APK carries `assets/third-party-licenses.txt`, generated at build time from the
  release runtime classpath (`generateReleaseThirdPartyLicenses` in
  `android/app/build.gradle.kts`, not Play Services' oss-licenses, which F-Droid forbids),
  and Perrito shows it under Open-source licences on the home screen (2026-10-15).
- [x] A link to the licences next to Version in the desktop app's Settings: it opens the
  installers' `third-party-licenses.txt` (2026-10-15).

## 4. Flip visibility **(maintainer)**

- [x] Account security (2026-10-15): the app trusts perritodeck.dev for plugin code and
  github.com/perritodeck for releases. 2FA is on for the maintainer's GitHub, Namecheap,
  Cloudflare and ImprovMX accounts; the perritodeck org requires 2FA
  (`two_factor_requirement_enabled: true`, and no member without 2FA, checked through the
  API); the domain auto-renews at Namecheap.
- [x] The two parts of account security that check didn't cover, confirmed on 2026-10-17:
  passkey 2FA on the Azure account (D1 keeps it for Windows signing), and Namecheap's
  Domain Lock on perritodeck.dev (the `.dev` registry's RDAP shows "client transfer
  prohibited", checked 2026-10-17).
- [x] No stale tag can be pushed (2026-10-15): no other clone exists, and there are no
  tags in this repository or on GitHub.
- [x] Right before the flip, delete all GitHub Actions runs and their artifacts (E2E logs,
  logcat, the 2026-10-11 desktop bundles): Claude does it, with the maintainer's OK at
  that moment (D9 in `docs/plan/phase-8.md`). Done 2026-10-24: all 257 runs and their
  artifacts.
- [x] Review Dependabot's open pull request #1. There are no issues. Reviewed 2026-10-24:
  it stays open, and is merged once it has a green CI run of its own.
- [x] Settings → General → Danger Zone → **Change visibility → Public**. Done 2026-10-24.
- [x] CI runs again from here (it was blocked by billing from 2026-10-11 until the flip, by
  the maintainer's choice). Run every workflow once by hand and get them all green before
  §7's tag. Done 2026-10-24: the first run, CI by hand on `main`, failed host and plugin
  tests on Windows and macOS, three emulators and iOS, and all of it was fixed. Then CI run
  40157961726 on `01673dd` was green in every job (host and plugins on Windows, Linux and
  macOS, the desktop app, all four emulators, Android, protocol, licences and advisories,
  CI gate), and so were iOS run 40157963719 (the first iOS build ever) and the push's own CI
  run.
- [ ] Desktop app with platforms = all (Actions → Desktop app → Run workflow): the Linux and
  macOS bundles and app crate, which CI's Run workflow leaves out. The weekly run (Mondays)
  covers them too.
- [x] The website's publishing (D12; "At the flip" in the cutover runbook in
  `docs/website.md`): Settings → Environments → `site`, with deployment branches limited to
  `main`, and `SITE_PUBLISH_TOKEN` in it (a fine-grained token, the perritodeck
  organization as resource owner, `perritodeck.github.io` only, Contents read and write;
  or a write deploy key). Run **Site publish** by hand and expect "site: no change", and
  **Site check**, which passes. So far (2026-10-24) Site publish built and checked the site
  on GitHub, and with no token yet it pushed nothing. Done 2026-10-25: the token is set, and
  a manual Site publish pushed `site: publish from perritodeck@ff0ba41` (the download page's
  certificate fingerprint) as `perritodeck-site-bot`; pushes to `main` now publish by
  themselves. (Deploy keys are switched off for the organization, hence the token.)
  - `SITE_PUBLISH_TOKEN` ("perritodeck site publish") **expires on 2027-10-26**. Renew it
    before then (regenerate, then `gh secret set SITE_PUBLISH_TOKEN --env site`), or the
    site stops publishing.
- [ ] Settings → General → Features: Issues on; Discussions on (optional, for Q&A);
  Wiki off; Projects as preferred.
- [x] Description, website `https://perritodeck.dev`, topics (`stream-deck`, `macro-pad`,
  `android`, `tauri`, `rust`, `obs`, `home-assistant`, `fdroid`), social preview image. Done 2026-10-24 through the API, except the
  social preview image: upload `website/img/og.png` in Settings → General → Social preview.

## 5. Protect the repository **(maintainer)**

- [x] **Private vulnerability reporting:** Settings → Code security → Private
  vulnerability reporting → Enable (`SECURITY.md` links to it). Done 2026-10-24.
- [x] Dependabot alerts on. Done 2026-10-24.
- [x] Dependabot security updates on; secret scanning and push protection on (free for
  public repos). Done 2026-10-24 through the API.
- [x] **Branch protection / ruleset for `main`:** require a pull request (allow the
  maintainer to bypass while the team is one person), block force pushes and deletion,
  require linear history. **Require one status check, `CI gate`** (source: GitHub
  Actions), the last job of `.github/workflows/ci.yml` (`docs/next-steps.md` item 16), and
  none of the individual checks: those run only when a change touches what they read, so a
  required one that didn't run would stay pending and block the PR. CI gate reports on
  every pull request, docs-only ones included, and fails if any check the change needed
  failed or was cancelled. CI gate has run on GitHub since 2026-10-24, so the ruleset
  offers it by name and it can be added now. Don't turn on a merge queue: `ci.yml` has no
  `merge_group` trigger, so CI gate would never report for a queued change and every merge would stall.
  `bootstrap-gradle-wrapper.yml` works under the ruleset: it pushes a
  `gradle-wrapper-<version>` branch, not `main`, and a person opens the pull request from
  the compare link in its run summary.
  Done 2026-10-24 through the API: ruleset `main` (default branch, active) with deletion
  and force-push blocks, linear history, a pull request (no approval required) and the
  `CI gate` check (GitHub Actions); Repository admin may bypass.
- [x] **Tag ruleset** for `v*`: only the maintainer can create, nobody can update or
  delete. Done 2026-10-24: ruleset `release tags` (`refs/tags/v*`)
  restricts creation, update and deletion; Repository admin may bypass.
- [ ] Recommended: a `release` environment for the three Android signing secrets, limited to
  `v*` tags with a required reviewer, and the `environment: release` line in `release.yml`
  turned on at the same time (`docs/releasing.md`, "Once the repository is public").
- [x] Actions → General: "Require approval for first-time contributors" for fork PRs. Done
  2026-10-24.
- [x] Optional: Actions → General, require actions to be pinned to a full commit SHA. Every
  `uses:` in the workflows already is. Done 2026-10-24.
- [x] Actions → General → "Workflow permissions" is already read-only, and "Allow GitHub
  Actions to create and approve pull requests" is off (checked through the API on
  2026-10-13 and 2026-10-15; the token is still read-only after the flip, 2026-10-24). Keep
  it off: `bootstrap-gradle-wrapper.yml` doesn't open
  pull requests itself (DECISIONS, 2026-10-15). Every workflow file also asks for read-only
  `permissions` at the top; only the jobs of `bootstrap-gradle-wrapper.yml` (`contents:
  write`, to push its branch) and the Release workflow's publish job ask for write.
- [ ] Optional: [DCO app](https://github.com/apps/dco) to enforce sign-offs (required
  from 2026-10-13; the app checks the commits of pull requests, not older history).
- [ ] Public repos get free standard GitHub-hosted runners, macOS included. Revisit the
  CI cost rules (weekly-only Linux/macOS jobs, two emulators per push) and consider running
  more on every push. `CLAUDE.md`'s CI section describes the public repo since 2026-10-24,
  but the matrices haven't changed.

## 6. Website and plugin catalogue (perritodeck.dev)

The steps are in [`docs/website.md`](website.md): "Deploy" for the setup and publishing,
"Cutover runbook" for the move to GitHub Pages (D12, decided 2026-10-17), "Launch day" for
the day the release goes out. `docs/next-steps.md` item 12 has what's left.

- [x] The site is live: since 2026-10-15 on `perritodeck.dev`, and since 2026-10-24 served
  by GitHub Pages from `perritodeck/perritodeck.github.io`, published by hand from WSL
  (the first publish, `34861f5`, is `perritodeck@328162b`).
- [x] **The cutover to GitHub Pages** (D12), done 2026-10-24, while noindex is on and no
  release is out: the cutover runbook in `docs/website.md`. GitHub's four `A` and four
  `AAAA` records and the `www` `CNAME` are in DNS (DNS only), the Let's Encrypt certificate
  for the apex and `www` was approved within about a minute, and Enforce HTTPS is on. The
  site repository's Pages build ran despite the billing block. The Cloudflare Worker was
  deleted at the swap, so there was no rollback; the `www` Redirect Rule and the Cloudflare
  GitHub app went too, and Web Analytics had no entry for the site. `website/_headers` is
  deleted, and the CSP test reads the `<meta>` tag alone. The zone settings only the
  dashboard shows no longer matter.
- [ ] Until the flip, publish by hand from WSL after each change to `website/` or
  `plugins/` on `main` ("Publishing by hand" in `docs/website.md`), with the maintainer's
  OK for each push to the site repository.
- [ ] Until the flip, check by hand now and then, and after any DNS change, that the site
  hasn't drifted to the state that took evi-ai.dev down (there's no weekly run before the
  flip):
  `python3 scripts/publish_site.py --check-live` (only GitHub's addresses for the apex and
  `www`, `Server: GitHub.com` on `/`, `/plugins/index.json` and `www`'s 301, a certificate
  with more than 21 days left)
  and `gh api repos/perritodeck/perritodeck.github.io/pages --jq .https_certificate.state`
  (`approved`).
- [x] Custom domain `perritodeck.dev`, with `www` answering 301 to it, path and query kept,
  HTTPS enforced (`http://` gets a 301) and DNSSEC in force (DS 2371/13/2 is at the `.dev`
  registry, and resolvers validate), first checked 2026-10-15. Checked again on GitHub
  Pages on 2026-10-24: `www` is
  a `CNAME` to `perritodeck.github.io`, GitHub does the redirects (`www` to the apex
  keeping the path and query, `http://` to `https://`, `perritodeck.github.io` to the
  domain), and DNSSEC and mail for `dev@` (§3, D2) didn't change.
- [ ] Check `https://perritodeck.dev/plugins/index.json` loads and that the desktop app's
  Plugins → "Look online" installs a plugin from it. The first half is done on GitHub
  Pages (24 plugins, `application/json`, the zips' SHA-256s match, 2026-10-24); the
  install in the real app is left.
- [ ] **Launch day**, after step 7 publishes the release: work through "Launch day" in
  `docs/website.md`. It removes the temporary `noindex` tags and the "Not released yet"
  notice and other pre-release lines, checks the status lines, and lists the checks after
  the publish. The home page's Download links already point at `download.html`. Then
  give the site repository its website link, `https://perritodeck.dev` (maintainer).
  The edits are done (2026-10-25); left are the checks after the publish and the site
  repository's website link.
- [x] Self-host the two web fonts instead of loading them from Google Fonts, in keeping
  with "nothing leaves your desk": done (`website/fonts/`); no page, the plugin catalogue
  included, makes a third-party request.

## 7. First release **(maintainer)**

Follow [docs/releasing.md](releasing.md). In short:

**Scope (D5, decided 2026-10-15):** the first release is v0.1.0, Windows and Android only.
It publishes the Windows NSIS installer and the Android APK. The MSI, macOS, the Linux
packages and the command-line archives are not in it. Linux and macOS still build and
test in CI, Linux keeps building from source, and they join a later release once tested
on real hardware.

- [x] Android signing: the maintainer's new release key, three secrets
  (`PERRITO_KEYSTORE_BASE64`, `PERRITO_KEYSTORE_PASSWORD`, `PERRITO_KEY_PASSWORD`) and two
  variables (`PERRITO_KEY_ALIAS`, and `PERRITO_CERT_SHA256`, recommended for the first
  release), and the certificate's fingerprint in `website/download.html`
  (`#android-certificate`). A tag build fails without the key. `docs/next-steps.md`
  item 11 has the steps. Done 2026-10-25: the key was generated and backed up twice, the three
  secrets are in the `release` environment, both variables are set, the fingerprint
  `848c5d3f…f24e5` is on the download page, and v0.1.0's APK verified with it.
- [x] Windows signing (D1): **v0.1.0 ships unsigned** (decided 2026-10-25: Azure Artifact
  Signing's validation is taking too long). The release notes and the download page tell
  people how to get past SmartScreen. Signing waits for a later release. Before wiring it in, check which name the certificate shows
  as publisher (an individual validation shows a legal name). Apple signing isn't needed
  for v0.1.0.
- [x] `release.yml` publishes only v0.1.0's scope (2026-10-15, `docs/plan/phase-8.md`
  M3-12): its `SHIP` list is `windows-nsis`, so a tag builds the NSIS installer and no
  MSI, Linux, macOS or command-line files, and the draft gets the installer, the APK, the
  plugin catalogue zip, `third-party-licenses.txt` and `SHA256SUMS`, with notes for those
  alone ("What ships" in `docs/releasing.md`). Not yet run on GitHub.
- [ ] Versions: every version field already says 0.1.0 / versionCode 1
  (`python3 scripts/release-version.py --check 0.1.0`). Rewrite
  `fastlane/metadata/android/en-US/changelogs/1.txt`, which describes the very first
  slice.
- [x] The full licence texts for the release: `release.yml` runs
  `scripts/third-party-notices.py --check --full` and puts `third-party-licenses.txt` in
  the release, the command-line archives and the desktop installers.
- [x] The third-party notices in the APK and a licences screen in Perrito (§3, 2026-10-15),
  so every app in the release carries the texts.
- [x] The first public release is `v0.1.0` on current `main` (D5, 2026-10-15). There are
  no tags in this repository or on GitHub, and no other clone exists (2026-10-15).
  Details in `docs/next-steps.md` item 13.
- [ ] Windows installer, from the NSIS test on 2026-10-13 (`docs/next-steps.md` item 5):
  - Unsigned, a downloaded installer will get SmartScreen's "Windows protected your PC" with
    "Unknown publisher" (More info → Run anyway). This wasn't seen in testing, because a file
    built locally has no download mark. Sign it, or say so on the download page.
  - An upgrade runs the old version's uninstall wizard, which has a "Delete the application
    data" checkbox (unticked). It doesn't delete PerritoDeck's data: ticked, it removes only
    Tauri's own folders (`%APPDATA%\dev.perritodeck.PerritoDeck`, and WebView2's data in
    `%LOCALAPPDATA%\dev.perritodeck.PerritoDeck`) and `HKCU\Software\PerritoDeck`. The
    pairings, profiles and settings in `%APPDATA%\perritodeck\PerritoDeck` stay either way,
    on an upgrade and on a real uninstall. Consider a custom NSIS template that removes that
    folder when ticked, or hides the box; until then, the release notes say to delete the
    folder by hand.
  - The running-app message reads "PerritoDeck is running! Click OK to kill it"; friendlier
    wording would need custom NSIS language strings.
  - Uninstall leaves `HKCU\Software\PerritoDeck` (install path and language) behind, unless
    "Delete the application data" is ticked.
  - The login entry's path isn't quoted (upstream `auto-launch`); it still works.
- [x] Tag, let the Release workflow build the draft, check it has the NSIS installer and
  the APK and none of the MSI, macOS, Linux or command-line files, test the installer on
  Windows and the APK on the Fire 7, publish. Done 2026-10-25:
  [v0.1.0](https://github.com/perritodeck/perritodeck/releases/tag/v0.1.0), a GitHub
  pre-release, with the unsigned NSIS installer, the signed APK, the plugin catalogue zip,
  `third-party-licenses.txt` and `SHA256SUMS`.

## 8. Stores and package managers

See [docs/distribution.md](distribution.md) for each channel's requirements.

- [ ] **IzzyOnDroid:** request inclusion. The request is ready in
  [docs/fdroid/IZZY-REQUEST.md](fdroid/IZZY-REQUEST.md) (on Codeberg now); read its first
  section first: IzzyOnDroid's AI policy likely rules the app out.
- [ ] **F-Droid:** merge request to `fdroiddata`. Ready in
  [docs/fdroid/SUBMIT.md](fdroid/SUBMIT.md): the recipe builds `v0.1.0`; decide first
  whether F-Droid signs it or D11 is settled for our signature.
- **Google Play: not for now** (D8 in `docs/plan/phase-8.md`, 2026-10-17). v0.1.0's
  Android app ships on GitHub Releases, then IzzyOnDroid and F-Droid. The closed test,
  the data safety form and the `play-bundle` upload are off the launch path; the notes for
  later are under "Google Play" in `docs/distribution.md`.
- [ ] **Desktop:** winget is the Windows channel: the v0.1.0 manifests are ready in
  `packaging/winget/` ([SUBMIT.md](../packaging/winget/SUBMIT.md)). Flathub and AUR (from `packaging/`) wait for the
  release that adds Linux, and a Homebrew cask for the one that adds macOS, best once
  notarised (D5).
- [ ] Amazon Appstore (optional, for Fire tablet owners).

## 9. Announcement

- [ ] Short demo video or GIF (tablet + editor), screenshots from `docs/assets/`.
- [ ] Blog-style post on the website or a pinned GitHub Discussion: what it is, what works,
  what doesn't yet, how to help.
- [ ] Post to r/streamdeck, r/selfhosted, r/homeassistant, r/obs, r/fdroid, Hacker News
  (Show HN), the Fediverse; tell the Home Assistant and OBS forums.
- [ ] Label a handful of `good first issue`s before announcing.
- [ ] Watch issues closely for the first week; be ready with a patch release.
