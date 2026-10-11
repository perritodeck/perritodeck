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

| Setup | Host field | Notes |
|---|---|---|
| Android emulator | `10.0.2.2` | Host runs with default settings |
| USB cable | `127.0.0.1` | Run `adb reverse tcp:46710 tcp:46710` first |
| Wi-Fi | your computer's IP | Start host with `--listen 0.0.0.0`; enter the printed token |

Back button disconnects. The grid is full-screen and keeps the display awake while connected.

## Layout

```
app/src/main/java/dev/perritodeck/client/
  protocol/   wire types + codec (mirrors host/crates/protocol)
  net/        DeckConnection: WebSocket, handshake, heartbeats, reconnect
  data/       Settings (last host, device id)
  ui/         ConnectScreen, DeckScreen, theme
  MainActivity, DeckViewModel
```

No Google Play Services or proprietary libraries — only AndroidX, Compose, kotlinx and OkHttp, so the app stays F-Droid-clean.
