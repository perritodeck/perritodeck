# Phase 8: going public, the website, the first release

The plan for everything still open, in order. Each item was checked against the tree on
2026-10-13, and again on 2026-10-15, when M1-1 to M1-4, M2-2, M3-1 to M3-5 and M3-12
landed, the maintainer decided D1, D2, D4, D5, D9 and D10, D11 was settled by
measurement, M1-7 and M1-8 were done, and so was most of M1-5. On 2026-10-17 the
maintainer decided D3, D6, D7, D8, D12 and D13 and confirmed the last two parts of M1-7. On
2026-10-24 the repository went public (M1-10 to M1-12), and CI ran again and went green
(M1-14, M3-7). The details behind each one are in [next-steps](../next-steps.md) (items
8–18), the steps in order in the [launch checklist](../launch-checklist.md), and the
release mechanics in [releasing](../releasing.md).

**Milestones**
- **M1, public repo:** what must be true before the repository goes public. That includes
  the website being up, because the README, the issue forms and the security contact all
  point at it.
- **M2, website live:** the site indexed and complete. Never before M1.
- **M3, first release:** installers and the APK published, and the first store submissions.
- **M4, after the release.**

**Who:** **C** is Claude (code and docs, no decision needed), **M** is the maintainer
(accounts, money, hardware, decisions, GitHub settings), **C+M** is both.

## Right now

- **The repository is public** since 2026-10-24 (M1-12). Right before the flip all 257 old
  Actions runs and their artifacts were deleted (D9, M1-10), and the final scans passed
  (M1-11): every commit is by dmang-dev or a bot, no ref's history has personal data,
  gitleaks is clean over every ref, and no signing material or `.env` was ever committed.
  GitHub still serves 8 deleted commits by SHA (old E2E artifacts and Dependabot heads,
  nothing personal), which is accepted. Organisation-level secrets weren't checked (the
  token lacked `admin:org`).
- **CI runs again** (M1-14, M3-7). The first run, CI by hand on `main`, failed host and
  plugin tests on Windows and macOS, three emulators and iOS; all of it is fixed
  (`git log 13ae37a..01673dd`). Then CI run 40157961726 on `01673dd` was green in every job,
  Linux and macOS and all four emulators included, iOS run 40157963719 was green (the first
  iOS build ever), and so was the push's own CI run. Site publish built and checked the
  site; with no token yet it pushed nothing. Dependabot PR #1 (the actions' major bumps)
  stays open until it has a green CI run of its own (M3-8).
- **Repository settings are partly done** (M1-13): private vulnerability reporting,
  Dependabot alerts, approval for first-time contributors' workflows, and a read-only
  workflow token. The rest is the maintainer's (launch checklist §4 and §5).
