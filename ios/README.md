# Perrito for iPhone and iPad

The iOS/iPadOS client for PerritoDeck: the same deck as the Android app, speaking the
same [protocol](../protocol/README.md). No third-party dependencies, no analytics; the
app talks only to the computer it's paired with.

**Status: written, not yet built.** It has never been compiled: building it needs a Mac,
and the weekly iOS workflow (`.github/workflows/ios.yml`) hasn't run so far. Expect the
first build to need fixes. It isn't part of the first release; before it can ship it also needs
a privacy manifest (`PrivacyInfo.xcprivacy`) and a TestFlight round.

| Path | What |
|---|---|
| `PerritoKit/` | Swift package with everything that isn't UI: protocol models and codec, pairing links, certificate fingerprints and pinning, the connection state machine, Bonjour discovery, Keychain storage, grid maths and slider throttling. Tested with XCTest against `protocol/fixtures`. |
| `Perrito/` | The SwiftUI app: home, pairing, QR scanner and deck screens. |
| `project.yml` | [XcodeGen](https://github.com/yonaskolb/XcodeGen) spec. The `.xcodeproj` is generated, not committed. |

## Build and run

Needs a Mac with Xcode 15 or newer (iOS 16+ deployment target).

```sh
brew install xcodegen
cd ios
xcodegen            # writes Perrito.xcodeproj from project.yml
open Perrito.xcodeproj
```

Pick the **Perrito** scheme and an iPhone or iPad simulator, then Run. To run on a
device, choose your team under *Signing & Capabilities* (the bundle id is
`dev.perritodeck.client`; change it if your team can't use that id).

Run `xcodegen` again after adding or removing files.

### Tests

```sh
cd ios/PerritoKit
swift test
```

The protocol tests read the golden files from `../../protocol/fixtures` (override with
`PERRITODECK_FIXTURES=/path/to/fixtures`). The iOS workflow runs these tests and builds
the app for the Simulator, unsigned, every Monday or by hand (Actions → iOS → Run
workflow).

### Trying it without a tablet

Start the host on the same Mac (`cargo run -p perritodeck-host -- --pair` in `host/`).
The Simulator shares the Mac's network, so on the pairing screen tap **This Mac**
(127.0.0.1) and type the code the host prints. Bonjour discovery also works in the
Simulator. The Simulator has no camera, so the QR scanner is hidden there; the system
Camera app on a real device also opens `perritodeck://pair` links.

## How it maps to the Android app

| Android | iOS |
|---|---|
| `protocol/Protocol.kt` | `PerritoKit/Protocol.swift` (Codable) |
| `net/Tls.kt` (`Fingerprint`, `PinningTrustManager`) | `Fingerprint.swift`, `PinnedWebSocket.swift` (URLSession trust challenge) |
| `net/Pairing.kt` | `Pairing.swift` |
| `net/DeckConnection.kt` | `DeckConnection.swift` (+ `ReconnectSchedule`, `SliderThrottle` in `DeckMath.swift`) |
| `net/Discovery.kt` (NsdManager) | `Discovery.swift` (NWBrowser) |
| `net/KeyImages.kt` | `KeyImages.swift` (ImageIO, PNG and WebP) |
| `data/PairingLink.kt` | `PairingLink.swift` |
| `data/Settings.kt` (SharedPreferences) | `Settings.swift` (Keychain for the device id and paired hosts) |
| `DeckViewModel.kt` | `Perrito/AppModel.swift` |
| `ui/*`, `scan/ScanScreen.kt` | `Perrito/*View.swift` |

Differences:

- **No USB.** Android reaches the host over a cable with `adb reverse`; iOS has no
  equivalent. `DeckConnection` still takes fallback addresses, so another route can be
  added the same way.
- **Platform.** `hello` says `"platform": "ios"` (the schema's `platform` enum and all
  three apps' types have it). `densityDpi` is reported in Android's terms: 160 per point
  of scale, so a 2× screen says 320.
- **Haptics** only do something on iPhone; iPads have no Taptic Engine.
- **Leaving the deck.** iOS has no back button, so the deck has a small menu (top right)
  with *Disconnect*.

## Security

Same model as Android (see `docs/SECURITY.md`): TLS to a self-signed host certificate
pinned by its SHA-256 fingerprint, learned at pairing (from the QR link, or compared by
eye for a typed code). The per-device secret and the paired hosts live in the Keychain,
readable only on this device after first unlock and never synced. Keychain items
survive uninstalling an app, so Perrito clears its own on the first launch after a
reinstall.
