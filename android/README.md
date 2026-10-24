# Perrito — Android client

Kotlin + Jetpack Compose. `applicationId dev.perritodeck.client` (debug builds add `.debug` so both can be installed side by side). minSdk 22, so it runs on Fire OS 5 and newer.

## Get an APK without Android Studio

Every push to `main` that touches the app builds a debug APK in GitHub Actions: **Actions → CI → the run for that push → Artifacts → `perrito-debug-apk`** (a run of Actions → Android → Run workflow has one too). Unzip it and install with `adb install -r app-debug.apk`, or copy it to the tablet and open it (allow "install unknown apps").

On a Kindle Fire: **Settings → Security & Privacy → Apps from Unknown Sources** (Fire OS 5), or allow it per app when prompted (Fire OS 7+).

## Build locally

Needs JDK 17 and the Android SDK (Android Studio installs both).

```sh
cd android
./gradlew testDebugUnitTest   # protocol fixtures + connection tests
./gradlew installDebug        # onto a connected device or running emulator
```

If `gradlew` is missing, run the **Bootstrap Gradle wrapper** workflow in GitHub Actions, then open the pull request its summary page links to and merge it (or run `gradle wrapper --gradle-version 8.14.3` locally).

## Connect to the host

Start `perritodeck-host`; it shows a QR code and a 6-digit code.

- **Scan the QR code** with the tablet's camera app: it opens Perrito and pairs automatically.
- **Or** tap **Pair with a code**, enter the computer's address and the code, then check the fingerprint matches the one the host prints.

| Setup | Address |
|---|---|
| Wi-Fi | Found automatically under "Found on this network", or your computer's IP |
| Android emulator | `10.0.2.2` |
| USB cable | `127.0.0.1` after `adb reverse tcp:46710 tcp:46710` |

After pairing, Perrito reconnects to the last computer whenever it starts. Back button disconnects. The grid is full-screen and keeps the display awake while connected.

## Layout

```
app/src/main/java/dev/perritodeck/client/
  protocol/   wire types + codec (mirrors host/crates/protocol)
  net/        DeckConnection (WebSocket, heartbeats, reconnect), Tls (pinning),
              Pairing, Discovery (mDNS via NsdManager)
  data/       Settings (device id, paired hosts), PairingLink (QR link parser),
              ThirdPartyLicenses (reads the licence list)
  ui/         HomeScreen, PairScreen, DeckScreen, LicensesScreen, theme
  MainActivity, DeckViewModel
app/licenses/ texts of licences other than Apache-2.0 that a library uses
```

No Google Play Services or proprietary libraries — only AndroidX (with CameraX), Compose, kotlinx, OkHttp and ZXing, so the app stays F-Droid-clean.

## Licences

**Open-source licences** at the bottom of the home screen shows Perrito's own licence (Apache-2.0), a link to the source and every library in the APK with its licence. The list is `assets/third-party-licenses.txt`, which the build writes for each variant (`generateReleaseThirdPartyLicenses` in `app/build.gradle.kts`) from the libraries actually packaged:

- It reads each library's licence from its POM, or from a parent POM when the library's own names none. Gradle resolves the POMs like any dependency, so it downloads one again if its cache cleanup has deleted it (the cleanup deletes files unused for 30 days, and Gradle builds without the POMs once it has the metadata).
- It copies any `NOTICE` file a library carries, and each licence text once (the Apache License 2.0 comes from the repository's `LICENSE`).
- A library under a licence with no text in `app/licenses/` stops the build, so a new dependency can't ship without its licence. Add the text there and an entry in `OTHER_LICENSES`. A licence for part of a library that its POM doesn't name goes in `UNDECLARED_LICENSES` (OkHttp's copy of the Public Suffix List, MPL-2.0).
- The file has no dates and a fixed order, so the same dependency versions always give the same bytes and the APK stays reproducible.

A configuration cache entry keeps the file paths it was stored with. If a build that reuses one stops because a POM is no longer at its path or a library file is missing, run it once with `--no-configuration-cache`: Gradle resolves the files again and downloads what the cleanup deleted.

`testDebugUnitTest` checks the generated list (`ThirdPartyLicensesTest`).

## Launcher icon

The icon is the paw from `docs/assets/`: `res/drawable/ic_launcher.xml` on Android 5 to 7 (API 22 to 25), and on Android 8 and newer the adaptive icon in `res/drawable-anydpi-v26/`, with a themed (monochrome) layer for Android 13. The files are copies of `docs/assets/android/`, and `LauncherIconTest` checks that they match and that each one places the paw where its SVG does.

To look at the API 22 to 25 icon, use the API 22 or API 25 emulator or a Fire OS 5 tablet. The Fire 7 test tablet runs Fire OS 7 (API 28), so it shows the adaptive icon.
