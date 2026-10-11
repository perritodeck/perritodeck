# Prior art

Surveyed October 2026. "Unverified" marks claims we couldn't confirm from a primary source. Release dates come from CHANGELOG/metainfo files and npm, not GitHub release pages.

## Summary

| Project | License | Stack | Phone/tablet client | Plugin model | Status (2026) |
|---|---|---|---|---|---|
| Bitfocus Companion | MIT (+CLA for core) | TypeScript/Node, web UI, Electron launcher | Via **Satellite API** (any client) | Device "modules"; v5 adds surface plugins | Active, v5.x |
| companion-satellite | MIT | TypeScript/Node 24 | Is a client (Pi/desktop) | — | Active, v3.5.0 (2026-10-08) |
| Macro Deck 3 | Apache-2.0 (branding excluded) | .NET 10 host, Tauri bootstrapper, Angular UI | Free web/PWA; **paid** native Android/iOS app | Out-of-process HTTP/WS JSON, signed store | Beta rewrite; MD2 deprecated |
| OpenDeck | GPL-3.0-or-later | Rust + Tauri 2 + SvelteKit | Via commercial sibling **Tacto** | **Runs Elgato SDK plugins** (+ OpenAction extensions) | Active, v2.14.x |
| StreamController | GPL-3.0 | Python, GTK4/libadwaita | None | Python plugins over RPyC | Beta (1.5.0-beta.16, 2026-08) |
| TilePad | Unverified (badge only, no LICENSE file found) | Rust + Tauri 2 + Svelte | Android touchscreen client (maturity unverified) | WebSocket, any language, HTML "inspectors" | Active (updated 2026-09) |
| Stream-Pi | GPL-3.0 | Java/JavaFX | Android, Raspberry Pi | Java | Largely stale |
| Touch Portal | Proprietary freemium | — | Android, iOS | Plugins need Pro ($13.99) | Active, v4.6 |
| Deckboard | Proprietary freemium | Electron | Android | — | Activity unverified |
| UP Deck | Proprietary | — | Android, iOS | OBS-focused | Dormant since 2020 |
| Bitfocus Buttons | Commercial | — | Tablets as UI | 500+ device APIs | Active, enterprise |

## Closest competitors

- **TilePad** is the nearest open-source analogue: same stack we planned (Rust + Tauri 2), WebSocket plugins with HTML inspectors, Android touchscreen devices as the surface. Its license and how finished the Android client is are unverified. See open question in `plan/phase-1.md` about collaborating vs. building separately.
- **Tacto** (tacto.live) is OpenDeck's developer's commercial phone-as-deck app: free for 6 keys, Pro $24.99 one-time or $1.49/month up to 64 keys.
- **Macro Deck 3** is well engineered but its native mobile client is paid and it's still in beta.

**Our differentiators:** a free, FOSS native Android client on F-Droid; first-class Linux/Wayland; tablet-native layouts (non-square tiles, sliders, dials, touch strip); Companion Satellite and Elgato-plugin compatibility in one app.

## Bitfocus Companion & the Satellite API

Companion targets broadcast/AV gear (switchers, PTZ cameras) rather than desktop macros. Its **Satellite API** lets any remote surface register with Companion and receive server-rendered button images — exactly what our Android client already needs to do.

Protocol (API ≥1.14):
- **Transport:** TCP `16622`, WebSocket `16623` (since Companion 3.5). Port may become configurable — don't hard-code.
- **Framing:** newline-terminated lines, `COMMAND KEY=VALUE …`; quoted values with spaces; booleans `0/1` or `true/false`.
- **Handshake:** server sends `BEGIN CompanionVersion=… ApiVersion=…`, then `CAPS` (1.10+) advertising e.g. `BITMAP_FORMATS=rgb,png,webp`, `NONSQUARE`, `SUBSCRIPTIONS`.
- **Keep-alive:** client `PING` ~every 2 s, server `PONG`; must also answer server pings.
- **Register:** `ADD-DEVICE DEVICEID= PRODUCT_NAME=` with `KEYS_TOTAL`, `KEYS_PER_ROW`, `BITMAPS=<px>`, `BITMAP_FORMAT`, `COLORS`, `TEXT`, `BRIGHTNESS`, `PINCODE_LOCK`, `CAN_CHANGE_PAGE`… or (1.9+) a base64 JSON `LAYOUT_MANIFEST` with style presets and per-control rows/cols for non-square keys, encoders and LED rings.
- **Client → server:** `KEY-PRESS … PRESSED=1/0`, `KEY-ROTATE … DIRECTION=±n`, `CHANGE-PAGE`, `PINCODE-KEY`, `SET-VARIABLE-VALUE`.
- **Server → client:** `KEY-STATE` (bitmap, color, base64 text, font size, LEDs), `KEYS-CLEAR`, `BRIGHTNESS`, `LOCKED-STATE`, `DEVICE-CONFIG`.
- **Bitmaps:** `rgb` = base64 raw RGB at requested size; `png`/`webp` arrive as `data:` URLs.
- **Discovery:** Companion v5 advertises satellite ports over mDNS. The companion-satellite client advertises itself as `companion-satellite` with a REST API on port 9999.

**Reuse:** implement Satellite as a second "host target" in the Android client (Phase 7 in the spec; cheap enough to consider earlier). Model our own protocol's image push on it.

Sources: https://companion.free/for-developers/Satellite-API · https://github.com/bitfocus/companion · https://github.com/bitfocus/companion-satellite

## Macro Deck 3

