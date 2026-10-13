# F-Droid and IzzyOnDroid submission kit

Everything needed to list **Perrito** (`dev.perritodeck.client`) on F-Droid and IzzyOnDroid.
Nothing here has been submitted. Both stores need the repository to be **public** and a release tag. The draft recipe assumes `v0.1.0`; if the first release is `v0.2.0` instead, update its version, versionCode and commit (see `docs/next-steps.md` item 13).

| File | What it is |
|---|---|
| [`dev.perritodeck.client.yml`](dev.perritodeck.client.yml) | Draft build recipe for `fdroiddata` (`metadata/dev.perritodeck.client.yml`) |
| [`../../fastlane/metadata/android/en-US/`](../../fastlane/metadata/android/en-US/) | Store text, icon, screenshots and changelogs. F-Droid, IzzyOnDroid and Play all read this layout straight from the repository |

## Readiness checklist

| Requirement | Status |
|---|---|
| FOSS licence (Apache-2.0), no proprietary dependencies, no Play Services, no analytics | ✔ (`android/app/build.gradle.kts`, `gradle/libs.versions.toml`) |
| Only Google Maven, Maven Central and the Gradle Plugin Portal as repositories | ✔ (`android/settings.gradle.kts`) |
| No prebuilt binaries in the tree except the official Gradle wrapper jar (F-Droid allows it) and a font | ✔ (checked with `git ls-files`) |
| `dependenciesInfo` block disabled (F-Droid flags the encrypted blob) | ✔ |
| Release build unsigned when no key is present (F-Droid signs itself) | ✔ (signing only when `PERRITO_KEYSTORE` is set) |
| `title.txt` (≤ 50; Play ≤ 30) | ✔ 28 characters |
| `short_description.txt` (≤ 80) | ✔ 73 characters |
| `full_description.txt` (≤ 4000) | ✔ about 1400 characters |
| `changelogs/<versionCode>.txt` (≤ 500) for every release | ✔ `1.txt`; `scripts/release-version.py --check` fails a release without one |
| `images/icon.png` 512×512 | ✔ the final paw-of-keys icon (the app's own launcher icon is still the old paw; `docs/next-steps.md` item 13) |
| `images/phoneScreenshots/`, `images/tenInchScreenshots/` | ✔ two phone screenshots at 1080×1920 and three ten-inch ones at 1920×1200, showing decks (streaming, homelab, desk). Play needs at least two |
| `images/featureGraphic.png` 1024×500 | ✔ (Play only) |
| Anti-features | None expected. `CAMERA` is used only to scan a pairing QR code |

### Screenshot shot list
For replacing or adding screenshots later. PNG or JPEG, no device frames, no text overlays that name other brands. Name them `1.png`, `2.png` and so on, in display order.

`phoneScreenshots/` (phone in portrait, e.g. the API 34 emulator):
1. A paired deck with a 5×3 grid of coloured keys (mic, scenes, media)
2. The pairing screen with the 6-digit code entry
3. A key being pressed, showing the ✓ feedback
4. The host list found on Wi-Fi

`tenInchScreenshots/` (landscape, the Fire HD 10 emulator or a real 10" tablet):
1. A full 8×5 deck: streaming page (scenes, mic, clip, BRB)
2. A smart-home page with live values (temperatures, lights on/off)
3. A page with sliders (volume) and a live clock/CPU key
4. The Fire 7 at 1024×600 (shows that old tablets work)

The E2E runs upload their screenshots as workflow artifacts (`e2e-<run>` on each run's summary page). They work as a starting point, but they show test grids, so take real ones from a nice-looking demo profile.

## F-Droid (main repository)

1. Make the repository public and push the release tag (maintainer).
2. Fork `gitlab.com/fdroid/fdroiddata`, copy the draft to `metadata/dev.perritodeck.client.yml`, then in that checkout:
   ```sh
   fdroid readmeta && fdroid rewritemeta dev.perritodeck.client && fdroid lint dev.perritodeck.client
   fdroid build -v -l dev.perritodeck.client   # needs the Android SDK; or let the MR's CI do it
   ```
3. Open the merge request with the "New app" template. Reviews take from a few days to several weeks.
4. Later releases need nothing from us: `UpdateCheckMode: Tags` picks up new `vX.Y.Z` tags (the regex skips `-rc` pre-releases), and `AutoUpdateMode: Version` adds the build entry.

**Unverified (fdroidserver hasn't been run on the draft yet):**
- `fdroid lint` and `fdroid build` have not been run on the draft.
- The F-Droid build server uses Debian's JDK 17 by default. AGP 8.11 needs 17, so no `sudo:` block is included. If the build reports the wrong Java version, add `sudo: [apt-get update, apt-get install -y openjdk-17-jdk-headless, update-alternatives --auto java]`.
- The draft assumes `fdroid` handles `subdir: android/app` with `settings.gradle.kts` one level up and the `../protocol/fixtures` path (it's used only by unit tests, which F-Droid doesn't run).
- Gradle 8.14.3 (from `gradle-wrapper.properties`) must be in fdroidserver's `gradlew-fdroid` list. Recent fdroidserver versions include it.

## Reproducible builds

Making the builds reproducible lets F-Droid ship **our** signed APK after checking it matches its own build from source. F-Droid users and GitHub/Obtainium users can then update across channels, and the release key stays the single identity of the app. The steps:

1. Set up release signing (see [`../releasing.md`](../releasing.md)), then publish the first release with `perrito-<version>.apk` attached.
2. Build the same tag twice in clean checkouts (CI and locally on Linux) and compare: `diffoscope a.apk b.apk`, or `apksigcopier compare`. The reference build is the one CI makes. Windows builds can differ because of line endings (`core.autocrlf`).
3. If they match, uncomment `Binaries:` and `AllowedAPKSigningKeys:` in the draft. The key is the lowercase SHA-256 of the certificate with no colons. The Release workflow prints it in the Android job summary and the release notes.

Known risks to check when doing step 2:
- **`META-INF/version-control-info.textproto`**: AGP 8.3+ writes the git commit into release APKs, so a build from a source tarball wouldn't match one from git. Release builds now leave it out (`vcsInfo { include = false }` in `android/app/build.gradle.kts`); check that the file is absent when comparing.
- **Native-lib stripping depends on the build machine.** The APK carries 12 prebuilt `.so` files (`libandroidx.graphics.path` and two CameraX libraries, for four ABIs each). AGP strips their debug symbols only if a matching NDK is installed: GitHub's Ubuntu runners have NDKs, F-Droid's build server has none unless asked, and a local build may have either. So the same commit could give different `.so` files. Release builds now keep them exactly as they come (`keepDebugSymbols` for `**/*.so` in `android/app/build.gradle.kts`); check that the `.so` files match when comparing.
- **Baseline profiles** (`assets/dexopt/baseline.prof`, which Compose libraries ship): older AGP versions wrote them non-deterministically. AGP 8.11 should be fine. Compare them if the APKs differ.
- **JDK and build-tools versions** must match: JDK 17 (Temurin in CI, OpenJDK on F-Droid), and build-tools 35.0.0, now pinned with `buildToolsVersion`. R8 output can change between JDK vendors only in rare cases.
- **No comparison has been run yet.** The minified release build itself works: on 2026-10-13 it was built with a throwaway key and run on the Fire 7 (code pairing, a key press, the scan screen), and `android.yml` builds it unsigned in every Android CI run. An API 22 emulator, QR decoding, USB, the deck and sliders are still to try by hand (`docs/next-steps.md` item 13).
- The `minSdk 22` APK is signed with v1 + v2 signatures (set explicitly in the signing config; v3 is off). `apksigcopier` handles both.

## IzzyOnDroid

Faster than F-Droid (days) and serves the APKs from our GitHub Releases as they are.

- Needs: a public repository, a **signed** `perrito-<version>.apk` attached to each GitHub Release, and the fastlane metadata in the repository. The metadata is there, and the Release workflow publishes only a signed APK, so what's left is the release key (`docs/next-steps.md` item 11) and going public.
- Its scanner rejects trackers, proprietary libraries and the dependency-info blob. None of these are present.
- Size limit is about 30 MB per APK. The minified release APK is about 2.1 MB (2,164,101 bytes signed, 2026-10-13), and `android.yml` reports its size in every run and warns above 30 MB.
- IzzyOnDroid now also verifies reproducible builds and shows a badge for them. The same reproducibility work as above applies.
- Request it by opening an issue at `gitlab.com/IzzyOnDroid/repo` with the "inclusion request" template. Give the repository URL and confirm that the APK is attached to releases.
- After inclusion, new GitHub Releases are picked up automatically, with no action needed per release.

## Google Play
See [`../distribution.md`](../distribution.md#google-play): the 12-tester closed test, data safety answers and Play App Signing. The Release workflow keeps `perrito-<version>.aab` as the `play-bundle` workflow artifact for upload.
