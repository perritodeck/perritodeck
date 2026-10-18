# Phase 8: going public, the website, the first release

The plan for everything still open, in order. Each item was checked against the tree on
2026-10-13, and again on 2026-10-15, when M1-1 to M1-4, M2-2, M3-1 to M3-5 and M3-12
landed, the maintainer decided D1, D2, D4, D5, D9 and D10, D11 was settled by
measurement, M1-7 and M1-8 were done, and so was most of M1-5. The details behind each
one are in [next-steps](../next-steps.md) (items 8–18), the steps in order in the
[launch checklist](../launch-checklist.md), and the release mechanics in
[releasing](../releasing.md).

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

- **The site is live** on `https://perritodeck.dev` with the temporary noindex (checked
  on 2026-10-15). A Cloudflare Worker named `perritodeck` with static assets serves it on
  the custom domain; it isn't a Pages project, and `perritodeck.pages.dev` doesn't exist.
  The 2026-10-13 decision chose Pages, so which to keep is open (D12). HSTS is the site's
  own two-year header from `_headers`, and NEL is off. `www` answers 301 to the bare
  domain, keeping the path and query (one Redirect Rule, plus a proxied `A` record to
  192.0.2.1 for `www`). DNSSEC is in force: DS 2371/13/2 is at the `.dev` registry, and
  resolvers validate. Mail for dev@ is set up as D2 says. The site matches the current
  `main` (`6861f46`): `/README.md` answers 404, `/img/og.png` loads, and
  `/plugins/index.json` lists the 24 plugins with 13 marked Experimental (checked by 15:16
  UTC and again at 15:46). Pushes to `main` redeploy the Worker by themselves (a push at
  03:02:14 UTC on 2026-10-17 was live by 03:02:42). To confirm: the zone settings only the
  dashboard shows ([website.md, The domain](../website.md#the-domain), "Left"), and two
  parts of M1-7 that weren't in its check.
- **The OBS real-world pass is done.** It ran on 2026-10-13 against OBS 31.0.3 (93 of 110
  cases), and all 22 findings were fixed (OBS plugin 1.0.3 and the host) and re-checked
  against the same OBS on 2026-10-15
  ([testing/obs-results-2026-10-13.md](../testing/obs-results-2026-10-13.md)). The OBS
  plugin is no longer Experimental: 13 plugins are listed in
  `plugins/catalogue_status.json`. The cases that couldn't run yet are in the results' §7
  ([next-steps](../next-steps.md) item 9).
- **The Twitch real-world pass** waits for the maintainer's developer app and three test
  accounts. It uses test accounts only; the Affiliate cases are deferred (DECISIONS,
  2026-10-15).
- **CI is blocked until the flip.** GitHub Actions has stopped every run since 2026-10-11
  at a billing block, and the maintainer is leaving it, because Actions are free once the
  repo is public (DECISIONS, 2026-10-15). Until then everything is checked locally
  (Windows, and Linux in WSL). Right before the flip every old run and its artifacts are
  deleted (D9, M1-10). The first runs after it are the first CI since 2026-10-11, and the
  first on Ubuntu 26 (`ubuntu-latest` moves from 2026-10-19). Every workflow has to go
  green once before the first tag (launch checklist §4).
- **Decided on 2026-10-15:** D1, D2, D4, D5, D9 and D10, and D11 by measurement (the
  table below). Account security (M1-7) and the stale tag (M1-8) are done.

## Decisions only the maintainer can make

Open: D3, D6, D7, D8 and D12. The rest were decided on 2026-10-15 (D11 by measurement),
and each is in [DECISIONS](../DECISIONS.md).

| # | Decision | Recommendation, or what was decided | Blocks |
|---|---|---|---|
| ✅ D1 | **Whose name signs the Windows builds.** Azure Artifact Signing allows no custom name: an *individual* validation puts the legal name, city, state and country into every signature and the SmartScreen prompt (US and Canada only). Only a registered legal entity passing *organization* validation shows a project name. [SignPath Foundation](https://signpath.org/) signs open-source projects for free as "SignPath Foundation", but needs a public repo with a release already out | **Decided 2026-10-15:** keep the Azure Artifact Signing account. The maintainer is waiting for its identity validation and certificate. v0.1.0 ships unsigned unless the certificate arrives first. Before wiring it in, check which name the certificate shows as publisher (an individual validation shows a legal name). SignPath stays the alternative | Windows signing (M3/M4) |
| ✅ D2 | **Mail for dev@perritodeck.dev:** which service receives it, and how to reply as dev@ | **Decided 2026-10-15:** ImprovMX (a grandfathered Light plan) forwards dev@ to the maintainer's mailbox. Cloudflare DNS has its `MX` records, SPF (`include:spf.improvmx.com ~all`), one DMARC record (`p=none`, aggregate reports to Cloudflare's DMARC Management) and ImprovMX's two DKIM `CNAME` records. Replies as dev@ go through Gmail's "Send mail as" with Gmail's own SMTP, so they fail DMARC alignment for perritodeck.dev. That's why DMARC stays at `p=none`: `p=reject` would bounce the maintainer's own replies. A mail-tester run scored this route about 5/10 for a real message. The upgrade is ImprovMX Premium (its SMTP; the DKIM records are already in DNS) or a free SMTP relay, either of which allows `p=reject`. **Before January 2027**, when Google removes "Send mail as" for custom domains, replies need another route | M1 (the contact everywhere) |
| D3 | **Where the plugin catalogue is built from:** `main` (plugin updates reach everyone before a release) or a release branch. The live site builds it from `main` today | `main` for now; revisit when manifests get a minimum host version | M1-5 |
| ✅ D4 | **Sign off the nominative Stream Deck, Elgato and Companion naming** (compatibility only, no logos) | **Decided 2026-10-15:** signed off as written | M1 |
| ✅ D5 | **First release scope:** the version, and which platforms and packages ship (the MSI was built once and never installed; macOS has never run on a real Mac) | **Decided 2026-10-15:** v0.1.0, Windows and Android only. The release publishes the Windows NSIS installer and the Android APK. The MSI, macOS, the Linux packages and the command-line archives are not in it. Linux and macOS still build and test in CI, and Linux keeps building from source; they join a later release once tested on real hardware. minSdk stays 22 (DECISIONS, 2026-10-12) | M3 |
| D6 | **RUSTSEC-2026-0192:** `ttf-parser` (behind `ab_glyph`, used to draw key titles) is unmaintained. Not a vulnerability | Accept it with a reason in `host/deny.toml` for now; replace later | CI running advisories |
| D7 | **Play:** a separate upload key or the release key; turn on v3 signing or not | Separate upload key (Play App Signing); v3 off | Play closed test |
| D8 | **Store and verification identities:** a personal Play account shows the legal name, country and email; Android developer verification (global in 2027) is another identity check | Check which name each one shows before using it, as for D1 | Play, Android verification |
| ✅ D9 | **Old Actions runs and artifacts** become public at the flip (E2E logs, the 2026-10-11 bundles) | **Decided 2026-10-15:** delete all runs and their artifacts right before the flip. Claude does it with the maintainer's OK at that moment (M1-10). Not done yet | M1 |
| ✅ D10 | **One name for the desktop app.** "PerritoDeck" is `tauri.conf.json`'s `productName` and so the window title, tray tooltip, installer, Start menu and uninstall entry, Windows version info, macOS bundle and Linux `.desktop` name (`tauri.conf.json:3,16`, `src-tauri/src/lib.rs:704,764-766`, `packaging/flatpak/dev.perritodeck.PerritoDeck.desktop:3` and `.metainfo.xml:5`, `packaging/aur/PKGBUILD:76`), and the website's "PerritoDeck desktop app" (`index.html:104`, `privacy.html:46`). "PerritoDeck Host" was what the tablet apps told people to start: Android `strings.xml:14,18,32`, `net/Pairing.kt:64`, `net/DeckConnection.kt:474`; iOS `ScanView.swift:44`, `HomeView.swift:22,69`, `Info.plist:50`, `PerritoKit/Pairing.swift:43`, `PerritoKit/PinnedWebSocket.swift:31`; the website's `download.html:7,13,53` and `privacy.html:55`. The command-line program stays `perritodeck-host` either way | **Decided 2026-10-15:** "PerritoDeck", the `productName`. "PerritoDeck Host" is retired everywhere people see it: the tablet strings, the website and the store text change in one commit (renamed in the tree on 2026-10-15, not yet committed). The tablet app stays "Perrito", and the command-line binary keeps its file name, `perritodeck-host` | The store listing (M3) |
| ✅ D11 | **Reproducible Android builds and the startup profile.** `assets/dexopt/baseline.prof`, the ART startup profile R8 rewrites for the minified code, records the checksum of `classes.dex`, so it matches whenever the dex does. Measured on 2026-10-15 on one Windows PC (JDK 17, AGP 8.11.1): three clean release builds with the build cache off, in three folders on two drives, gave the same APK (2,179,425 bytes unsigned), while an incremental Kotlin compile changed the dex and so the profile (six classes when the compile came from the user-level build cache). With R8's profile rewriting off (only `android { experimentalProperties[...] }` does it; the `gradle.properties` flag has no effect on AGP 8.11.1) builds still match, but only 464 of the profile's 35,030 rules name a class in the obfuscated code (8,666 of 8,738 with it on), so the startup profile is effectively lost; turning off the ArtProfile tasks loses it outright on API 24+ | **Settled 2026-10-15 by measurement**, so no maintainer decision is needed: keep AGP's defaults and compare clean builds made with `--no-build-cache` ([fdroid/README.md](../fdroid/README.md#reproducible-builds)). Decide again only if CI's and F-Droid's builds differ; no comparison between machines has been run yet | The reproducible-build comparison and F-Droid's `Binaries` (M4) |
| D12 | **Keep the Worker or move to Pages.** The live site is a Cloudflare Worker with static assets, while the 2026-10-13 decision chose Cloudflare Pages with Git integration. The Worker answers unknown addresses with an empty 404 instead of `404.html`, redirects `/privacy.html` with a 307, and sends no `X-Content-Type-Options: nosniff`; the repository has no Wrangler config for it | Either works, and [website.md, The project](../website.md#the-project) lists what each takes. Keeping the Worker: commit a Wrangler config (`"not_found_handling": "404-page"`, the catalogue build), add `nosniff` to `_headers`, and turn off its `workers.dev` address and preview URLs. Moving to Pages: create the project, then move the domain, a few minutes down. Either way, record it in DECISIONS, since the 2026-10-13 entry says Pages | M1-5, M1-6 (the 404 page) |

## M1: public repo

| Item | Who | Needs / depends on |
|---|---|---|
| 1. ✅ Refresh the docs public readers hit first: `docs/ARCHITECTURE.md`, `ios/README.md` (it has the `ios` platform now), `protocol/README.md` (Swift types), the plugin READMEs that call tested Windows code unrun, the store listing text. Done 2026-10-13; on 2026-10-15 the store text was redone after review (plugins that aren't experimental first, fewer brand names; since D10 and D5 it says "PerritoDeck" for the desktop app, with an installer for Windows and Linux from source) and `ARCHITECTURE.md`, `host/README.md` and the Stream Deck crate's doc comment were corrected | C | |
| 2. ✅ Bring the website, mail and DNS docs in line with reality (nameservers already moved, the mail service, NEL and Speed Brain off, DNSSEC, the privacy URL). Done 2026-10-15: the site's notes moved to [`docs/website.md`](../website.md), whose "Deploy", "Where it stands" and "The domain" describe the live Worker, `www`, HSTS, NEL, DNSSEC and D2's mail, and whose "The project" asks which to keep (D12). Next-steps, the launch checklist, distribution and "Right now" above say the same | C | |
| 3. ✅ `bootstrap-gradle-wrapper.yml` no longer pushes to `main`. Done 2026-10-15, differently from planned: it pushes a signed-off `gradle-wrapper-<version>` branch and links a compare page from which a person opens the PR, so "Allow GitHub Actions to create and approve pull requests" stays off (DECISIONS). It hasn't run on GitHub yet (CI is blocked) | C | |
| 4. ✅ Stale docs from today's review (Actions permissions are already read-only; MSI and Linux lines in HANDOFF; the Twitch Affiliate item, decided). Done 2026-10-15; the Affiliate decision is in DECISIONS, the Twitch test plan and the tools README | C | |
| 5. **Website:** attach `perritodeck.dev`, `www` and the redirects, zone settings, DNSSEC ([docs/website.md](../website.md#deploy)). Mostly done 2026-10-15: a Cloudflare Worker named `perritodeck` with static assets serves the custom domain (not a Pages project; there's no `perritodeck.pages.dev`), with HSTS from `_headers`, NEL off, `www` answering 301 to the bare domain (one Redirect Rule and a proxied `A` record to 192.0.2.1) and DNSSEC in force (DS 2371/13/2 at the `.dev` registry). Pushes to `main` redeploy it by themselves (seen 2026-10-17). Left, to confirm with the maintainer: the zone settings only the dashboard shows (Look online itself works: the app's `PerritoDeck` user agent gets the catalogue, and only Python's default user agent is refused, by the Browser Integrity Check) ([website.md, The domain](../website.md#the-domain), "Left"); and D12 | M | D3, D12 |
| 6. Checks after attaching: headers, redirects, 404, `/plugins/index.json`, **Look online** installing from it. Partly done 2026-10-15 from outside: the headers, `http://` and `www` redirects, a 404 status (with an empty body, not `404.html`), `/README.md` gone and `index.json` (24 plugins, 13 marked Experimental) are right. Left: the 404 page (D12) and Look online installing from the live catalogue | C | 5 |
| 7. ✅ Account security. Done 2026-10-15: 2FA is on for the maintainer's GitHub, Namecheap, Cloudflare and ImprovMX accounts; the perritodeck org requires 2FA (`two_factor_requirement_enabled: true`, and no member without 2FA, checked through the API); auto-renew is on at Namecheap. Two parts of the item as planned weren't in that check, so they're to confirm with the maintainer: 2FA on the Azure account D1 keeps, and the registrar lock at Namecheap | M | |
| 8. ✅ The stale `v0.1.0` tag. Done 2026-10-15: no other clone exists, and there are no tags in the repository or on GitHub | M | |
| 9. Create the `triage` and `plugin` labels the issue forms use | C+M | the maintainer's OK |
| 10. Right before the flip, delete every GitHub Actions run and its artifacts (D9: Claude does it with the maintainer's OK at that moment), and review Dependabot PR #1 | C+M | the maintainer's OK then |
| 11. Final scans right before the flip: personal data in history, gitleaks over every ref | C | everything above pushed |
| 12. **Flip the repository to public** | M | 1–11 |
| 13. Repo settings after the flip: private vulnerability reporting, Dependabot alerts, secret scanning with push protection, rulesets for `main` (no required checks until the gate job exists) and `v*` tags, first-time contributor approval, require SHA-pinned actions, description/topics/website | M | 12 |
| 14. Confirm Actions runs again | M | 12 |

## M2: website live

| Item | Who | Needs / depends on |
|---|---|---|
| 1. Drop "the online catalogue isn't published yet" from the guide (`website/guide/plugins.html`). Unblocked: the live catalogue answers (2026-10-15) | C | M1-5 |
| 2. ✅ Open Graph and canonical tags, a social preview image. Done 2026-10-15: every page, the catalogue page's template in `plugins/build_catalog.py` included, has a canonical link, Open Graph and `twitter:card` tags matching `sitemap.xml`, and `website/img/og.png` (1200 × 630, made by `scripts/og-image.py`); `plugins/test_build_catalog.py` checks them. Live on the site since 2026-10-15. The repository's own social preview is a GitHub setting (launch checklist §4) | C | |
| 3. Remove the temporary noindex (`website/_headers`) | C | M1-12; with the release if it follows within days, otherwise right after the flip |

## M3: first release

**Release blockers I can do now (C):**
1. ✅ **Licence texts in the APK and a licences screen** in Perrito (and a link in the desktop app). F-Droid-friendly: generated at build time, no Play oss-licenses. Done 2026-10-15: a Gradle task writes `assets/third-party-licenses.txt` from the release runtime classpath (86 libraries, POMs resolved through Gradle) and Open-source licences on Perrito's home screen shows it; the desktop app's Settings → Version → Open-source licences opens the installers' copy (DECISIONS). Not yet looked at on a device.
2. ✅ **App icons:** the paw-of-keys in the Android launcher (adaptive and monochrome, from `docs/assets/android/`), the desktop icons (`npx tauri icon` from `docs/assets/icon-master-rounded.svg`) and the iOS icon. Done 2026-10-15, with the plain vector fallback for API 22–25, the desktop window's brand mark, and the Flatpak and AUR drafts (they take the desktop icons). Not yet seen on a device: the fallback needs an API 22/25 emulator or a Fire OS 5 tablet (the Fire 7 is API 28), the themed icon API 33+.
3. ✅ **The update check sees 0.x releases** (today every 0.x is a GitHub pre-release, which `/releases/latest` ignores). Done 2026-10-15: it lists `/releases` and picks by the rules in DECISIONS.
4. ✅ **16 KB page alignment** check of the release APK. Checked 2026-10-15, no change needed: its 12 native libraries are stored compressed (minSdk 22), so they're extracted at install and zip alignment doesn't apply, and every `LOAD` segment is 16 KB-aligned.
5. ✅ **"Experimental" labels** in the catalogue for plugins tested only against fakes. Done 2026-10-15: 13 plugins listed in `plugins/catalogue_status.json` get an Experimental tag and a "tested" note on the catalogue page, in `index.json` and in the desktop app's Look online (DECISIONS). Live on the site since 2026-10-15.
6. **Release notes** for the first release (what works, what's experimental, known issues), after the OBS and Twitch passes.

**Needs the maintainer or both:**

| Item | Who | Needs / depends on |
|---|---|---|
| 7. First CI run on the ~180 commits since 2026-10-11 (Host and Desktop on all platforms, by hand) and fix the fallout; then put CI back on push | C+M | M1-14 |
| 8. Review and merge Dependabot PR #1 | C+M | 7 |
| 9. Android release key: generate, back up twice, set 3 secrets + 2 variables, publish the fingerprint ([releasing](../releasing.md)) | M | D7; the `release` environment once public |
| 10. Finish the minified-APK checks: API 22 emulator, QR decoding, USB, sliders; also the licences screen, the launcher icons (fallback on API 22–25, themed on API 33+) and edge to edge on API 35+, where `SystemBarsTest` covers the Home, Licences and Pair screens once e2e runs | C+M | tablet/emulator time |
| 11. Linux: compile and test today's commits (Linux isn't in v0.1.0, but it still builds and tests in CI and builds from source, D5); a real Linux desktop with a tablet before Linux joins a release | C+M | Linux hardware for the second part |
| 12. ✅ MSI and macOS: dropped from v0.1.0 (D5), and so are the Linux packages and the command-line archives. Done 2026-10-15: `SHIP`, at the top of `release.yml`, lists what a release publishes (`windows-nsis` for v0.1.0), and a tag runs only those build jobs, so a platform that isn't listed can't fail the release. The publish job checks that the draft gets exactly the NSIS installer, the APK, the plugin catalogue zip and `third-party-licenses.txt` (the desktop app's licences link points at the release's copy), plus `SHA256SUMS`, and the notes have rows only for those. A manual run with **untested** ticked builds the rest as workflow artifacts ([releasing, What ships](../releasing.md#what-ships)). It hasn't run on GitHub yet (CI is blocked). Testing the MSI and macOS moves to the release that adds them | C | |
| 13. NSIS installer findings: fix or accept each ([launch-checklist](../launch-checklist.md) §7) | C+M | |
| 14. ✅ OBS real-world pass: finish, fix what it finds. Ran 2026-10-13; all 22 findings fixed and re-checked 2026-10-15 ([results](../testing/obs-results-2026-10-13.md)) | C | |
| 15. Twitch real-world pass with test accounts: the tools are built ([plugins/tools](../../plugins/tools/README.md)). Test accounts only; sections I, J and K (Affiliate) wait for a later pass. L6 (subs, cheers, hype trains) is open: the Twitch CLI's mock server, a download that needs the maintainer's OK, or that later pass ([next-steps](../next-steps.md) item 10) | C+M | a Twitch developer app and three test accounts |
| 16. Soak test (hours), sleep/resume, Wi-Fi drop | C+M | hardware time |
| 17. Remaining Windows manual test-plan steps | M | PC + tablet time |
| 18. `fdroid lint` and `fdroid build` on the recipe | C+M | OK to install fdroidserver |
| 19. Play closed test (12 testers, 14 days), started as soon as a signed AAB exists | M | 9, D7, D8, M1-5 |
| 20. **Tag, check the draft, test the NSIS installer and the APK, publish** | M | all of the above |
| 21. Launch-day site edits (the pre-release text, download links, the fingerprint) and post-launch checks | C | 20 |

## M4: after the release

- **Windows signing** per D1: Azure Artifact Signing once its certificate arrives and the
  publisher name it shows has been checked (if that happens before v0.1.0, the first
  release is signed); SignPath as the alternative.
- **Linux and macOS in a release** (D5), each once tested on real hardware: a real Linux
  desktop with a tablet (M3-11), a real Mac. The MSI too, once it's been installed and
  tested.
- Store submissions: IzzyOnDroid, the F-Droid merge request, Play production, winget (the
  NSIS installer); Flathub and AUR once Linux is in a release (the drafts need the current
  plugin list and the MCP bridge).
- macOS entitlements and usage strings, Apple signing (D1-style identity question).
- A form with **Test connection** for built-in OBS, Home Assistant and Companion instead of
  hand-editing `integrations.toml`.
- Warn when importing a profile with keys that run commands or programs.
- Per-key image state, so page switches don't stall slow tablets (numbers in next-steps
  item 6).
- Small fixes: copy the MCP bridge only when MCP is on; Node SDK exits on shutdown; READMEs
  for the HTTP, MQTT and Now playing plugins; a byte-identical catalogue build on Windows;
  Dependabot entries for Gradle (ignoring bumps that need minSdk 23), cargo and npm.
- Real-world checks: Companion 3/4, published Stream Deck plugins, the service plugins,
  a real Windows Firewall block, the Fire OS 5 hardware matrix.
- Reproducible-build comparison, then F-Droid's `Binaries` and `AllowedAPKSigningKeys`.
- The security model's remaining items, localization, an accessibility pass, iOS (needs CI
  on macOS and an Apple account), the announcement.

## Claude's queue, in order

Done by 2026-10-15, in the tree: the OBS pass and its fixes, the Twitch test tools, M1-1
to M1-4, M2-2, M3-1 to M3-5, M3-12 and D10's renames.

1. Commit the 2026-10-15 work in reviewed pieces, with D10's renames (tablet strings,
   website, store text) as one commit.
2. Ask the maintainer what only the dashboard shows: the zone settings and D12 (M1-5), and the two parts of M1-7 its check didn't
   cover.
3. M2-1, and the rest of M1-6: the 404 page once D12 is decided, and Look online
   installing from the live catalogue.
4. M3-6 after the Twitch pass. Right before the flip: M1-10 (delete every Actions run and
   its artifacts, with the maintainer's OK then) and M1-11.
