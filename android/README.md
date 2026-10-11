# Perrito — Android client

Kotlin + Jetpack Compose. `applicationId dev.perritodeck.client` (debug builds add `.debug` so both can be installed side by side). minSdk 22, so it runs on Fire OS 5 and newer.

## Get an APK without Android Studio

Every push to `main` builds a debug APK in GitHub Actions: **Actions → Android → latest run → Artifacts → `perrito-debug-apk`**. Unzip it and install with `adb install -r app-debug.apk`, or copy it to the tablet and open it (allow "install unknown apps").

On a Kindle Fire: **Settings → Security & Privacy → Apps from Unknown Sources** (Fire OS 5), or allow it per app when prompted (Fire OS 7+).

## Build locally

Needs JDK 17 and the Android SDK (Android Studio installs both).

```sh
cd android
./gradlew testDebugUnitTest   # protocol fixtures + connection tests
./gradlew installDebug        # onto a connected device or running emulator
```

If `gradlew` is missing, run the **Bootstrap Gradle wrapper** workflow in GitHub Actions once (or `gradle wrapper --gradle-version 8.14.3` locally).

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
  data/       Settings (device id, paired hosts), PairingLink (QR link parser)
  ui/         HomeScreen, PairScreen, DeckScreen, theme
  MainActivity, DeckViewModel
```

No Google Play Services or proprietary libraries — only AndroidX, Compose, kotlinx and OkHttp, so the app stays F-Droid-clean.