- Host is authoritative; clients use REST plus a ticketed JSON WebSocket. Public listener on 8193 (HTTP) / 8194 (HTTPS); a separate loopback listener for the desktop UI guarded by a per-launch secret.
- mDNS `_macrodeck._tcp`.
- **Pairing:** QR "connect link" encodes a versioned binary payload — instance name, endpoints with TLS flags, numeric pairing code, 12-byte host-identity fingerprint. Clients get per-device identities and scoped tokens (`admin` vs `client`).
- **USB:** ADB reverse tunnels — same approach we planned.
- Plugins run out of process, versioned protocol, store packages signed by the store. Widgets beyond buttons: sliders, media player, weather, graphs.

**Reuse (ideas, own implementation):** QR payload shape (endpoints + pairing code + host-key fingerprint), separate loopback admin listener, scoped client vs. admin roles, signed plugin packages.

Source: https://github.com/Macro-Deck-App/Macro-Deck

## OpenDeck & the OpenAction extensions

- Hosts the Elgato plugin WebSocket protocol on a dynamic port from 57116 upward, plus a static HTTP server (port+2) for property inspectors and assets.
- Launches plugins with `-port -pluginUUID -registerEvent -info`; Node plugins run under system `node`; Windows-only plugin binaries run under Wine on Linux/macOS.
- **OpenAction manifest extensions:** `CodePathLin`, `CodePaths` (keyed by Rust target triple), per-OS `manifest.{os}.json` patches, `DeviceNamespace`, `ApplicationsToMonitor`, `HasSettingsInterface`.
- **Device plugins** with a 2-character `DeviceNamespace` can `registerDevice` and send `keyDown`/`keyUp`/`encoderChange`/`touchscreenPress` — how non-Elgato hardware plugs in.
- Uses the `elgato-streamdeck` crate (MPL-2.0) for real hardware.

**Reuse:** be compatible with OpenAction (it's the de facto open standard). A PerritoDeck "device plugin" for OpenDeck would let OpenDeck users drive our tablet client too. **Do not copy code unless we choose GPL-3.0.**

Sources: https://github.com/nekename/OpenDeck · https://openaction.amankhanna.me/

## StreamController

Linux-only Python/GTK4 app for real Elgato hardware. No network/phone client and no Elgato-plugin compatibility. Worth borrowing ideas: Wayland compositor integrations (GNOME, KDE, Hyprland, Sway) for app-aware profile switching, a D-Bus scripting API, and "fake decks" for development without hardware.

Source: https://github.com/StreamController/StreamController

## Elgato Stream Deck SDK (compatibility target)

- Current: SDK 3.0.0 docs; manifest `SDKVersion` 2 or 3. `@elgato/streamdeck` 3.0.2 and `@elgato/cli` 1.10.2 (MIT, Corsair Memory Inc.).
- **Registration:** host launches plugin with `-port -pluginUUID -registerEvent -info`; plugin connects to `ws://127.0.0.1:<port>` and sends `{"event": <registerEvent>, "uuid": <pluginUUID>}`. `-info` JSON carries application info, colors, `devicePixelRatio`, `devices[]` (id, name, size {columns, rows}, type) and plugin info.
- **Events to plugins:** `keyDown`, `keyUp`, `willAppear`, `willDisappear`, `dialDown`, `dialUp`, `dialRotate` (`ticks`), `touchTap`, `didReceiveSettings`, `didReceiveGlobalSettings`, `propertyInspectorDidAppear/Disappear`, `sendToPlugin`, `titleParametersDidChange`, `deviceDidConnect/Change/Disconnect`, `applicationDidLaunch/Terminate`, `systemDidWakeUp`, `didReceiveDeepLink`.
- **Commands from plugins:** `setTitle`, `setImage` (path or base64 data URL), `setState`, `setFeedback`, `setFeedbackLayout`, `setTriggerDescription`, `showOk`, `showAlert`, `setSettings`/`getSettings`, `setGlobalSettings`/`getGlobalSettings`, `switchToProfile`, `sendToPropertyInspector`, `openUrl`, `logMessage`.
- **Manifest:** `UUID`, `Name`, `Version`, `SDKVersion`, `Author`, `Description`, `Icon`, `CodePath` (+Win/Mac), `OS[]`, `Software.MinimumVersion`, `Actions[]` (each with `UUID`, `Name`, `Icon`, `States[]`, `Controllers` Keypad/Encoder, `PropertyInspectorPath`…).
- **Terms:** SDK libraries are MIT. No published clause found on third-party hosts (unverified). Marketplace plugins can be DRM-protected and likely won't run outside Elgato's app (unverified). OpenDeck and Mirabox ship compatible hosts openly. Never use Elgato trademarks.

Sources: https://docs.elgato.com/streamdeck/sdk/references/websocket/plugin · https://docs.elgato.com/streamdeck/sdk/references/manifest · https://github.com/elgatosf/streamdeck

## Takeaways for PerritoDeck

1. **Own protocol, Satellite-shaped.** JSON over WebSocket, host-rendered images as lossless PNG/WebP, explicit capability negotiation. Keep the Satellite mapping easy so the Android client can speak both.
2. **Pairing:** QR with endpoints + one-time code + host public-key fingerprint; per-device keys afterwards; ADB-reverse USB.
3. **Plugins:** our native protocol should be a near-superset of the Elgato SDK event/command names so a compatibility layer is mostly a shim; support OpenAction manifest extensions.
4. **License matters for reuse:** GPL-3.0 would let us borrow from OpenDeck/StreamController/Stream-Pi; MIT/Apache-2.0 would let us share code with Companion (and TilePad, if it is MIT as its badge suggests) but forbids copying from the GPL projects.
