# Phase 8: going public, the website, the first release

The plan for everything still open, in order. Each item was checked against the tree on
2026-10-13, and again on 2026-10-15 when M1-1, M1-3, M1-4, most of M1-2, M2-2 and M3-1
to M3-5 landed. The
details behind each one are in [next-steps](../next-steps.md) (items 8–18),
the steps in order in the [launch checklist](../launch-checklist.md), and the release
mechanics in [releasing](../releasing.md).

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

- **The site is live** on `https://perritodeck.dev`, serving `main` with the temporary
  noindex (checked from outside on 2026-10-15): `/plugins/index.json` lists the 24 plugins,
  `http://` and `www` redirect to the bare domain with a 301 (path and query kept), every
  response carries `_headers`' two-year HSTS and no `Nel` or `Report-To` header, DNSSEC is
  in force (the DS record is at the `.dev` registry and resolvers validate the answers),
  and SPF and DMARC (`p=none`) are in place. So HSTS, NEL, the `www` redirect and the DS
  record are done too. Left: check whether the live project is a Pages project or a
  Worker (it has no `pages.dev` address, redirects `/privacy.html` with a 307, answers
  `/nope` with an empty 404 and lacks Pages' `nosniff` header; see
  [website.md, The project](../website.md#the-project)), confirm the zone settings that
  can't be seen from outside ([The domain](../website.md#the-domain), step 1), and D2.
  Until today's work is pushed, `/README.md` still serves the old notes, `/img/og.png` is
  a 404 and `index.json` marks no plugin Experimental.
- **The OBS real-world pass** ran on 2026-10-13 against OBS 31.0.3 (93 of 110 cases), and
  its 22 findings were fixed and re-checked against the same OBS on 2026-10-15
  ([testing/obs-results-2026-10-13.md](../testing/obs-results-2026-10-13.md)). Follow-up
  fixes are still landing.
- **The Twitch real-world pass** waits for the maintainer's developer app and three test
  accounts. It uses test accounts only; the Affiliate cases are deferred (DECISIONS,
  2026-10-15).
- **GitHub Actions** has failed every run since 2026-10-11 because of a billing block. The
  maintainer is leaving it: CI comes back when the repo goes public, where Actions are free.
  Until then everything is checked locally (Linux in WSL). The first runs after the flip are
  the first CI since 2026-10-11, and the first on Ubuntu 26 (`ubuntu-latest` moves from
  2026-10-19). Every workflow has to go green once before the first tag (launch checklist §4).

## Decisions only the maintainer can make

| # | Decision | Recommendation | Blocks |
|---|---|---|---|
| D1 | **Whose name signs the Windows builds.** Azure Artifact Signing allows no custom name: an *individual* validation puts the legal name, city, state and country into every signature, the SmartScreen prompt and the MSI's UAC prompt (US and Canada only). Only a registered legal entity passing *organization* validation shows a project name. [SignPath Foundation](https://signpath.org/) signs open-source projects for free as "SignPath Foundation", but needs a public repo with a release already out | Ship the first release unsigned as planned, then apply to SignPath. Keep Azure only if a legal entity exists or is planned; Azure bills the full month from account creation, so delete the account if it won't be used | Windows signing (M3/M4) |
| D2 | **Mail for dev@**: DNS points mail at ImprovMX, not Cloudflare Email Routing, and the SPF record (`v=spf1 include:spf.improvmx.com ~all`) and DMARC (`v=DMARC1; p=none`) are in place (2026-10-15). Confirm the service, send a test mail, and decide how to reply without showing a personal address | Keep ImprovMX if it's set up (the SPF record already names it); switching means editing that record, not adding one | M1 (the contact everywhere) |
| D3 | **Where the plugin catalogue is built from:** `main` (plugin updates reach everyone before a release) or a release branch. The live site builds it from `main` today | `main` for now; revisit when manifests get a minimum host version | M1-5 |
| D4 | **Sign off the nominative Stream Deck/Elgato naming** (compatibility only, no logos) | Sign off as written | M1 |
| D5 | **First release scope:** v0.1.0 or v0.2.0; ship or drop the MSI (built once, never installed); ship or drop macOS (never run on a real Mac); keep minSdk 22 | v0.1.0; drop macOS from the first release unless a Mac test happens; keep the MSI only if it's tested | M3 |
| D6 | **RUSTSEC-2026-0192:** `ttf-parser` (behind `ab_glyph`, used to draw key titles) is unmaintained. Not a vulnerability | Accept it with a reason in `host/deny.toml` for now; replace later | CI running advisories |
| D7 | **Play:** a separate upload key or the release key; turn on v3 signing or not | Separate upload key (Play App Signing); v3 off | Play closed test |
| D8 | **Store and verification identities:** a personal Play account shows the legal name, country and email; Android developer verification (global in 2027) is another identity check | Decide alongside D1 | Play, Android verification |
| D9 | **Old Actions runs and artifacts** become public at the flip (E2E logs, the 2026-10-11 bundles) | Delete runs older than 2026-10-12 | M1 |
| D10 | **One name for the desktop app.** "PerritoDeck" is `tauri.conf.json`'s `productName` and so the window title, tray tooltip, installer, Start menu and uninstall entry, Windows version info, macOS bundle and Linux `.desktop` name (`tauri.conf.json:3,16`, `src-tauri/src/lib.rs:704,764-766`, `packaging/flatpak/dev.perritodeck.PerritoDeck.desktop:3` and `.metainfo.xml:5`, `packaging/aur/PKGBUILD:76`), and the website's "PerritoDeck desktop app" (`index.html:104`, `privacy.html:46`). "PerritoDeck Host" is what the tablet apps tell people to start: Android `strings.xml:14,18,32`, `net/Pairing.kt:64`, `net/DeckConnection.kt:474`; iOS `ScanView.swift:44`, `HomeView.swift:22,69`, `Info.plist:50`, `PerritoKit/Pairing.swift:43`, `PerritoKit/PinnedWebSocket.swift:31`; the website's `download.html:7,13,53` and `privacy.html:55`; and the store text until this is decided. The command-line program stays `perritodeck-host` either way | "PerritoDeck": it's what the computer shows in every installed place, and the other name lives in strings. Change the tablet strings, the website and the store text in one commit | The store listing (M3) |
| D11 | **Reproducible Android builds and the startup profile.** `assets/dexopt/baseline.prof`, the ART startup profile R8 rewrites for the minified code, records the checksum of `classes.dex`, so it matches whenever the dex does. Measured on 2026-10-15 on one Windows PC (JDK 17, AGP 8.11.1): three clean release builds with the build cache off, in three folders on two drives, gave the same APK (2,179,425 bytes unsigned), while an incremental Kotlin compile changed the dex and so the profile (six classes when the compile came from the user-level build cache). With R8's profile rewriting off (only `android { experimentalProperties[...] }` does it; the `gradle.properties` flag has no effect on AGP 8.11.1) builds still match, but only 464 of the profile's 35,030 rules name a class in the obfuscated code (8,666 of 8,738 with it on), so the startup profile is effectively lost; turning off the ArtProfile tasks loses it outright on API 24+ | Keep AGP's defaults and compare clean builds made with `--no-build-cache` ([fdroid/README.md](../fdroid/README.md#reproducible-builds)). Decide again only if CI's and F-Droid's builds differ; no comparison between machines has been run yet | The reproducible-build comparison and F-Droid's `Binaries` (M4) |

## M1: public repo

| Item | Who | Needs / depends on |
|---|---|---|
| 1. ✅ Refresh the docs public readers hit first: `docs/ARCHITECTURE.md`, `ios/README.md` (it has the `ios` platform now), `protocol/README.md` (Swift types), the plugin READMEs that call tested Windows code unrun, the store listing text. Done 2026-10-13; on 2026-10-15 the store text was redone after review (plugins that aren't experimental first, fewer brand names, "PerritoDeck Host" until D10) and `ARCHITECTURE.md`, `host/README.md` and the Stream Deck crate's doc comment were corrected | C | |
| 2. Bring the website, mail and DNS docs in line with reality (nameservers already moved, the mail service, NEL and Speed Brain off, DNSSEC, the privacy URL). Mostly done 2026-10-15: the site's notes moved to [`docs/website.md`](../website.md), and next-steps, the launch checklist, distribution and "Right now" above describe what's live. Left: `docs/website.md` itself, whose "Where it stands" and steps 1–3 of "The domain" still describe the state before that day's fixes (`www` giving 522, HSTS `max-age=0`, NEL on, no DS record). D2 itself is still open | C | D2 |
| 3. ✅ `bootstrap-gradle-wrapper.yml` no longer pushes to `main`. Done 2026-10-15, differently from planned: it pushes a signed-off `gradle-wrapper-<version>` branch and links a compare page from which a person opens the PR, so "Allow GitHub Actions to create and approve pull requests" stays off (DECISIONS). It hasn't run on GitHub yet (CI is blocked) | C | |
| 4. ✅ Stale docs from today's review (Actions permissions are already read-only; MSI and Linux lines in HANDOFF; the Twitch Affiliate item, decided). Done 2026-10-15; the Affiliate decision is in DECISIONS, the Twitch test plan and the tools README | C | |
| 5. **Website:** Pages project, delete the leftover apex record, attach `perritodeck.dev`, `www` and the redirects, zone settings, DNSSEC ([docs/website.md](../website.md#deploy)). Mostly done by 2026-10-15: attached and serving `main`, `www` redirecting, HSTS from `_headers`, NEL off, DNSSEC in force. Left: Pages or Worker ([The project](../website.md#the-project)) and the zone settings only the dashboard shows | M | D2, D3, D4 |
| 6. Checks after attaching: headers, redirects, 404, `/plugins/index.json`, **Look online** installing from it. Partly done 2026-10-15 from outside: the headers, `http://` and `www` redirects, a 404 status (with an empty body, not `404.html`) and `index.json` (24 plugins) are right. Left: the 404 page, `/README.md` gone after the next deploy, and Look online installing from the live catalogue | C | 5 |
| 7. Account security: org 2FA required; 2FA and auto-renew at the registrar; 2FA on Cloudflare, ImprovMX and Azure | M | |
| 8. Delete the stale `v0.1.0` tag in the other clone | M | |
| 9. Create the `triage` and `plugin` labels the issue forms use | C+M | the maintainer's OK |
| 10. Review what becomes public: old runs and artifacts, Dependabot PR #1 | C+M | D9 |
| 11. Final scans right before the flip: personal data in history, gitleaks over every ref | C | everything above pushed |
| 12. **Flip the repository to public** | M | 1–11 |
| 13. Repo settings after the flip: private vulnerability reporting, Dependabot alerts, secret scanning with push protection, rulesets for `main` (no required checks until the gate job exists) and `v*` tags, first-time contributor approval, require SHA-pinned actions, description/topics/website | M | 12 |
| 14. Confirm Actions runs again | M | 12 |

## M2: website live

| Item | Who | Needs / depends on |
|---|---|---|
| 1. Drop "the online catalogue isn't published yet" from the guide (`website/guide/plugins.html`). Unblocked: the live catalogue answers (2026-10-15) | C | M1-5 |
| 2. ✅ Open Graph and canonical tags, a social preview image. Done 2026-10-15: every page, the catalogue page's template in `plugins/build_catalog.py` included, has a canonical link, Open Graph and `twitter:card` tags matching `sitemap.xml`, and `website/img/og.png` (1200 × 630, made by `scripts/og-image.py`); `plugins/test_build_catalog.py` checks them. Live after the next deploy. The repository's own social preview is a GitHub setting (launch checklist §4) | C | |
| 3. Remove the temporary noindex (`website/_headers`) | C | M1-12; with the release if it follows within days, otherwise right after the flip |

## M3: first release

**Release blockers I can do now (C):**
1. ✅ **Licence texts in the APK and a licences screen** in Perrito (and a link in the desktop app). F-Droid-friendly: generated at build time, no Play oss-licenses. Done 2026-10-15: a Gradle task writes `assets/third-party-licenses.txt` from the release runtime classpath (86 libraries, POMs resolved through Gradle) and Open-source licences on Perrito's home screen shows it; the desktop app's Settings → Version → Open-source licences opens the installers' copy (DECISIONS). Not yet looked at on a device.
2. ✅ **App icons:** the paw-of-keys in the Android launcher (adaptive and monochrome, from `docs/assets/android/`), the desktop icons (`npx tauri icon` from `docs/assets/icon-master-rounded.svg`) and the iOS icon. Done 2026-10-15, with the plain vector fallback for API 22–25, the desktop window's brand mark, and the Flatpak and AUR drafts (they take the desktop icons). Not yet seen on a device: the fallback needs an API 22/25 emulator or a Fire OS 5 tablet (the Fire 7 is API 28), the themed icon API 33+.
3. ✅ **The update check sees 0.x releases** (today every 0.x is a GitHub pre-release, which `/releases/latest` ignores). Done 2026-10-15: it lists `/releases` and picks by the rules in DECISIONS.
4. ✅ **16 KB page alignment** check of the release APK. Checked 2026-10-15, no change needed: its 12 native libraries are stored compressed (minSdk 22), so they're extracted at install and zip alignment doesn't apply, and every `LOAD` segment is 16 KB-aligned.
5. ✅ **"Experimental" labels** in the catalogue for plugins tested only against fakes. Done 2026-10-15: 13 plugins listed in `plugins/catalogue_status.json` get an Experimental tag and a "tested" note on the catalogue page, in `index.json` and in the desktop app's Look online (DECISIONS). Live after the next deploy.
6. **Release notes** for the first release (what works, what's experimental, known issues), after the OBS and Twitch passes.

**Needs the maintainer or both:**

| Item | Who | Needs / depends on |
|---|---|---|
| 7. First CI run on the ~180 commits since 2026-10-11 (Host and Desktop on all platforms, by hand) and fix the fallout; then put CI back on push | C+M | M1-14 |
| 8. Review and merge Dependabot PR #1 | C+M | 7 |
| 9. Android release key: generate, back up twice, set 3 secrets + 2 variables, publish the fingerprint ([releasing](../releasing.md)) | M | D7; the `release` environment once public |
| 10. Finish the minified-APK checks: API 22 emulator, QR decoding, USB, sliders; also the licences screen, the launcher icons (fallback on API 22–25, themed on API 33+) and edge to edge on API 35+, where `SystemBarsTest` covers the Home, Licences and Pair screens once e2e runs | C+M | tablet/emulator time |
| 11. Linux: compile and test today's commits; a real Linux desktop with a tablet | C+M | Linux hardware for the second part |
| 12. MSI and macOS: test or drop (D5) | C+M | D5 |
| 13. NSIS installer findings: fix or accept each ([launch-checklist](../launch-checklist.md) §7) | C+M | |
| 14. ✅ OBS real-world pass: finish, fix what it finds. Ran 2026-10-13; all 22 findings fixed and re-checked 2026-10-15 ([results](../testing/obs-results-2026-10-13.md)) | C | |
| 15. Twitch real-world pass with test accounts: the tools are built ([plugins/tools](../../plugins/tools/README.md)). Test accounts only; sections I, J and K (Affiliate) wait for a later pass. L6 (subs, cheers, hype trains) is open: the Twitch CLI's mock server, a download that needs the maintainer's OK, or that later pass ([next-steps](../next-steps.md) item 10) | C+M | a Twitch developer app and three test accounts |
| 16. Soak test (hours), sleep/resume, Wi-Fi drop | C+M | hardware time |
| 17. Remaining Windows manual test-plan steps | M | PC + tablet time |
| 18. `fdroid lint` and `fdroid build` on the recipe | C+M | OK to install fdroidserver |
| 19. Play closed test (12 testers, 14 days), started as soon as a signed AAB exists | M | 9, D7, D8, M1-5 |
| 20. **Tag, check the draft, test the installers and the APK, publish** | M | all of the above |
| 21. Launch-day site edits (the pre-release text, download links, the fingerprint) and post-launch checks | C | 20 |

## M4: after the release

- **Windows signing** per D1 (SignPath application, or Azure with an organization).
- Store submissions: IzzyOnDroid, the F-Droid merge request, Play production, winget; then
  Flathub and AUR (the drafts need the current plugin list and the MCP bridge).
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

Done by 2026-10-15: the OBS pass and its fixes, the Twitch test tools, M1-1, M1-3, M1-4,
most of M1-2, M2-2 and M3-1 to M3-5.

1. Bring `docs/website.md`'s "Where it stands" and "The domain" up to what's live (M1-2),
   then commit the 2026-10-15 work in reviewed pieces; it deploys with the next push to
   `main`.
2. M2-1 and the rest of M1-6 once that push is live (the 404 page, `/README.md` gone,
   Look online installing from the live catalogue).
3. D10's renames once the maintainer picks the name (tablet strings, website, store text,
   in one commit).
4. M3-6 after the Twitch pass; M1-11 right before the flip.
