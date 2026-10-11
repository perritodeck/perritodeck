# Releasing

A release is one tag. Pushing `vX.Y.Z` runs the **Release** workflow, which builds every file and leaves a **draft** GitHub Release for a person to check and publish. Without signing secrets everything still builds, just unsigned. Releases are on hold until the maintainer says go.

## The checklist

### 1. Before the first release only
- [ ] Decide the desktop bundle identifier. It's `dev.perritodeck.desktop` in `host/app/src-tauri/tauri.conf.json`. It can never change after release (it names the macOS app's settings and the Windows install registry), and `distribution.md` used to say `dev.perritodeck.host`. It also makes an awkward Flatpak ID (`dev.perritodeck.desktop.desktop`). Something like `dev.perritodeck.PerritoDeck` reads better everywhere. If you change it, match the Flatpak app-id in `packaging/flatpak/`.
- [ ] Android signing secrets (below). Without them the APK is `-unsigned` and IzzyOnDroid/Obtainium can't use it.
- [ ] Optional: Apple Developer ID and notarisation secrets (below). Without them the Mac app is ad-hoc signed and people have to use "Open Anyway".
- [ ] Optional: Windows code signing (below). Without it, SmartScreen warns.
- [ ] Make the repository public before launch. The website's download links and every store need it.

### 2. Bump the version
```sh
python3 scripts/release-version.py 0.2.0     # sets every version below, then lists them
python3 scripts/release-version.py           # just list them (fails if they differ)
```
It changes these, all of which must match the tag (the workflow runs `--check`):

| File | Field |
|---|---|
| `host/Cargo.toml` | `[workspace.package] version` (every host crate inherits it) |
| `host/Cargo.lock`, `host/app/src-tauri/Cargo.lock` | the `perritodeck-*` entries, so `--locked` builds keep working |
| `host/app/src-tauri/Cargo.toml` | `[package] version` |
| `host/app/src-tauri/tauri.conf.json` | top-level `version` (installer and bundle versions) |
| `host/app/package.json`, `host/app/package-lock.json` | `version` |
| `android/app/build.gradle.kts` | `versionName`, and **`versionCode` + 1** |
| `ios/project.yml` | `MARKETING_VERSION`, and **`CURRENT_PROJECT_VERSION` + 1** |
| `website/download.html` | the version in every download link |

Not bumped automatically:
- **Plugins** each have their own version in `plugins/<id>/perritodeck-plugin.json`. Bump one when its code changes: the catalogue zip is named after the version, and a changed zip under an old name breaks the SHA-256 that existing catalogues pin. `plugins/sdk-node/package.json` is versioned on its own as well.
- **Protocol fixtures** (`protocol/fixtures/*.json`) contain example versions. Leave them alone.
- **Flatpak, AUR and F-Droid recipes**: they're updated after the tag (see `packaging/README.md` and `docs/fdroid/README.md`).

Pre-release tags like `v0.3.0-rc.1` work, but MSI needs numeric versions, so a pre-release ships only the NSIS `.exe` for Windows. Every `0.x` and every `-suffix` version is marked pre-release on GitHub.

### 3. Write the changelog
- [ ] `fastlane/metadata/android/en-US/changelogs/<new versionCode>.txt`: 500 characters at most, plain text. F-Droid, IzzyOnDroid and Play show it, and the release notes include it. The workflow fails without it.
- [ ] Optional: a `<release>` entry in `packaging/flatpak/dev.perritodeck.PerritoDeck.metainfo.xml`.

### 4. Check locally, commit
```sh
python3 scripts/release-version.py --check 0.2.0
cd host && cargo test && cd app/src-tauri && cargo check --locked
```
Commit ("release: v0.2.0"), push to `main` and let CI pass.

### 5. Tag (maintainer only)
```sh
git tag -a v0.2.0 -m "PerritoDeck 0.2.0" && git push origin v0.2.0
```
To try the whole build **without** a release, go to Actions → Release → Run workflow, give an existing tag and leave "publish" unticked. The files are then only kept as workflow artifacts.

### 6. What the workflow does
| Job | Output |
|---|---|
| Check versions | fails early if any version or the changelog is missing |
| Android APK | `perrito-X.Y.Z.apk` (or `-unsigned.apk`). The Play bundle `perrito-X.Y.Z.aab` stays a workflow artifact (`play-bundle`), not a release asset |
| Command-line host ×3 | `perritodeck-host-X.Y.Z-linux-x86_64.tar.gz`, `-windows-x86_64.zip`, `-macos-universal.tar.gz` (host + fakedeck, LICENSE, README) |
| Desktop app ×3 | `perritodeck-X.Y.Z-windows-x64-setup.exe` (NSIS, per user, no admin), `-windows-x64.msi` (per machine), `-macos-universal.dmg`, `-linux-x86_64.AppImage`, `-linux-amd64.deb`, `-linux-x86_64.rpm` |
| Plugin catalogue | `perritodeck-X.Y.Z-plugin-catalogue.zip`, what `plugins/build_catalog.py` produces (index.json, index.html, one zip per plugin) |
| Draft GitHub Release | all of the above plus `SHA256SUMS`, with notes made from the Android changelog plus GitHub's generated notes |

