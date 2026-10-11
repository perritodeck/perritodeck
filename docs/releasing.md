# Releasing

## One-time setup: the Android signing key

The release key signs every APK published outside Google Play (GitHub Releases, IzzyOnDroid, Obtainium). **If it's lost, existing users can't update** — they'd have to uninstall first. Back it up offline (password manager + an encrypted USB stick).

1. Create it (JDK's `keytool`, valid for 30 years):

   ```sh
   keytool -genkeypair -v -keystore perrito-release.jks -alias perrito \
     -keyalg RSA -keysize 4096 -validity 10950 \
     -dname "CN=PerritoDeck, O=PerritoDeck"
   ```

2. Add it to the repository's Actions secrets:

   ```sh
   base64 -w0 perrito-release.jks | gh secret set PERRITO_KEYSTORE_BASE64 -R perritodeck/perritodeck
   gh secret set PERRITO_KEYSTORE_PASSWORD -R perritodeck/perritodeck
   gh secret set PERRITO_KEY_ALIAS -R perritodeck/perritodeck --body perrito
   gh secret set PERRITO_KEY_PASSWORD -R perritodeck/perritodeck
   ```

   (macOS: `base64 -i perrito-release.jks` instead of `-w0`.)

Google Play uses a separate **upload key** registered in the Play Console; Play re-signs with its own app-signing key. Keep the two apart.

Without these secrets the release still runs and publishes an `-unsigned.apk`, which is fine for F-Droid (it builds and signs from source itself) but not installable directly.

## Cutting a release

1. Bump the version in two places, which must match the tag:
   - `android/app/build.gradle.kts` → `versionName`, and increment `versionCode`
   - `host/Cargo.toml` → `[workspace.package] version`
2. Add `fastlane/metadata/android/en-US/changelogs/<versionCode>.txt` (≤ 500 characters).
3. Commit, then tag and push:

   ```sh
   git tag v0.2.0 && git push origin v0.2.0
   ```

The **Release** workflow then:
- checks the tag matches both versions,
- builds the signed APK and AAB,
- builds `perritodeck-host` + `perritodeck-fakedeck` for Linux x86_64, Windows x86_64 and macOS (universal),
- publishes a GitHub Release with all files and `SHA256SUMS.txt`. Versions starting with `0.` or containing `-` are marked pre-release.

To rebuild an existing tag, run the workflow manually from the Actions tab with that tag.

## Not yet automated (Phase 6)
macOS signing + notarisation, Windows code signing and MSI/MSIX, Linux packages (Flatpak, .deb, .rpm, AppImage), Play upload via `fastlane supply`, winget/Homebrew manifests.
