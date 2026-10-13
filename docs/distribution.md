# Distribution

Where PerritoDeck ships, what each channel needs, and what's ready. Store text and images live in [`fastlane/metadata/android/`](../fastlane/metadata/android/) — F-Droid, IzzyOnDroid and Google Play (via `fastlane supply`) all read that layout.

## Identifiers (never change after first release)

| | |
|---|---|
| Android package | `dev.perritodeck.client` (debug: `dev.perritodeck.client.debug`) |
| Desktop bundle id | `dev.perritodeck.PerritoDeck` (`tauri.conf.json`, Flatpak app-id); never change it after the first release |
| Store title | Perrito: tablet control deck |
| Contact | dev@perritodeck.dev |
| Privacy policy | https://perritodeck.dev/privacy.html |

**Trademarks:** store listings must not use "Stream Deck" or "Elgato" as keywords or comparisons; Google Play rejects listings that lean on other brands. The website may say "Stream Deck–style" descriptively. Every listing ends with the not-affiliated line.

## Android channels

### F-Droid (main repo)
- Needs: FOSS license ✔, no proprietary dependencies ✔, no Google Play Services ✔, `dependenciesInfo` block disabled ✔, fastlane metadata ✔, a tagged release (`v0.1.0` or `v0.2.0`, see [`next-steps.md`](next-steps.md) item 13), reproducible build (optional but planned).
- Submit a merge request to `gitlab.com/fdroid/fdroiddata` adding `metadata/dev.perritodeck.client.yml`. The draft, the lint/build steps and the reproducible-build plan are in [`fdroid/`](fdroid/README.md).
- **Blocker:** the repository must be public before submission.
- Expect no anti-features: no trackers, no proprietary libraries, and the tablet talks to the computer over TLS.

### IzzyOnDroid
- Faster than F-Droid main (days, not weeks); picks up signed APKs from GitHub Releases.
- Needs a public repo, a release APK attached to a GitHub Release, and the same fastlane metadata. Request inclusion via an issue on `gitlab.com/IzzyOnDroid/repo`.

### GitHub Releases + Obtainium
- The Release workflow attaches `perrito-<version>.apk`, signed with the release key, to each tagged release, and the notes give the certificate's fingerprint. Obtainium users can then track it directly.

### Google Play
- **Developer account:** personal accounts created after Nov 2023 must run a **closed test with at least 12 testers for 14 continuous days** before production access. Start this early.
- **Identity verification** for the developer account takes a few days; the developer name shown publicly can be "PerritoDeck".
- **Target API:** new apps must target the latest required level (currently API 36) ✔.
- **App signing:** enrol in Play App Signing; upload an AAB signed with an upload key. Keep the upload key and the release key (GitHub Releases, IzzyOnDroid, and F-Droid once builds are reproducible) separate and backed up offline. Which key uploads to Play is still to decide ([`next-steps.md`](next-steps.md) item 11): today the Release workflow signs the `play-bundle` AAB with the release key.
- **Data safety form** — answers:
  - Does the app collect or share user data? **No.**
  - Is data encrypted in transit? **Yes**: the tablet and the computer talk over TLS with a pinned certificate, on the local network only.
  - Account deletion: not applicable (no accounts).
- **Content rating (IARC):** utility app, no user-generated content, no ads → expected "Everyone".
- **Ads:** no. **In-app purchases:** none.
- **Permissions declaration:** none needed. The app asks for `INTERNET` and `CAMERA`; the camera only scans the pairing QR code while the scanner is open, and nothing it sees is stored or sent. Neither permission needs a special declaration.

### Amazon Appstore, Samsung Galaxy Store, Huawei AppGallery
- Optional. Amazon's store is the only way most Fire tablet owners install apps without sideloading, so it's worth doing after Play.

## Desktop channels (Phase 6)

| Channel | Status |
|---|---|
| GitHub Releases | Ready: `release.yml` builds NSIS `.exe` (per user) + `.msi` (per machine), a universal `.dmg`, AppImage/`.deb`/`.rpm`, the CLI host for three OSes, the plugin catalogue, `third-party-licenses.txt` (also inside the installers and archives) and `SHA256SUMS`, into a **draft** release. The desktop apps are unsigned until their secrets exist ([releasing](releasing.md)). The workflow hasn't run yet |
| Website downloads | Ready: [`website/download.html`](../website/download.html) links the release assets. The links work once the repository is public and a release is published |
| Flathub | Draft manifest in [`packaging/flatpak/`](../packaging/flatpak/) with open questions (resource path, sandboxing of "Run command", start at login); it bundles 13 of the 24 plugins and no `perritodeck-mcp` |
| AUR | Draft `PKGBUILD` in [`packaging/aur/`](../packaging/aur/); the same plugin list and bridge gaps |
| winget | After the first release: `wingetcreate new` with the NSIS or MSI URL |
| Homebrew cask | After the first release, best once the app is notarised |
| macOS signing + notarisation | Wired up behind secrets; needs an Apple Developer account |
| Windows signing | Not wired up; SignPath Foundation (free for OSS) or Azure Trusted Signing |

## Still needed
- [x] Phone and 10" tablet screenshots: `images/phoneScreenshots/` has two at 1080×1920 and `images/tenInchScreenshots/` three at 1920×1200.
- [x] Feature graphic 1024×500 (`images/featureGraphic.png`) for Play.
- [x] Store icon: `images/icon.png` is the final paw-of-keys artwork.
- [ ] The same icon in the apps themselves: Perrito's launcher icon (`res/drawable/ic_launcher.xml`), the desktop icons, the iOS app icon and the Linux packages still use the old plain paw. The finished adaptive and monochrome set is in `docs/assets/android/`.
- [x] Release signing config and a `release.yml` workflow that builds, signs, checks (`apksigner`, and the `PERRITO_CERT_SHA256` pin when it's set) and attaches the APK on tags, with the certificate's fingerprint in the release notes. A tag build fails without the key.
- [ ] Android signing: the maintainer generates the new release key and sets three secrets and two variables (`PERRITO_KEY_ALIAS`, and `PERRITO_CERT_SHA256`, recommended), then puts the fingerprint on `website/download.html`. The pipeline is in place, and until then the Release workflow can't publish (Android CI builds the release APK unsigned either way). The steps are in [`next-steps.md`](next-steps.md) item 11. Apple and Windows signing are optional.
- [ ] Third-party licence texts in the APK, or a licences screen in Perrito: a release blocker. The desktop installers and the command-line archives already carry `third-party-licenses.txt` ([`launch-checklist.md`](launch-checklist.md) §3).
- [x] Desktop bundle identifier decided: `dev.perritodeck.PerritoDeck` (see the table above and `DECISIONS.md`).
- [ ] Make the repository public before F-Droid/IzzyOnDroid/Flathub submission and before the download page goes live.
- [x] The privacy policy covers the camera: it's used only to scan pairing QR codes, and nothing it sees is stored or sent (`website/privacy.html`).
- [ ] Bring the store text up to date: `full_description.txt` still says "PerritoDeck Host", leaves out sliders, QR pairing, pages, profiles, live values and plugins, and promises macOS before it has run on a Mac.