Linux builds run on Ubuntu 22.04 so they work with glibc 2.35 and newer. A published (non-draft) release is never overwritten: the job stops instead. The workflow never creates tags (`--verify-tag`).

Cost: 4 Linux, 2 Windows (2× rate) and 2 macOS (10× rate) jobs per run. Don't re-run it casually.

### 7. Publish
- [ ] Open the draft. Check the asset list, try the installers on each OS (and the APK on the Fire 7), and compare a couple of checksums.
- [ ] Press **Publish release**.
- [ ] Unzip the plugin catalogue into `website/plugins/` and deploy the website, which makes the download page links work. Point the home page's Download links at `download.html`.
- [ ] Stores: F-Droid picks up new tags by itself once listed; IzzyOnDroid picks up the APK; upload the `play-bundle` artifact to Play; update the Flathub and AUR recipes (`packaging/README.md`).

## Signing secrets (none are set yet)

Set with `gh secret set NAME -R perritodeck/perritodeck` (maintainer only). Each one is optional: whatever is missing is skipped with a warning, never a failure.

### Android: the release key
It signs every APK published outside Google Play (GitHub Releases, IzzyOnDroid, Obtainium, and F-Droid once builds are reproducible). **If it's lost, existing users can't update** and would have to uninstall first. Back it up offline (password manager plus an encrypted USB stick). The key has already been generated and handed over.

```sh
# Only if a new key is ever needed (valid for 30 years):
keytool -genkeypair -v -keystore perrito-release.jks -alias perrito \
  -keyalg RSA -keysize 4096 -validity 10950 -dname "CN=PerritoDeck, O=PerritoDeck"

base64 -w0 perrito-release.jks | gh secret set PERRITO_KEYSTORE_BASE64 -R perritodeck/perritodeck
gh secret set PERRITO_KEYSTORE_PASSWORD -R perritodeck/perritodeck
gh secret set PERRITO_KEY_ALIAS -R perritodeck/perritodeck --body perrito
gh secret set PERRITO_KEY_PASSWORD -R perritodeck/perritodeck
```
(On macOS use `base64 -i perrito-release.jks`. In PowerShell use `[Convert]::ToBase64String([IO.File]::ReadAllBytes("perrito-release.jks"))`.)

Google Play uses a separate **upload key** registered in the Play Console; Play re-signs with its own app-signing key. Keep the two apart. The workflow prints the release certificate's SHA-256 in the Android job summary. Publish it on the download page for people who verify APKs.

### macOS: Developer ID and notarisation
Needs an Apple Developer Program membership (99 USD a year).

| Secret | What |
|---|---|
| `APPLE_CERTIFICATE` | "Developer ID Application" certificate exported as `.p12`, base64 |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12` export password |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: <name> (<TEAMID>)` |
| `APPLE_API_ISSUER`, `APPLE_API_KEY` | App Store Connect API key: issuer ID and key ID (for notarisation) |
| `APPLE_API_KEY_P8_BASE64` | the `AuthKey_<id>.p8` file, base64 |

If only the certificate is set, the app is signed but not notarised. If nothing is set, `"signingIdentity": "-"` in `tauri.conf.json` ad-hoc signs it, which Apple silicon needs before it will run the app at all. The command-line host binaries are plain executables, ad-hoc signed by the linker, and aren't notarised.

### Windows: code signing (not set up)
Unsigned installers trigger SmartScreen until they build up reputation. Options, none chosen yet:
- **SignPath Foundation**: free certificates for open-source projects, used through their GitHub Action. Needs an application and a public repository.
- **Azure Trusted Signing**: about 10 USD a month, with identity validation. Used through `bundle.windows.signCommand` or Microsoft's signing action.
- OV/EV certificates on hardware tokens can't be used from hosted CI.

Once one is chosen, add a Windows-only step in the desktop job before "Build and bundle", behind a secret check like the macOS one.

## Not automated yet
winget manifest (`wingetcreate new` after the first release), Homebrew cask, Flathub and AUR submissions (drafts in `packaging/`), Play upload (`fastlane supply`), signed `SHA256SUMS` (minisign, or GitHub artifact attestations, which need a public repository), Linux ARM64 builds (GitHub's free ARM runners are for public repositories).
