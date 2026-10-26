# IzzyOnDroid inclusion request for Perrito (v0.1.0)

Prepared on 2026-10-25 for the maintainer to send. Nothing has been posted, and no account
has been made.

## Read this first: IzzyOnDroid's AI policy

IzzyOnDroid's [App Inclusion Policy](https://izzyondroid.org/docs/general/AppInclusionPolicy/)
now has an AI section. In short, it is strongly opposed to apps created fully or in part by
generative AI tools, says such requests "will most likely be rejected", rejects vibe-coded
apps and apps that integrate with LLM services, allows LLM-written docs but wants the code
free of it, and treats a lack of transparency as grounds for rejection. The request form
asks how much "AI" was used.

The honest answers for this project:
- 316 of the 321 commits up to `v0.1.0`, and 42 of the 44 that touch `android/`, carry a
  `Co-Authored-By: Claude` trailer. On the form's scale that is **"Dominant"**, with Claude
  (Claude Code) as the tool.
- The store description advertises the desktop app's MCP support ("AI assistants that
  speak the Model Context Protocol can see your buttons…"), and a plugin talks to Ollama.
  Both are on the desktop side, not in the APK, but reviewers read the description.

So under the current policy the request is **very likely to be rejected**. Sending it with
the AI fields understated isn't an option (and the commit history would show it anyway).
Whether to send it at all is the maintainer's call; if not, the Android channels are GitHub
Releases (also through Obtainium) and F-Droid ([SUBMIT.md](SUBMIT.md)), whose inclusion
policy has no rule on AI-assisted code. The text below is filled in truthfully in case it's
sent.

## Criteria, checked against v0.1.0

| Criterion (policy and form) | Perrito v0.1.0 |
|---|---|
| Free licence, OSI/FSF approved | ✔ Apache-2.0 (`LICENSE`; plugins MIT) |
| Code openly accessible | ✔ https://github.com/perritodeck/perritodeck, public since 2026-10-24 |
| No proprietary components, no trackers | ✔ `android/app/build.gradle.kts` depends only on AndroidX/Compose, CameraX, OkHttp, ZXing and kotlinx; all 86 libraries in the APK are Apache-2.0 (listed in its `assets/third-party-licenses.txt`). No Play Services, Firebase, analytics or ads. F-Droid's scanner finds nothing in the APK. IzzyOnDroid runs its own scanner (Katastima/apkscanner) on request; not run here |
| `usesCleartextTraffic` avoided | ✔ `android:usesCleartextTraffic="false"`; the app talks TLS to the paired computer only |
| Targets end users | ✔ |
| No downloads of executable code without consent; no self-updater | ✔ The app downloads nothing executable and has no update check |
| Unique package name and display name | ✔ `dev.perritodeck.client`, "Perrito" |
| Repository has a proper description | ✔ GitHub description: "Turn an old Android tablet or phone into a Stream Deck–style control surface…" |
| Fastlane metadata in the repo (short and full description, icon, screenshots) | ✔ `fastlane/metadata/android/en-US/` at the repository root (the default path): `title.txt`, `short_description.txt`, `full_description.txt`, `images/icon.png`, `featureGraphic.png`, two phone and three ten-inch screenshots, `changelogs/1.txt` |
| APK signed with a release key, not debuggable or testOnly | ✔ `perrito-0.1.0.apk` is signed v1+v2, certificate SHA-256 `848c5d3f31f3bef31ecdaf308d4fa1db6a2c0a78b63ce6dacb3d6bacc98f24e5`; release build, not debuggable |
| APK attached to a tagged GitHub release | ✔ https://github.com/perritodeck/perritodeck/releases/tag/v0.1.0 (but marked **pre-release**, see below) |
| APK size ≤ about 30 MB | ✔ 2,249,787 bytes, one universal APK |
| No controversial topics in name, description, graphics | ✔ |
| Not an LLM front-end, no LLM integration | ✔ for the APK; the desktop app has optional MCP support (disclose it, see above) |
| AI policy | ✘ see above |

## Fix or note before sending

1. **The release is a GitHub pre-release.** IzzyOnDroid's updater may skip pre-releases.
   Say in the request that releases are marked pre-release while the project is pre-alpha
   and ask them to include them (the text below does), or mark a later release as latest.
2. **The request goes to Codeberg now**, not GitLab: https://codeberg.org/IzzyOnDroid/repodata/issues,
   template "App Inclusion Request" (title `[AppRequest] <App Name>`). It needs a Codeberg
   account; a handle is enough.
