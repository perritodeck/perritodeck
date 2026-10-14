# Phase 8: going public, the website, the first release

The plan for everything still open, in order. Each item was checked against the tree on
2026-10-13. The details behind each one are in [next-steps](../next-steps.md) (items 8–18),
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

- **The site is down.** The nameservers moved to Cloudflare at 15:47 UTC, but the old apex
  `A` record still points at the registrar's forwarder. Every visitor, and the app's
  **Look online**, gets a 522 after about 19 s. The fix is step M1-5: create the Pages
  project, delete that record, and attach the domain (with noindex still on).
- **The OBS real-world pass** has run its plugin path and is finishing the built-in path,
  the error cases and the report ([testing/obs.md](../testing/obs.md)).
- **GitHub Actions** has failed every run since 2026-10-11 because of a billing block. CI
  comes back when the repo is public or billing is sorted.

## Decisions only the maintainer can make

| # | Decision | Recommendation | Blocks |
|---|---|---|---|
| D1 | **Whose name signs the Windows builds.** Azure Artifact Signing allows no custom name: an *individual* validation puts the legal name, city, state and country into every signature, the SmartScreen prompt and the MSI's UAC prompt (US and Canada only). Only a registered legal entity passing *organization* validation shows a project name. [SignPath Foundation](https://signpath.org/) signs open-source projects for free as "SignPath Foundation", but needs a public repo with a release already out | Ship the first release unsigned as planned, then apply to SignPath. Keep Azure only if a legal entity exists or is planned; Azure bills the full month from account creation, so delete the account if it won't be used | Windows signing (M3/M4) |
| D2 | **Mail for dev@**: DNS now points mail at ImprovMX, not Cloudflare Email Routing. Keep one, add SPF and DMARC, and decide how to reply without showing a personal address | Keep ImprovMX if it's set up; add `v=spf1 include:spf.improvmx.com ~all` and `_dmarc` `v=DMARC1; p=none` | M1 (the contact everywhere) |
| D3 | **Where the plugin catalogue is built from:** `main` (plugin updates reach everyone before a release) or a release branch | `main` for now; revisit when manifests get a minimum host version | M1-5 |
| D4 | **Sign off the nominative Stream Deck/Elgato naming** (compatibility only, no logos) | Sign off as written | M1 |
| D5 | **First release scope:** v0.1.0 or v0.2.0; ship or drop the MSI (built once, never installed); ship or drop macOS (never run on a real Mac); keep minSdk 22 | v0.1.0; drop macOS from the first release unless a Mac test happens; keep the MSI only if it's tested | M3 |
| D6 | **RUSTSEC-2026-0192:** `ttf-parser` (behind `ab_glyph`, used to draw key titles) is unmaintained. Not a vulnerability | Accept it with a reason in `host/deny.toml` for now; replace later | CI running advisories |
| D7 | **Play:** a separate upload key or the release key; turn on v3 signing or not | Separate upload key (Play App Signing); v3 off | Play closed test |
| D8 | **Store and verification identities:** a personal Play account shows the legal name, country and email; Android developer verification (global in 2027) is another identity check | Decide alongside D1 | Play, Android verification |
| D9 | **Old Actions runs and artifacts** become public at the flip (E2E logs, the 2026-10-11 bundles) | Delete runs older than 2026-10-12 | M1 |

## M1: public repo

