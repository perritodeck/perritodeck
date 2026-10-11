# Distribution

Where PerritoDeck ships, what each channel needs, and what's ready. Store text and images live in [`fastlane/metadata/android/`](../fastlane/metadata/android/) — F-Droid, IzzyOnDroid and Google Play (via `fastlane supply`) all read that layout.

## Identifiers (never change after first release)

| | |
|---|---|
| Android package | `dev.perritodeck.client` (debug: `dev.perritodeck.client.debug`) |
| Desktop bundle id | `dev.perritodeck.desktop` in `tauri.conf.json` (**undecided**: `dev.perritodeck.PerritoDeck` would suit Flatpak better; settle it before the first release, see [releasing](releasing.md#1-before-the-first-release-only)) |
| Store title | Perrito: tablet control deck |
| Contact | dev@perritodeck.dev |
| Privacy policy | https://perritodeck.dev/privacy.html |

**Trademarks:** store listings must not use "Stream Deck" or "Elgato" as keywords or comparisons; Google Play rejects listings that lean on other brands. The website may say "Stream Deck–style" descriptively. Every listing ends with the not-affiliated line.

## Android channels

### F-Droid (main repo)
- Needs: FOSS license ✔, no proprietary dependencies ✔, no Google Play Services ✔, `dependenciesInfo` block disabled ✔, fastlane metadata ✔, a tagged release (`v0.1.0`), reproducible build (optional but planned).
- Submit a merge request to `gitlab.com/fdroid/fdroiddata` adding `metadata/dev.perritodeck.client.yml`. The draft, the lint/build steps and the reproducible-build plan are in [`fdroid/`](fdroid/README.md).
- **Blocker:** the repository must be public before submission.
- Expect no anti-features. Cleartext traffic (Phase 1) isn't an anti-feature, but Phase 2's TLS removes the question.

### IzzyOnDroid
- Faster than F-Droid main (days, not weeks); picks up signed APKs from GitHub Releases.
- Needs a public repo, a release APK attached to a GitHub Release, and the same fastlane metadata. Request inclusion via an issue on `gitlab.com/IzzyOnDroid/repo`.

### GitHub Releases + Obtainium
- Attach `perrito-<version>.apk` (release-signed, not debug) to each tagged release. Obtainium users can then track it directly.

### Google Play
- **Developer account:** personal accounts created after Nov 2023 must run a **closed test with at least 12 testers for 14 continuous days** before production access. Start this early.
- **Identity verification** for the developer account takes a few days; the developer name shown publicly can be "PerritoDeck".
- **Target API:** new apps must target the latest required level (currently API 36) ✔.
- **App signing:** enrol in Play App Signing; upload an AAB signed with an upload key. Keep the upload key and the F-Droid/GitHub signing key separate and backed up offline.
- **Data safety form** — answers:
  - Does the app collect or share user data? **No.**
  - Is data encrypted in transit? **Phase 1: no** (local network only, nothing leaves the LAN); **Phase 2: yes**.
  - Account deletion: not applicable (no accounts).
- **Content rating (IARC):** utility app, no user-generated content, no ads → expected "Everyone".
- **Ads:** no. **In-app purchases:** none.
- **Permissions declaration:** none needed (only `INTERNET`; camera comes later and needs no special declaration).

### Amazon Appstore, Samsung Galaxy Store, Huawei AppGallery
- Optional. Amazon's store is the only way most Fire tablet owners install apps without sideloading, so it's worth doing after Play.

## Desktop channels (Phase 6)

| Channel | Status |
|---|---|
| GitHub Releases | Ready: `release.yml` builds NSIS `.exe` (per user) + `.msi` (per machine), a universal `.dmg`, AppImage/`.deb`/`.rpm`, the CLI host for three OSes, the plugin catalogue and `SHA256SUMS`, into a **draft** release. Unsigned until secrets exist ([releasing](releasing.md)) |
| Website downloads | Ready: [`website/download.html`](../website/download.html) links the release assets. The links work once the repository is public and a release is published |
| Flathub | Draft manifest in [`packaging/flatpak/`](../packaging/flatpak/) with open questions (app ID, resource path, sandboxing of "Run command") |
| AUR | Draft `PKGBUILD` in [`packaging/aur/`](../packaging/aur/) |
| winget | After the first release: `wingetcreate new` with the NSIS or MSI URL |
| Homebrew cask | After the first release, best once the app is notarised |
| macOS signing + notarisation | Wired up behind secrets; needs an Apple Developer account |
| Windows signing | Not wired up; SignPath Foundation (free for OSS) or Azure Trusted Signing |

## Still needed
- [ ] Real phone and 10" tablet screenshots (`images/phoneScreenshots`, `images/tenInchScreenshots`). There's a shot list in [`fdroid/README.md`](fdroid/README.md#screenshot-shot-list).
- [ ] Feature graphic 1024×500 (`images/featureGraphic.png`) for Play.
- [ ] Final launcher icon artwork (the paw in `images/icon.png` is a placeholder).
- [x] Release signing config and a `release.yml` workflow that builds, signs and attaches the APK on tags. Signing waits on the secrets.
- [ ] Set the signing secrets (Android first; Apple and Windows optional).
- [ ] Decide the desktop bundle identifier before the first release.
- [ ] Make the repository public before F-Droid/IzzyOnDroid/Flathub submission and before the download page goes live.
- [ ] The privacy policy says camera access comes "in a future version", but the app already requests `CAMERA` for QR scanning. Update `website/privacy.html` before the Play listing.