3. **Reproducible builds.** IzzyOnDroid rebuilds apps and shows a badge when the builds
   match. v0.1.0 wouldn't match: its baseline profile varied between builds (D11). From
   v0.1.1 (AGP 8.11.2) builds are reproducible ([README.md](README.md#reproducible-builds)).
   Not a blocker for inclusion.
4. The AI fields and the two "AI Accountability" boxes are the maintainer's own statements:
   check them against what actually happened before ticking anything.

## The request (Codeberg form, field by field)

**Title:** `[AppRequest] Perrito`

**Guidelines**
- [x] I am the developer of the app.
- [ ] The app complies with the App Inclusion Policy. Leave this unticked, because of the
  AI policy; the Further Notices field explains.
- [x] The app is not already listed in the repo or issue tracker. (Check the tracker for
  "perritodeck" first; nothing was found when this was written.)
- [x] The Fastlane folder is available in the app's repo.

**Link to the source code:** `https://github.com/perritodeck/perritodeck`

**Link to app in another app store:** leave empty (F-Droid is pending; add the link if it's
listed by then).

**License used:** `Apache-2.0`

**Categories:** Connectivity, Automation, System

**Summary:**
```
Turn an old Android tablet or phone into a programmable button deck for your computer: mute the mic, switch OBS scenes, press shortcuts, all with one tap.
```

**Description:**
```
Perrito is the tablet half of PerritoDeck (https://perritodeck.dev), an open-source alternative to hardware stream decks. It shows a full-screen grid of buttons and sliders; the free PerritoDeck desktop app (Windows now, Linux from source) is where you lay them out and where their actions run.

- Pair once with a QR code or a six-digit code; TLS pinned to that computer; Wi-Fi (found by mDNS) or a USB cable
- Keys drawn by the computer: titles, icons, animated GIFs and live values (time, CPU, what's playing, the OBS scene)
- Hotkeys, typed text, media and volume keys, opening apps, files and websites, running commands, multi-step keys, toggles that follow live state, sliders, pages, and profiles that switch with the app in front
- OBS Studio and Home Assistant built in on the desktop side, plus plugins in Python or Node.js
- Runs on Android 5.1 and newer, including Amazon Fire tablets
- No account, no cloud, no ads, no analytics: the app talks only to the computer it's paired with. The camera is used only to scan the pairing QR code.
```

**Build instructions:**
```
# Needs JDK 17 and the Android SDK (platform 36, build-tools 35.0.0).
git clone https://github.com/perritodeck/perritodeck.git
cd perritodeck
git checkout v0.1.0
cd android
./gradlew assembleRelease
# Unsigned APK: android/app/build/outputs/apk/release/app-release-unsigned.apk
# (the release build signs only when PERRITO_KEYSTORE is set).
```

**AI Tools Usage**
- Assistance Level: `Dominant – Most code or content was "AI"-generated`
- "AI" Tool(s): `Claude (Claude Code)`
- What did the tools help with, and how?
  ```
  Claude Code wrote most of the code, tests and documentation under the maintainer's direction: 316 of the 321 commits up to v0.1.0 (42 of the 44 touching android/) carry a Co-Authored-By: Claude trailer. The maintainer chose the design, reviewed the changes and tested on devices (an Amazon Fire 7, Android emulators in CI).
  ```
  Check the last sentence against what actually happened, and edit it, before sending.
- AI Accountability: tick each box only if it's true.

**Further Notices:**
```
I'm the maintainer. I know your AI policy and I'm not hiding anything: the AI section above is accurate, and the commit history shows it. Please decide on that basis. If it rules the app out, no hard feelings, and thanks for reading this far.

Other notes:
- The APK is attached to each GitHub release as perrito-<version>.apk (v0.1.0: https://github.com/perritodeck/perritodeck/releases/download/v0.1.0/perrito-0.1.0.apk). Releases are marked "pre-release" on GitHub while the project is pre-alpha; please include pre-releases.
- Signing certificate SHA-256: 848c5d3f31f3bef31ecdaf308d4fa1db6a2c0a78b63ce6dacb3d6bacc98f24e5
- The desktop app (not this APK) has optional MCP support for AI assistants, and one of its plugins talks to a local Ollama. Perrito itself only talks to the paired computer.
- Reproducible builds: v0.1.0's ART baseline profile (assets/dexopt/baseline.prof) varied between builds because of an R8 race; everything else, classes.dex included, matched. From v0.1.1 (AGP 8.11.2, whose R8 has the fix) independent builds match ours byte for byte.
- Anti-features: none known. No trackers, ads, analytics, proprietary libraries or self-updater; CAMERA is only for the pairing QR code.
```