- **The site is on GitHub Pages** since 2026-10-24 (D12, M1-5), on
  `https://perritodeck.dev` with the temporary noindex: the public repository
  `perritodeck/perritodeck.github.io` serves what `scripts/publish_site.py` builds from
  `website/` and `plugins/`, and Cloudflare does DNS only. The first publish, by hand from
  WSL, was `perritodeck@328162b`, and its Pages build ran despite the billing block. The
  certificate was approved within about a minute and Enforce HTTPS is on. The Cloudflare
  Worker was deleted at the swap, so there was no rollback, and every check after the swap
  passed ([cutover runbook](../website.md#cutover-runbook)). `website/_headers` is gone and
  the CSP test reads the `<meta>` tag alone. Look online passed in the real app on
  2026-10-24, which finishes M1-6. Until `SITE_PUBLISH_TOKEN` exists the site is published
  by hand from WSL. DNSSEC (DS 2371/13/2) and mail for dev@ (D2) didn't change.
- **The OBS real-world pass is done.** It ran on 2026-10-13 against OBS 31.0.3 (93 of 110
  cases), and all 22 findings were fixed (OBS plugin 1.0.3 and the host) and re-checked
  against the same OBS on 2026-10-15
  ([testing/obs-results-2026-10-13.md](../testing/obs-results-2026-10-13.md)). The OBS
  plugin is no longer Experimental: 13 plugins are listed in
  `plugins/catalogue_status.json`. The cases that couldn't run yet are in the results' §7
  ([next-steps](../next-steps.md) item 9).
- **The Twitch real-world pass is skipped for v0.1.0** (decided 2026-10-24): the Twitch plugin
  ships marked Experimental, and the pass waits for a later release.
- **The release APK on the Fire 7** (M3-10, 2026-10-24): the cable-takeover host fix holds
  (twice, keys, the slider and the clock kept working on the cable after Wi-Fi went off), and
  pairing and the deck work over Wi-Fi. The "On this network" fix is still unchecked: the
  tablet's Android NSD never reported the host, though its mDNS packets reach the tablet.
  A reboot of the tablet and a rerun are next.
- **The real-app pass passed** (2026-10-24): the import dialog, Settings → Integrations and
  Look online, with one bug fixed (`13ae37a`). Its small findings are in
  [next-steps](../next-steps.md) item 14.
- **Decided on 2026-10-15:** D1, D2, D4, D5, D9 and D10, and D11 by measurement (the
  table below). The stale tag (M1-8) is done.
- **Decided on 2026-10-17:** D3, D6, D7, D8, D12 and D13 (the table below). There's no Google
  Play for now (D8): v0.1.0's Android app ships on GitHub Releases, then IzzyOnDroid and
  F-Droid, so the Play closed test is off the launch path (M3-19). Account security
  (M1-7) is fully done: the maintainer confirmed passkey 2FA on the Azure account, and
  Namecheap's Domain Lock is on for perritodeck.dev.

## Decisions only the maintainer can make

Open: D11, reopened on 2026-10-24: the F-Droid test build showed the startup profile
changes from build to build, so builds don't match. It blocks only F-Droid's `Binaries`
(after v0.1.0), not the release. D3, D6, D7, D8, D12 and D13 were decided on 2026-10-17
and the rest on 2026-10-15, and each is in [DECISIONS](../DECISIONS.md).

| # | Decision | Recommendation, or what was decided | Blocks |
|---|---|---|---|
| ✅ D1 | **Whose name signs the Windows builds.** Azure Artifact Signing allows no custom name: an *individual* validation puts the legal name, city, state and country into every signature and the SmartScreen prompt (US and Canada only). Only a registered legal entity passing *organization* validation shows a project name. [SignPath Foundation](https://signpath.org/) signs open-source projects for free as "SignPath Foundation", but needs a public repo with a release already out | **Decided 2026-10-15:** keep the Azure Artifact Signing account. The maintainer is waiting for its identity validation and certificate. v0.1.0 ships unsigned (decided 2026-10-25: the validation is taking too long). Before wiring it in, check which name the certificate shows as publisher (an individual validation shows a legal name). SignPath stays the alternative | Windows signing (M3/M4) |
| ✅ D2 | **Mail for dev@perritodeck.dev:** which service receives it, and how to reply as dev@ | **Decided 2026-10-15:** ImprovMX (a grandfathered Light plan) forwards dev@ to the maintainer's mailbox. Cloudflare DNS has its `MX` records, SPF (`include:spf.improvmx.com ~all`), one DMARC record (`p=none`, aggregate reports to Cloudflare's DMARC Management) and ImprovMX's two DKIM `CNAME` records. Replies as dev@ go through Gmail's "Send mail as" with Gmail's own SMTP, so they fail DMARC alignment for perritodeck.dev. That's why DMARC stays at `p=none`: `p=reject` would bounce the maintainer's own replies. A mail-tester run scored this route about 5/10 for a real message. The upgrade is ImprovMX Premium (its SMTP; the DKIM records are already in DNS) or a free SMTP relay, either of which allows `p=reject`. **Before January 2027**, when Google removes "Send mail as" for custom domains, replies need another route | M1 (the contact everywhere) |
| ✅ D3 | **Where the plugin catalogue is built from:** `main` (plugin updates reach everyone before a release) or a release branch. The live site builds it from `main` today | **Decided 2026-10-17:** `main`. The site rebuilds the catalogue on every push to `main` (since D12, at each publish: by hand until the flip, then on every push to `main` that touches `website/` or `plugins/`), so plugin fixes reach users without an app release. Revisit when plugin manifests can declare a minimum host version | M1-5 |
| ✅ D4 | **Sign off the nominative Stream Deck, Elgato and Companion naming** (compatibility only, no logos) | **Decided 2026-10-15:** signed off as written | M1 |
| ✅ D5 | **First release scope:** the version, and which platforms and packages ship (the MSI was built once and never installed; macOS has never run on a real Mac) | **Decided 2026-10-15:** v0.1.0, Windows and Android only. The release publishes the Windows NSIS installer and the Android APK. The MSI, macOS, the Linux packages and the command-line archives are not in it. Linux and macOS still build and test in CI, and Linux keeps building from source; they join a later release once tested on real hardware. minSdk stays 22 (DECISIONS, 2026-10-12) | M3 |
| ✅ D6 | **RUSTSEC-2026-0192:** `ttf-parser` (behind `ab_glyph`, used to draw key titles) is unmaintained. Not a vulnerability | **Decided 2026-10-17:** accepted for now, with a reason in `host/deny.toml` (`[advisories] ignore`), and the library is replaced in a later release. `cargo deny check advisories` still fails on other advisories ([next-steps](../next-steps.md) item 8) | CI running advisories |
| ✅ D7 | **Play:** a separate upload key or the release key; turn on v3 signing or not | **Decided 2026-10-17**, for whenever Play happens: Play App Signing with the app-signing key Google generates, and our release key as the upload key. The Release workflow already signs the `play-bundle` AAB with the release key, so nothing in it changes. Play installs and GitHub, IzzyOnDroid or F-Droid installs can't update each other. v3 key rotation stays off | Play (not for now, D8) |
| ✅ D8 | **Store and verification identities:** a personal Play account shows the legal name, country and email; Android developer verification (global in 2027) is another identity check | **Decided 2026-10-17:** no Google Play for now. v0.1.0's Android app ships on GitHub Releases, then IzzyOnDroid and F-Droid; revisit if people ask for it. A personal Play account would publicly show the legal name, country and developer email, and an organization account needs a registered company. So the Play closed test (12 testers for 14 days), the Play listing, the data safety form and the Play account are off the launch path. The fastlane metadata stays: F-Droid and IzzyOnDroid use it. Google's Android developer verification isn't part of the decision: it's an open item to watch in M4 | Play |
| ✅ D9 | **Old Actions runs and artifacts** become public at the flip (E2E logs, the 2026-10-11 bundles) | **Decided 2026-10-15:** delete all runs and their artifacts right before the flip. Claude does it with the maintainer's OK at that moment (M1-10). **Done 2026-10-24:** all 257 runs and their artifacts | M1 |
| ✅ D10 | **One name for the desktop app.** "PerritoDeck" is `tauri.conf.json`'s `productName` and so the window title, tray tooltip, installer, Start menu and uninstall entry, Windows version info, macOS bundle and Linux `.desktop` name (`tauri.conf.json:3,16`, `src-tauri/src/lib.rs:704,764-766`, `packaging/flatpak/dev.perritodeck.PerritoDeck.desktop:3` and `.metainfo.xml:5`, `packaging/aur/PKGBUILD:76`), and the website's "PerritoDeck desktop app" (`index.html:104`, `privacy.html:46`). "PerritoDeck Host" was what the tablet apps told people to start: Android `strings.xml:14,18,32`, `net/Pairing.kt:64`, `net/DeckConnection.kt:474`; iOS `ScanView.swift:44`, `HomeView.swift:22,69`, `Info.plist:50`, `PerritoKit/Pairing.swift:43`, `PerritoKit/PinnedWebSocket.swift:31`; the website's `download.html:7,13,53` and `privacy.html:55`. The command-line program stays `perritodeck-host` either way | **Decided 2026-10-15:** "PerritoDeck", the `productName`. "PerritoDeck Host" is retired everywhere people see it: the tablet strings, the website and the store text change in one commit (renamed in the tree on 2026-10-15, not yet committed). The tablet app stays "Perrito", and the command-line binary keeps its file name, `perritodeck-host` | The store listing (M3) |
| D11 | **Reproducible Android builds and the startup profile.** `assets/dexopt/baseline.prof`, the ART startup profile R8 rewrites for the minified code, records the checksum of `classes.dex`, so it matches whenever the dex does. Measured on 2026-10-15 on one Windows PC (JDK 17, AGP 8.11.1): three clean release builds with the build cache off, in three folders on two drives, gave the same APK (2,179,425 bytes unsigned), while an incremental Kotlin compile changed the dex and so the profile (six classes when the compile came from the user-level build cache). With R8's profile rewriting off (only `android { experimentalProperties[...] }` does it; the `gradle.properties` flag has no effect on AGP 8.11.1) builds still match, but only 464 of the profile's 35,030 rules name a class in the obfuscated code (8,666 of 8,738 with it on), so the startup profile is effectively lost; turning off the ArtProfile tasks loses it outright on API 24+ | **Settled 2026-10-15 by measurement**, so no maintainer decision is needed: keep AGP's defaults and compare clean builds made with `--no-build-cache` ([fdroid/README.md](../fdroid/README.md#reproducible-builds)). Decide again only if CI's and F-Droid's builds differ; no comparison between machines has been run yet. **Reopened 2026-10-24:** in WSL, F-Droid's build and ours of the same commit differed only in `baseline.prof`, and that profile changed between runs of one setup too, so it isn't deterministic ([fdroid/README.md](../fdroid/README.md#reproducible-builds)). Decide before adding `Binaries` | The reproducible-build comparison and F-Droid's `Binaries` (M4) |
| ✅ D12 | **Where the website is hosted.** The live site is a Cloudflare Worker with static assets, set up in the dashboard, while the 2026-10-13 decision chose Cloudflare Pages with Git integration. The Worker answers unknown addresses with an empty 404 instead of `404.html` and redirects `/privacy.html` with a 307; the repository has no Wrangler config for it. The maintainer's other project, evi-ai.dev, is on GitHub Pages | **Decided 2026-10-17:** GitHub Pages, set up as evi-ai.dev is. A separate public repository, `perritodeck/perritodeck.github.io`, is the organization's Pages site (Deploy from a branch, `main`, `/`, with a `CNAME` file and `.nojekyll`), and Cloudflare does DNS only, never proxied. `website/` stays the only source and the site repository holds generated output only, written by `scripts/publish_site.py`: by hand from WSL until the flip, then by the Site publish workflow with a token in a `site` environment. GitHub Pages can't send headers, so the CSP (without `frame-ancestors`), the referrer policy and noindex become `<meta>` tags; framing protection, `nosniff`, `Permissions-Policy`, COOP, long cache lifetimes and noindex on files that aren't pages are given up, and so is the HSTS header, moot on `.dev`. The site's source is public on github.com before launch. `website/_headers` stays until the Worker is deleted, a few stable days after the cutover. The Worker, its dashboard-only settings and the Cloudflare GitHub app go after the cutover ([website.md, Cutover runbook](../website.md#cutover-runbook)). **Done 2026-10-24:** the cutover, with the Worker deleted at the swap (no rollback) and `website/_headers` deleted the same day (M1-5) | M1-5, M1-6 (the 404 page) |
| ✅ D13 | **The unmaintained advisories with no upgrade:** `zvariant`, `zbus_names` and `zvariant_derive` (RUSTSEC-2026-0370 to 0372, merged into zbus 6) and `proc-macro-error` (RUSTSEC-2024-0370). All Linux only, so not in v0.1.0; ashpd, Tauri's opener and single-instance plugins and Tauri's GTK stack haven't moved yet | **Decided 2026-10-17:** accepted for now, each with a reason in `host/deny.toml`, like D6. They go when ashpd and the two Tauri plugins move to zbus 6 and Tauri moves to gtk-rs 0.19; cargo-deny's `advisory-not-detected` warning flags a stale ignore. Every advisory with a fix was updated (rustls, zip, webpki-roots, rustls-webpki, tokio), so `cargo deny check` passes on both workspaces (2026-10-24) | CI running advisories |

## M1: public repo

| Item | Who | Needs / depends on |
|---|---|---|
| 1. ✅ Refresh the docs public readers hit first: `docs/ARCHITECTURE.md`, `ios/README.md` (it has the `ios` platform now), `protocol/README.md` (Swift types), the plugin READMEs that call tested Windows code unrun, the store listing text. Done 2026-10-13; on 2026-10-15 the store text was redone after review (plugins that aren't experimental first, fewer brand names; since D10 and D5 it says "PerritoDeck" for the desktop app, with an installer for Windows and Linux from source) and `ARCHITECTURE.md`, `host/README.md` and the Stream Deck crate's doc comment were corrected | C | |
| 2. ✅ Bring the website, mail and DNS docs in line with reality (nameservers already moved, the mail service, NEL and Speed Brain off, DNSSEC, the privacy URL). Done 2026-10-15: the site's notes moved to [`docs/website.md`](../website.md), whose "Deploy", "Where it stands" and "The domain" describe the live Worker, `www`, HSTS, NEL, DNSSEC and D2's mail, and whose "The project" asked which to keep (D12; on 2026-10-17 it gave way to the cutover runbook). Next-steps, the launch checklist, distribution and "Right now" above say the same | C | |
| 3. ✅ `bootstrap-gradle-wrapper.yml` no longer pushes to `main`. Done 2026-10-15, differently from planned: it pushes a signed-off `gradle-wrapper-<version>` branch and links a compare page from which a person opens the PR, so "Allow GitHub Actions to create and approve pull requests" stays off (DECISIONS). It hasn't run on GitHub yet (it runs only by hand) | C | |
| 4. ✅ Stale docs from today's review (Actions permissions are already read-only; MSI and Linux lines in HANDOFF; the Twitch Affiliate item, decided). Done 2026-10-15; the Affiliate decision is in DECISIONS, the Twitch test plan and the tools README | C | |
| 5. ✅ **Website: the cutover to GitHub Pages (D12)**, step by step in the [cutover runbook](../website.md#cutover-runbook). Done 2026-10-24: the D12 changes committed; `perritodeck.dev` verified for the organization (the challenge `TXT` record kept); the public site repository created and published by hand from WSL (`34861f5`, from `perritodeck@328162b`), with `pages-build-deployment` passing despite the billing block; the custom domain set in Pages; the DNS swap (GitHub's 4 `A` and 4 `AAAA` records and the `www` `CNAME`, all DNS only); the Let's Encrypt certificate approved within about a minute, then Enforce HTTPS. The Cloudflare Worker was deleted at the swap instead of a few stable days later, so there was no rollback; the `www` Redirect Rule and the Cloudflare GitHub app went too, and Web Analytics had no entry for the site. Then (C) `website/_headers` deleted, the CSP test made to read the `<meta>` tag alone, and `--check-live`'s skip for the Worker removed. The zone settings only the dashboard shows no longer matter. D3 keeps the catalogue on `main` (2026-10-17) | M (C checks) | D12 ✅ |
| 6. ✅ Checks after the cutover (in the runbook): `--check-live`, the Pages API's certificate and proxy state, the `http://`, `www` and `perritodeck.github.io` redirects, `/plugins/index.json` and a zip's SHA-256, the 404 page (GitHub serves `404.html`), `/README.md` and `/_headers` gone, DNSSEC and mail unchanged, no CSP errors, and **Look online** installing from the live catalogue. Done 2026-10-24 but for the last: every check passed (24 plugins, `application/json`, the zips' SHA-256s match; no console errors on `/`, `/download` and `/plugins/`). **Look online** passed in the real desktop app on 2026-10-24 | C | 5 ✅ |
| 7. ✅ Account security. Done 2026-10-15: 2FA is on for the maintainer's GitHub, Namecheap, Cloudflare and ImprovMX accounts; the perritodeck org requires 2FA (`two_factor_requirement_enabled: true`, and no member without 2FA, checked through the API); auto-renew is on at Namecheap. The two parts that check didn't cover were confirmed on 2026-10-17: the maintainer has passkey 2FA on the Azure account D1 keeps, and Namecheap's Domain Lock is on for perritodeck.dev (the `.dev` registry's RDAP shows "client transfer prohibited", checked 2026-10-17). Nothing is left to confirm | M | |
| 8. ✅ The stale `v0.1.0` tag. Done 2026-10-15: no other clone exists, and there are no tags in the repository or on GitHub | M | |
| 9. Create the `triage` and `plugin` labels the issue forms use | C+M | the maintainer's OK |
| 10. ✅ Right before the flip, delete every GitHub Actions run and its artifacts (D9: Claude does it with the maintainer's OK at that moment), and review Dependabot PR #1. Done 2026-10-24: all 257 runs and their artifacts deleted; PR #1 stays open until it has a green CI run of its own (M3-8) | C+M | the maintainer's OK then |
| 11. ✅ Final scans right before the flip: personal data in history, gitleaks over every ref. Done 2026-10-24: every commit is by dmang-dev or a bot; no ref's history has personal data (the upstream enigo author list is allowed); gitleaks is clean over every ref (the 8 fixture fingerprints are in `.gitleaksignore`); no signing material or `.env` was ever committed. GitHub still serves 8 deleted commits by SHA (old E2E artifacts and Dependabot heads, nothing personal): accepted. Organisation-level secrets weren't checked (the token lacked `admin:org`). The checklist's home-path search was fixed to find Windows paths (`1b69e47`) | C | everything above pushed |
| 12. ✅ **Flip the repository to public.** Done 2026-10-24 | M | 1–11 |
| 13. Repo settings after the flip: private vulnerability reporting, Dependabot alerts, secret scanning with push protection, rulesets for `main` (requiring CI gate) and `v*` tags, first-time contributor approval, require SHA-pinned actions, description/topics/website, and the `site` environment with `SITE_PUBLISH_TOKEN` (D12, [website.md](../website.md#after-the-flip-site-publish)). **Partly done 2026-10-24:** private vulnerability reporting, Dependabot alerts, first-time contributor approval, and the workflow token read-only. Left: secret scanning and push protection, Dependabot security updates, the `main` ruleset (CI gate has run, so it can be required now), the `v*` tag ruleset, the description, website and topics, SHA pinning (optional), and the `site` environment's branch limit with `SITE_PUBLISH_TOKEN` | M | 12 |
| 14. ✅ Confirm Actions runs again, Site publish and Site check included (a manual run of Site publish says "site: no change"; Site check passes) (the runbook's last steps). Done 2026-10-24: CI run 40157961726 on `01673dd` green in every job, iOS run 40157963719 green, the push's CI run green; Site publish built and checked the site, and with no token yet pushed nothing. The run that says "site: no change" waits on the token (M1-13) | M | 12 |

## M2: website live

| Item | Who | Needs / depends on |
|---|---|---|
| 1. Drop "the online catalogue isn't published yet" from the guide (`website/guide/plugins.html`). Unblocked: the live catalogue answers (2026-10-15) | C | M1-5 |
| 2. ✅ Open Graph and canonical tags, a social preview image. Done 2026-10-15: every page, the catalogue page's template in `plugins/build_catalog.py` included, has a canonical link, Open Graph and `twitter:card` tags matching `sitemap.xml`, and `website/img/og.png` (1200 × 630, made by `scripts/og-image.py`); `plugins/test_build_catalog.py` checks them. Live on the site since 2026-10-15. The repository's own social preview is a GitHub setting (launch checklist §4) | C | |
| 3. Remove the temporary noindex: the `robots` `<meta>` tag on every page but `404.html` and in the catalogue template, then `LAUNCHED = True` in `plugins/test_build_catalog.py` ([website.md, Launch day](../website.md#launch-day)) | C | M1-12; with the release if it follows within days, otherwise right after the flip |

## M3: first release

**Release blockers I can do now (C):**
1. ✅ **Licence texts in the APK and a licences screen** in Perrito (and a link in the desktop app). F-Droid-friendly: generated at build time, no Play oss-licenses. Done 2026-10-15: a Gradle task writes `assets/third-party-licenses.txt` from the release runtime classpath (86 libraries, POMs resolved through Gradle) and Open-source licences on Perrito's home screen shows it; the desktop app's Settings → Version → Open-source licences opens the installers' copy (DECISIONS). Not yet looked at on a device.
2. ✅ **App icons:** the paw-of-keys in the Android launcher (adaptive and monochrome, from `docs/assets/android/`), the desktop icons (`npx tauri icon` from `docs/assets/icon-master-rounded.svg`) and the iOS icon. Done 2026-10-15, with a fallback for API 22–25 (PNGs since 2026-10-24), the desktop window's brand mark, and the Flatpak and AUR drafts (they take the desktop icons). Seen 2026-10-24 on the Fire 7 and API 22 and 34 emulators (M3-10).
3. ✅ **The update check sees 0.x releases** (today every 0.x is a GitHub pre-release, which `/releases/latest` ignores). Done 2026-10-15: it lists `/releases` and picks by the rules in DECISIONS.
4. ✅ **16 KB page alignment** check of the release APK. Checked 2026-10-15, no change needed: its 12 native libraries are stored compressed (minSdk 22), so they're extracted at install and zip alignment doesn't apply, and every `LOAD` segment is 16 KB-aligned.
5. ✅ **"Experimental" labels** in the catalogue for plugins tested only against fakes. Done 2026-10-15: 13 plugins listed in `plugins/catalogue_status.json` get an Experimental tag and a "tested" note on the catalogue page, in `index.json` and in the desktop app's Look online (DECISIONS). Live on the site since 2026-10-15.
6. **Release notes** for the first release (what works, what's experimental, known issues), after the OBS and Twitch passes.

**Needs the maintainer or both:**

| Item | Who | Needs / depends on |
|---|---|---|
| 7. ✅ First CI run on the ~180 commits since 2026-10-11 (Host and Desktop on all platforms, by hand) and fix the fallout; then put CI back on push. Done 2026-10-24: the first run (CI by hand on `main`) failed host and plugin tests on Windows and macOS, three emulators and iOS; the fixes are `0073073`..`32f5f12` (E2E), `6a78156`..`2bc1e22` (iOS) and `9e04600`..`01673dd` (host and plugins). Then CI run 40157961726 on `01673dd` was green in every job (host and plugins on Windows, Linux and macOS, the desktop app, all four emulators, Android, protocol, licences and advisories, CI gate), iOS run 40157963719 was green, and pushes run CI again. Not run yet: Desktop app with platforms = all (the Linux and macOS bundles) | C+M | M1-14 ✅ |
| 8. Review and merge Dependabot PR #1. Reviewed 2026-10-24: merge it once it has a green CI run of its own | C+M | 7 ✅ |
| 9. Android release key: generate, back up twice, set 3 secrets + 2 variables, publish the fingerprint ([releasing](../releasing.md)). If Play ever happens, the same key is its upload key (D7) | M | the `release` environment once public |
| 10. Finish the minified-APK checks. Mostly done 2026-10-24: the Fire 7 and API 22 and 34 emulators ran pairing, keys, sliders, USB, the licences screen and the launcher icons, and four bugs they found are fixed. Then, on the Fire 7: the cable-takeover host fix is verified (twice, keys, the slider and the clock kept working on the cable after Wi-Fi went off), and pairing and the deck work over Wi-Fi. The "On this network" discovery fix couldn't be checked: the tablet's Android NSD never reported the host, even with Wi-Fi on, though the host's mDNS packets reach the tablet (Amazon's own JmDNS logs them). Likely a stale mdnsd (up 11 days), so a reboot of the tablet and a rerun are next. The PC's network was Public, which blocked inbound Wi-Fi; it's Private now. Left: QR decoding, the discovery recheck, and edge to edge on API 35+, where `SystemBarsTest` covers the Home, Licences and Pair screens (e2e stops at API 34) | C+M | tablet/emulator time |
| 11. Linux: compile and test today's commits (Linux isn't in v0.1.0, but it still builds and tests in CI and builds from source, D5); a real Linux desktop with a tablet before Linux joins a release | C+M | Linux hardware for the second part |
| 12. ✅ MSI and macOS: dropped from v0.1.0 (D5), and so are the Linux packages and the command-line archives. Done 2026-10-15: `SHIP`, at the top of `release.yml`, lists what a release publishes (`windows-nsis` for v0.1.0), and a tag runs only those build jobs, so a platform that isn't listed can't fail the release. The publish job checks that the draft gets exactly the NSIS installer, the APK, the plugin catalogue zip and `third-party-licenses.txt` (the desktop app's licences link points at the release's copy), plus `SHA256SUMS`, and the notes have rows only for those. A manual run with **untested** ticked builds the rest as workflow artifacts ([releasing, What ships](../releasing.md#what-ships)). It hasn't run on GitHub yet (it runs on a tag or by hand). Testing the MSI and macOS moves to the release that adds them | C | |
| 13. NSIS installer findings: fix or accept each ([launch-checklist](../launch-checklist.md) §7) | C+M | |
| 14. ✅ OBS real-world pass: finish, fix what it finds. Ran 2026-10-13; all 22 findings fixed and re-checked 2026-10-15 ([results](../testing/obs-results-2026-10-13.md)) | C | |
| 15. **Not for v0.1.0 (decided 2026-10-24):** the Twitch plugin ships Experimental. Twitch real-world pass with test accounts: the tools are built ([plugins/tools](../../plugins/tools/README.md)). Test accounts only; sections I, J and K (Affiliate) wait for a later pass. L6 (subs, cheers, hype trains) is open: the Twitch CLI's mock server, a download that needs the maintainer's OK, or that later pass ([next-steps](../next-steps.md) item 10) | C+M | a Twitch developer app and three test accounts |
| 16. Soak test (hours), sleep/resume, Wi-Fi drop | C+M | hardware time |
| 17. Remaining Windows manual test-plan steps | M | PC + tablet time |
| 18. ✅ `fdroid lint` and `fdroid build` on the recipe. Done 2026-10-24 in WSL (fdroidserver 2.4.5 and master): the recipe lints clean and a test copy builds. Fixed on the way: the recipe had comments, which fdroiddata's rewritemeta check rejects; F-Droid's scanner stopped at `plugins/sdk-node/package.json` without a lockfile; and the build server's JDK 21 can't satisfy `jvmToolchain(17)`, so the recipe installs OpenJDK 17 in `sudo:` ([fdroid/README.md](../fdroid/README.md#f-droid-main-repository)). It also showed that the baseline profile isn't deterministic, which reopens D11 before `Binaries:` | C+M | |
| 19. **Not for now (D8):** the Play closed test (12 testers, 14 days), the Play upload key (settled by D7 if Play happens) and the Play listing with its data safety form. Off the launch path; the notes for later are in [distribution, Google Play](../distribution.md#google-play) | — | D8, if it's revisited |
| 20. **Tag, check the draft, test the NSIS installer and the APK, publish** | M | all of the above but 19 |
| 21. Launch-day site edits (the pre-release text, download links, the fingerprint) and post-launch checks | C | 20 |

## M4: after the release

- **Windows signing** per D1: Azure Artifact Signing once its certificate arrives and the
  publisher name it shows has been checked (if that happens before v0.1.0, the first
  release is signed); SignPath as the alternative.
- **Linux and macOS in a release** (D5), each once tested on real hardware: a real Linux
  desktop with a tablet (M3-11), a real Mac. The MSI too, once it's been installed and
  tested.
- Store submissions: IzzyOnDroid, the F-Droid merge request, winget (the NSIS installer);
  Flathub and AUR once Linux is in a release (the drafts need the current plugin list and
  the MCP bridge). Google Play isn't one of them for now (D8).
- **To watch: Google's Android developer verification** (global from 2027). Apps
  installed on Google-certified Android devices, sideloaded ones included, will have to
  come from a verified developer. That affects the GitHub, IzzyOnDroid and F-Droid builds,
  but not Fire tablets, which aren't Google-certified. It's an open item, not a decision
  yet, and another identity check, as in D1 and D8.
- macOS entitlements and usage strings, Apple signing (D1-style identity question).
- ✅ A form with **Test connection** for built-in OBS, Home Assistant and Companion instead of
  hand-editing `integrations.toml` (done in M3, 2026-10-24; saves apply after a restart).
- Warn when importing a profile with keys that run commands or programs.
- Per-key image state, so page switches don't stall slow tablets (numbers in next-steps
  item 6).
- Small fixes: copy the MCP bridge only when MCP is on; Node SDK exits on shutdown; READMEs
  for the HTTP, MQTT and Now playing plugins; Dependabot entries for Gradle (ignoring bumps
  that need minSdk 23), cargo and npm. (A byte-identical catalogue build on Windows is in
  the tree since 2026-10-17: stored zips, file modes from git; DECISIONS.)
- Real-world checks: Companion 3/4, published Stream Deck plugins, the service plugins,
  a real Windows Firewall block, the Fire OS 5 hardware matrix.
- Reproducible-build comparison, then F-Droid's `Binaries` and `AllowedAPKSigningKeys`.
- The security model's remaining items, localization, an accessibility pass, iOS (needs CI
  on macOS and an Apple account), the announcement.

## Claude's queue, in order

Done by 2026-10-15, in the tree: the OBS pass and its fixes, the Twitch test tools, M1-1
to M1-4, M2-2, M3-1 to M3-5, M3-12 and D10's renames. On 2026-10-17: D6's ignore in
`host/deny.toml`, and D3, D6, D7, D8 and M1-7 written up in these docs; then D12's repo
side: the policy `<meta>` tags and their tests, the catalogue build that's the same on any
OS, `scripts/publish_site.py` with its tests, `site-publish.yml`, the drift check in
`site-check.yml`, and these docs with the cutover runbook. On 2026-10-24: the cutover, then
M1-10 and M1-11 before the flip, and after it the fixes that took CI green (M3-7) and the
real-app pass's bug (`13ae37a`).

1. ✅ Commit the 2026-10-15 work in reviewed pieces, with D10's renames (tablet strings,
   website, store text) as one commit (`25ee003`).
2. ✅ D12's repo side committed, and the cutover runbook worked through with the maintainer
   (M1-5): done 2026-10-24, with the first publish by hand, the read-only checks after each
   step, and then `website/_headers` deleted and the CSP test made to read the `<meta>` tag
   alone.
3. M2-1 (M1-6 is done: Look online passed in the real app on 2026-10-24). Until
   `SITE_PUBLISH_TOKEN` exists, publish by hand after each change to `website/` or
   `plugins/` on `main`, and run `--check-live` now and then.
4. ✅ M3-6: the release notes are written; the Twitch pass is skipped (2026-10-24). M1-10
   and M1-11 were done right before the flip (2026-10-24).
5. ✅ The advisories `cargo deny check advisories` flagged besides D6's: done 2026-10-24.
   Every crate with a fix was updated, and the maintainer accepted the four with none (D13).
6. ✅ M3-7: CI's first run since 2026-10-11, and its fallout fixed (2026-10-24).
7. M3-8: merge Dependabot PR #1 once it has a green CI run of its own.
8. M3-10 with the maintainer: the discovery recheck on the Fire 7 after a reboot, and QR
   decoding by hand. The release notes' TODO(apk) goes once those pass and CI is green on
   the commit that gets tagged.
9. The real-app pass's small findings and the Fire 7's possible improvements
   ([next-steps](../next-steps.md) items 14 and 15).