| Item | Who | Needs / depends on |
|---|---|---|
| 1. Refresh the docs public readers hit first: `docs/ARCHITECTURE.md`, `ios/README.md` (it has the `ios` platform now), `protocol/README.md` (Swift types), the plugin READMEs that call tested Windows code unrun, the store listing text | C | |
| 2. Bring the website, mail and DNS docs in line with reality (nameservers already moved, the mail service, NEL and Speed Brain off, DNSSEC, the privacy URL) | C | D2 |
| 3. `bootstrap-gradle-wrapper.yml` opens a signed-off PR instead of pushing to `main` | C | |
| 4. Stale docs from today's review (Actions permissions are already read-only; MSI and Linux lines in HANDOFF; the Twitch Affiliate item, decided) | C | |
| 5. **Website:** Pages project, delete the leftover apex record, attach `perritodeck.dev`, `www` and the redirects, zone settings, DNSSEC ([website/README.md](../../website/README.md#deploy)) | M | D2, D3, D4 |
| 6. Checks after attaching: headers, redirects, 404, `/plugins/index.json`, **Look online** installing from it | C | 5 |
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
| 1. Drop "the online catalogue isn't published yet" from the guide | C | M1-5 |
| 2. Open Graph and canonical tags, a social preview image | C | |
| 3. Remove the temporary noindex (`website/_headers`) | C | M1-12; with the release if it follows within days, otherwise right after the flip |

## M3: first release

**Release blockers I can do now (C):**
1. **Licence texts in the APK and a licences screen** in Perrito (and a link in the desktop app). F-Droid-friendly: generated at build time, no Play oss-licenses.
2. **App icons:** the paw-of-keys in the Android launcher (adaptive and monochrome, from `docs/assets/android/`), the desktop icons (`npx tauri icon` from `docs/assets/icon-master-rounded.svg`) and the iOS icon.
3. **The update check sees 0.x releases** (today every 0.x is a GitHub pre-release, which `/releases/latest` ignores).
4. **16 KB page alignment** check of the release APK.
5. **"Experimental" labels** in the catalogue for plugins tested only against fakes.
6. **Release notes** for the first release (what works, what's experimental, known issues), after the OBS and Twitch passes.

**Needs the maintainer or both:**

| Item | Who | Needs / depends on |
|---|---|---|
| 7. First CI run on the ~180 commits since 2026-10-11 (Host and Desktop on all platforms, by hand) and fix the fallout; then put CI back on push | C+M | M1-14 |
| 8. Review and merge Dependabot PR #1 | C+M | 7 |
| 9. Android release key: generate, back up twice, set 3 secrets + 2 variables, publish the fingerprint ([releasing](../releasing.md)) | M | D7; the `release` environment once public |
| 10. Finish the minified-APK checks: API 22 emulator, QR decoding, USB, sliders | C+M | tablet/emulator time |
| 11. Linux: compile and test today's commits; a real Linux desktop with a tablet | C+M | Linux hardware for the second part |
| 12. MSI and macOS: test or drop (D5) | C+M | D5 |
| 13. NSIS installer findings: fix or accept each ([launch-checklist](../launch-checklist.md) §7) | C+M | |
| 14. OBS real-world pass: finish, fix what it finds | C | running now |
| 15. Twitch real-world pass with test accounts: the tools are built ([plugins/tools](../../plugins/tools/README.md)) | C+M | a Twitch developer app and three test accounts |
| 16. Soak test (hours), sleep/resume, Wi-Fi drop | C+M | hardware time |
| 17. Remaining Windows manual test-plan steps | M | PC + tablet time |
| 18. `fdroid lint` and `fdroid build` on the recipe | C+M | OK to install fdroidserver |
| 19. Play closed test (12 testers, 14 days), started as soon as a signed AAB exists | M | 9, D7, D8, M1-5 |
| 20. **Tag, check the draft, test the installers and the APK, publish** | M | all of the above |
| 21. Launch-day site edits (the pre-release text, download links, the fingerprint) and post-launch checks | C | 20 |

## M4: after the release

- **Windows signing** per D1 (SignPath application, or Azure with an organization).
- Store submissions: IzzyOnDroid, the F-Droid merge request, Play production, winget; then
  Flathub and AUR (the drafts need the current plugin list, the MCP bridge and the new icon).
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

1. Finish the OBS pass and fix what it finds; commit the Twitch test tools.
2. M1-1 to M1-4 (docs, website docs, the bootstrap workflow, stale lines).
3. M3-1 to M3-5 (licences, icons, update check, 16 KB check, experimental labels).
4. M2-2 (Open Graph tags).
5. M1-6 and M2-1 once the domain is attached; M1-11 right before the flip.
