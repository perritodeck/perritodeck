# F-Droid and IzzyOnDroid submission kit

Everything needed to list **Perrito** (`dev.perritodeck.client`) on F-Droid and IzzyOnDroid.
Nothing here has been submitted. Both stores need the repository to be **public** and a release tag. The draft recipe builds `v0.1.0`, which is what the first release will be (decision D5 in `docs/plan/phase-8.md`; see `docs/next-steps.md` item 13).

| File | What it is |
|---|---|
| [`dev.perritodeck.client.yml`](dev.perritodeck.client.yml) | Draft build recipe for `fdroiddata` (`metadata/dev.perritodeck.client.yml`) |
| [`../../fastlane/metadata/android/en-US/`](../../fastlane/metadata/android/en-US/) | Store text, icon, screenshots and changelogs. F-Droid and IzzyOnDroid read this layout straight from the repository (so would Play, which isn't used for now) |

## Readiness checklist

| Requirement | Status |
|---|---|
| FOSS licence (Apache-2.0), no proprietary dependencies, no Play Services, no analytics | ✔ (`android/app/build.gradle.kts`, `gradle/libs.versions.toml`) |
| Only Google Maven, Maven Central and the Gradle Plugin Portal as repositories | ✔ (`android/settings.gradle.kts`) |
| No prebuilt binaries in the tree except the official Gradle wrapper jar (F-Droid allows it) and a font | ✔ (checked with `git ls-files`; F-Droid's scanner removes the wrapper jar and finds nothing else) |
| Every `package.json`, `Cargo.toml` and `pubspec.yaml` in the tree has a lockfile next to it or above it | ✔ F-Droid's scanner scans the whole repository, not just `android/`, and stops the build on a dependency file without a lock. `plugins/sdk-node/package-lock.json` exists for this alone (the SDK has no dependencies) |
| `dependenciesInfo` block disabled (F-Droid flags the encrypted blob) | ✔ |
| Release build unsigned when no key is present (F-Droid signs itself) | ✔ (signing only when `PERRITO_KEYSTORE` is set) |
| `title.txt` (≤ 50; Play ≤ 30) | ✔ 28 characters |
| `short_description.txt` (≤ 80) | ✔ 76 characters |
| `full_description.txt` (≤ 4000) | ✔ 2,977 characters, counted as decoded text with the `<b>` tags and without the final newline (3,020 bytes in UTF-8) |
| `changelogs/<versionCode>.txt` (≤ 500) for every release | ✔ `1.txt`; `scripts/release-version.py --check` fails a release without one |
| `images/icon.png` 512×512 | ✔ the final paw-of-keys icon, the same as the app's launcher icon |
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

The E2E runs upload their screenshots as workflow artifacts (`e2e-<run>`, on the CI run's summary page for a push or pull request, on the E2E run's for a weekly or manual one). They work as a starting point, but they show test grids, so take real ones from a nice-looking demo profile.

## F-Droid (main repository)

1. Make the repository public and push the release tag (maintainer).
2. Fork `gitlab.com/fdroid/fdroiddata` and copy the draft, unchanged, to `metadata/dev.perritodeck.client.yml`. The draft is exactly what `fdroid rewritemeta` writes, so it has no comments: fdroiddata's CI runs `fdroid rewritemeta` on the merge request and fails it if the file changes, and rewritemeta drops comments. Reviewers usually swap `commit: v0.1.0` for the tag's full commit hash. Then in that checkout:
   ```sh
   fdroid readmeta && fdroid rewritemeta dev.perritodeck.client && fdroid lint dev.perritodeck.client
   fdroid build -v -l dev.perritodeck.client   # needs the Android SDK; or let the MR's CI do it
   ```
3. Open the merge request with the "New app" template. Reviews take from a few days to several weeks.
4. Later releases need nothing from us: `UpdateCheckMode: Tags` picks up new `vX.Y.Z` tags (the regex skips `-rc` pre-releases), and `AutoUpdateMode: Version` adds the build entry.

**Checked on 2026-10-24** in WSL Ubuntu, with fdroidserver 2.4.5 (PyPI) and git master (`2d1e622`, with Debian trixie's ruamel.yaml 0.18.10, as fdroiddata's CI runs it):
- The draft as it is here passes `fdroid lint`, `fdroid lint -f`, fdroiddata CI's rewritemeta check (`fdroid rewritemeta`, then `git diff --exit-code`), yamllint with fdroiddata's `.yamllint` and fdroiddata's JSON schema, with no warnings.
- `fdroid build -v -l` of a test copy (`Repo:` a local clone, `commit:` a commit of `main`) builds the APK, and `fdroid scanner` finds nothing in it. Before that, two things stopped the build, both fixed now:
  - The scanner stopped it at `plugins/sdk-node/package.json`, which had no lockfile (see the checklist above).
  - Gradle couldn't find a JDK 17 for `kotlin { jvmToolchain(17) }`. F-Droid's build server and fdroiddata's CI run Debian trixie, whose default JDK is 21 and which has no `openjdk-17` package, and the build server turns off toolchain downloads. The `sudo:` lines install bookworm's OpenJDK 17 and make it the default `java`, a common pattern in fdroiddata. They keep F-Droid on JDK 17, like CI, which matters for reproducible builds. `fdroid build` skips `sudo:` unless it runs on a build server, so locally JDK 17 has to be the `java` on the PATH.
- `subdir: android/app` works with `settings.gradle.kts` one level up. Gradle 8.14.3 is in `gradlew-fdroid`'s table, and `fdroid build` uses it in place of the wrapper, which the scanner removes.
- Not run: the build on F-Droid's own build server, where the `sudo:` lines run. The merge request's CI runs it.
- The lockfile has to be in the tagged commit: the build of `v0.1.0` fails without it.

## Reproducible builds

Making the builds reproducible lets F-Droid ship **our** signed APK after checking it matches its own build from source. F-Droid users and GitHub/Obtainium users can then update across channels, and the release key stays the single identity of the app. The steps:

1. Set up release signing (see [`../releasing.md`](../releasing.md)), then publish the first release with `perrito-<version>.apk` attached.
2. Build the same tag twice in clean checkouts (CI and locally on Linux, locally with `--no-build-cache`; see the incremental-compile risk below) and compare: `diffoscope a.apk b.apk`, or `apksigcopier compare`. The reference build is the one CI makes. Windows builds can differ because of line endings (`core.autocrlf`).
3. If they match, add two fields to the draft, where `fdroid rewritemeta` puts them: `Binaries:` on the line after `Repo:`, and `AllowedAPKSigningKeys:` after the `Builds:` block, with a blank line before it. F-Droid then builds from source, compares with our signed APK and, if they match, publishes ours.
   ```yaml
   Repo: https://github.com/perritodeck/perritodeck.git
   Binaries: https://github.com/perritodeck/perritodeck/releases/download/v%v/perrito-%v.apk

   # ... Builds: unchanged ...

   AllowedAPKSigningKeys: <lowercase SHA-256 of the release certificate, no colons>
   ```
   The Release workflow prints the key in the Android job summary and the release notes (or `keytool -printcert -jarfile perrito-<version>.apk`, without the colons). Run `fdroid rewritemeta` and `fdroid lint` again afterwards, and keep what rewritemeta writes (it puts the key in quotes if YAML would read it as a number). This placement was checked with both fdroidserver versions above on 2026-10-24.

Known risks to check when doing step 2:
- **`META-INF/version-control-info.textproto`**: AGP 8.3+ writes the git commit into release APKs, so a build from a source tarball wouldn't match one from git. Release builds now leave it out (`vcsInfo { include = false }` in `android/app/build.gradle.kts`); check that the file is absent when comparing.
- **Native-lib stripping depends on the build machine.** The APK carries 12 prebuilt `.so` files (`libandroidx.graphics.path` and two CameraX libraries, for four ABIs each). AGP strips their debug symbols only if a matching NDK is installed: GitHub's Ubuntu runners have NDKs, F-Droid's build server has none unless asked, and a local build may have either. So the same commit could give different `.so` files. Release builds now keep them exactly as they come (`keepDebugSymbols` for `**/*.so` in `android/app/build.gradle.kts`); check that the `.so` files match when comparing.
- **Baseline profiles are not deterministic: this blocks step 3.** (`assets/dexopt/baseline.prof` and `baseline.profm`, built from the profiles Compose and other libraries ship.) On 2026-10-15 three release builds on Windows gave byte-identical APKs, but on 2026-10-24 nine clean builds of the same commit in WSL (Temurin 17, an empty build cache each time; five with `./gradlew clean assembleRelease --no-build-cache`, three by `fdroid build` and one more in `fdroid build`'s checkout) gave three different APKs. `classes.dex` and every other entry were identical each time; only the two profile files differed, and the same tree gave different results run after run. The cause is R8's rewriting of the profile (`r8_art_profile/release/minifyReleaseWithR8/baseline-prof.txt` had 8,736, 8,738 or 8,740 lines): it sometimes leaves out the constructors of classes that R8's horizontal class merging made (`kotlinx.coroutines.DeferredCoroutine`, a merged lambda class in `MainActivityKt`). D11 kept the profile because the builds matched; this needs deciding again before `Binaries:` is added. The options: turn off R8's profile rewriting (through `android { experimentalProperties[...] }`; D11 measured it as keeping builds identical but leaving most rules naming classes that no longer exist), leave the profile out of release APKs, or try a newer R8. `baseline.prof` also records the checksum of `classes.dex`, so it differs whenever the dex does.
- **Incremental Kotlin compiles can give a different `classes.dex`.** When only some files are recompiled, the Compose compiler can infer an argument's stability differently and pass different `$changed` flags (on 2026-10-15, six classes, all in files edited since the last full compile). A fresh checkout isn't enough to avoid it: `android/gradle.properties` turns on Gradle's build cache, and the user-level cache (`~/.gradle/caches/build-cache-1`) can hold `compileReleaseKotlin` output from an incremental compile of the same sources. That day a fresh copy of the tree took `compileReleaseKotlin` from the cache and gave a different APK from the same copy built with the cache off. Compare builds made with the build cache off: `clean assembleRelease --no-build-cache`. CI isn't affected, because its cache entries come from full compiles on fresh runners.
- **JDK and build-tools versions** must match: JDK 17 (Temurin in CI, OpenJDK on F-Droid), and build-tools 35.0.0, now pinned with `buildToolsVersion`. R8 output can change between JDK vendors only in rare cases.
- **Builds by `fdroid build` and our own build give the same `classes.dex`, resources and native libraries** (2026-10-24, WSL Ubuntu on the same PC, Temurin 17 for both, the same commit): only the baseline-profile files differ, as above. Not compared yet: F-Droid's build server (Debian OpenJDK 17) with CI (Temurin 17 on GitHub's runners), and builds on different machines. The minified release build itself works: on 2026-10-13 it was built with a throwaway key and run on the Fire 7 (code pairing, a key press, the scan screen), and `android.yml` builds it unsigned in every Android CI run. An API 22 emulator, QR decoding, USB, the deck and sliders are still to try by hand (`docs/next-steps.md` item 13).
- The `minSdk 22` APK is signed with v1 + v2 signatures (set explicitly in the signing config; v3 is off, D7). `apksigcopier` handles both.

## IzzyOnDroid

Faster than F-Droid (days) and serves the APKs from our GitHub Releases as they are.

- Needs: a public repository, a **signed** `perrito-<version>.apk` attached to each GitHub Release, and the fastlane metadata in the repository. The metadata is there, and the Release workflow publishes only a signed APK, so what's left is the release key (`docs/next-steps.md` item 11) and going public.
- Its scanner rejects trackers, proprietary libraries and the dependency-info blob. None of these are present.
- Size limit is about 30 MB per APK. The minified release APK is about 2.2 MB (2,179,425 bytes unsigned, 2026-10-15; the signed size needs measuring again once the release key exists), and `android.yml` reports its size in every run and warns above 30 MB.
- IzzyOnDroid now also verifies reproducible builds and shows a badge for them. The same reproducibility work as above applies.
- Request it by opening an issue at `gitlab.com/IzzyOnDroid/repo` with the "inclusion request" template. Give the repository URL and confirm that the APK is attached to releases.
- After inclusion, new GitHub Releases are picked up automatically, with no action needed per release.

## Google Play
Not for now (D8 in `docs/plan/phase-8.md`, decided 2026-10-17): v0.1.0's Android app ships on GitHub Releases, then IzzyOnDroid and F-Droid. The notes for later are in [`../distribution.md`](../distribution.md#google-play): the 12-tester closed test, the data safety answers, and Play App Signing as D7 settles it (our release key as the upload key). The Release workflow still keeps `perrito-<version>.aab` as the `play-bundle` workflow artifact.
