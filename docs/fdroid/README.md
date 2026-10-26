# F-Droid submission kit

Everything needed to list **Perrito** (`dev.perritodeck.client`) on F-Droid.
Nothing here has been submitted. The repository is public and `v0.1.0` is released (2026-10-25). F-Droid is submitted with `v0.1.1`, the first release that builds reproducibly, so that F-Droid publishes our signed APK (D11, settled 2026-10-25): the recipe is a draft for that tag. The submission is ready to send once `v0.1.1` is out: [`SUBMIT.md`](SUBMIT.md).

**Not IzzyOnDroid** (decided 2026-10-25, [`DECISIONS.md`](../DECISIONS.md)). Its inclusion policy says apps made fully or partly with generative AI will most likely be rejected, and PerritoDeck is built with heavy Claude assistance. [`IZZY-REQUEST.md`](IZZY-REQUEST.md) is the request that was prepared; it wasn't sent.

| File | What it is |
|---|---|
| [`dev.perritodeck.client.yml`](dev.perritodeck.client.yml) | Build recipe for `fdroiddata` (`metadata/dev.perritodeck.client.yml`), for `v0.1.1` (commit `155b58fb77cc8d9e0a7d9536f2b857052c7f57c2`) with `Binaries:` and `AllowedAPKSigningKeys:`, ready to submit ([SUBMIT.md](SUBMIT.md)) |
| [`SUBMIT.md`](SUBMIT.md) | F-Droid: what to do at the `v0.1.1` tag, the merge request steps and text, and the builds and comparisons that show F-Droid's build matches ours |
| [`IZZY-REQUEST.md`](IZZY-REQUEST.md) | IzzyOnDroid: the request prepared on 2026-10-25 and not sent (see the note above) |
| [`../../fastlane/metadata/android/en-US/`](../../fastlane/metadata/android/en-US/) | Store text, icon, screenshots and changelogs. F-Droid reads this layout straight from the repository (so would Play, which isn't used for now) |

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

1. Make the repository public and push the release tag (maintainer). Done 2026-10-24 and 2026-10-25 for `v0.1.0`; F-Droid gets `v0.1.1`, with `perrito-0.1.1.apk` attached to its GitHub release by the Release workflow.
2. Fork `gitlab.com/fdroid/fdroiddata` and copy the recipe, unchanged, to `metadata/dev.perritodeck.client.yml` (the full steps and the merge request text are in [`SUBMIT.md`](SUBMIT.md)). The recipe is exactly what `fdroid rewritemeta` writes, so it has no comments: fdroiddata's CI runs `fdroid rewritemeta` on the merge request and fails it if the file changes, and rewritemeta drops comments. `commit:` is the tag's full commit hash, as reviewers want: in the draft it reads `v0.1.1` until the tag exists, and is replaced then ([SUBMIT.md](SUBMIT.md), step 1). Then in that checkout:
   ```sh
   fdroid readmeta && fdroid rewritemeta dev.perritodeck.client && fdroid lint dev.perritodeck.client
   fdroid build -v -l dev.perritodeck.client   # needs the Android SDK; or let the MR's CI do it
   ```
3. Open the merge request with the "New app" template. Reviews take from a few days to several weeks.
4. Later releases need nothing from us: `UpdateCheckMode: Tags` picks up new `vX.Y.Z` tags (the regex skips `-rc` pre-releases), and `AutoUpdateMode: Version` adds the build entry. With `Binaries:`, F-Droid downloads `perrito-X.Y.Z.apk` from each tag's release and publishes it only if its own build matches; a version whose build doesn't match isn't published at all (not even signed by F-Droid), so every release must keep building reproducibly. The Android workflow's Reproducible release build job checks that on every change to `android/` ([Reproducible builds](#reproducible-builds)).

**Checked on 2026-10-24** in WSL Ubuntu, with fdroidserver 2.4.5 (PyPI) and git master (`2d1e622`, with Debian trixie's ruamel.yaml 0.18.10, as fdroiddata's CI runs it):
- The draft as it is here passes `fdroid lint`, `fdroid lint -f`, fdroiddata CI's rewritemeta check (`fdroid rewritemeta`, then `git diff --exit-code`), yamllint with fdroiddata's `.yamllint` and fdroiddata's JSON schema, with no warnings.
- `fdroid build -v -l` of a test copy (`Repo:` a local clone, `commit:` a commit of `main`) builds the APK, and `fdroid scanner` finds nothing in it. Before that, two things stopped the build, both fixed now:
  - The scanner stopped it at `plugins/sdk-node/package.json`, which had no lockfile (see the checklist above).
  - Gradle couldn't find a JDK 17 for `kotlin { jvmToolchain(17) }`. F-Droid's build server and fdroiddata's CI run Debian trixie, whose default JDK is 21 and which has no `openjdk-17` package, and the build server turns off toolchain downloads. The `sudo:` lines install bookworm's OpenJDK 17 and make it the default `java`, a common pattern in fdroiddata. They keep F-Droid on JDK 17, like CI, which matters for reproducible builds. `fdroid build` skips `sudo:` unless it runs on a build server, so locally JDK 17 has to be the `java` on the PATH.
- `subdir: android/app` works with `settings.gradle.kts` one level up. Gradle 8.14.3 is in `gradlew-fdroid`'s table, and `fdroid build` uses it in place of the wrapper, which the scanner removes.
- Not run: the build on F-Droid's own build server, where the `sudo:` lines run. The merge request's CI runs it.
- The lockfile has to be in the tagged commit: the build of `v0.1.0` fails without it.

**Checked on 2026-10-25**, the `v0.1.1` draft with `Binaries:` and `AllowedAPKSigningKeys:`, with the same two fdroidserver versions and fdroiddata at `0a6edd29`: `fdroid readmeta`, `lint -v`, `lint -f`, the rewritemeta check (no change), yamllint and the JSON schema are all clean. rewritemeta keeps the key unquoted (it starts with digits but isn't a number). A test copy of the draft (`Repo:` a local clone of the branch, `commit:` its hash, version 0.1.0/1 as that commit builds, `Binaries:` left out because there is no `v0.1.1` release to download) builds with both versions and gives the same APK, byte for byte, as our own clean builds ([Reproducible builds](#reproducible-builds)).

## Reproducible builds

Making the builds reproducible lets F-Droid ship **our** signed APK after checking it matches its own build from source. F-Droid users and GitHub/Obtainium users can then update across channels, and the release key stays the single identity of the app. Since 2026-10-25 (D11 settled) the release build is reproducible on Linux, and F-Droid is submitted with `v0.1.1` and `Binaries:`.

**How it's checked.** `scripts/compare_apks.py a.apk b.apk` compares two APKs entry by entry, leaving out the signature (so one of them may be our signed release): names, order, compression, contents and the stored bytes. For two unsigned builds it also says whether the files are byte for byte the same. When `assets/dexopt/baseline.prof` differs, it decodes both profiles and names the classes and methods only one of them has, from `classes.dex`; when the dex itself differs, it says to look there. F-Droid's own check is stricter: it copies our signature onto its unsigned build and verifies it (`fdroidserver.common.verify_apks`, or `apksigcopier compare`), so the zip layout has to match too. Build what you compare from a clean checkout with the build cache off, on Linux (WSL on Windows):
```sh
cd android && ./gradlew --no-daemon --no-build-cache clean assembleRelease
python3 ../scripts/compare_apks.py app/build/outputs/apk/release/app-release-unsigned.apk perrito-X.Y.Z.apk
```
The Android workflow's **Reproducible release build** job does this on every change to `android/`: it builds the release APK twice, in two folders, with the build cache off, and fails if they differ (the two APKs are then a workflow artifact). A difference that comes and goes, as the profile race below did, fails some runs and not others, so a failure there is never noise. The Release workflow builds with `--no-build-cache`, so the published APK is built from scratch, like F-Droid's.

**Measured on 2026-10-25** (`fix/reproducible-apk`, AGP 8.11.2, R8 8.11.26):
- WSL Ubuntu, Temurin 17.0.20, nine clean builds in fresh clones at nine different paths, each with an empty Gradle build cache: all byte-identical, 2,209,025 bytes, SHA-256 `8a3d06dcf34c051bcd9975718985baf2e534013abc0ef762dfe71c5ae6052654`. The same commit's parent with AGP 8.11.1 (R8 8.11.18), six builds the same way: three different APKs with the same `classes.dex`.
- `fdroid build -v -l` of a test copy of the `v0.1.1` draft against a local clone of the branch, with fdroidserver 2.4.5 and with git master: both give that same APK, byte for byte, and `fdroid scanner` finds nothing.
- A release build signed by AGP with a throwaway key, as the Release workflow signs with ours: `verify_apks` copies its signature onto F-Droid's unsigned build and it verifies, which is exactly what F-Droid does before publishing our APK.
- GitHub's Ubuntu runner (Temurin 17), the new CI job on pull request #7 (run `40252576111`): both of its builds were that same APK, `8a3d06dc…`. So CI, WSL on this PC and `fdroid build` agree; F-Droid's build server (Debian's OpenJDK 17) is the one left, and the merge request's pipeline runs it.
- Windows (Zulu 17), two fresh clones on two drives: identical to each other (2,209,029 bytes, `fd0637d2…`), and the same as the Linux builds except for two 5-byte files, see the last risk below.

What made builds differ, and what keeps them the same:
- **The baseline profile: fixed by AGP 8.11.2.** `assets/dexopt/baseline.prof` and `baseline.profm` are the ART startup profile, built from the profiles Compose and other libraries ship and rewritten by R8 for the minified code. AGP 8.11.1 bundled R8 8.11.18, whose horizontal class merging adds a merged class's new constructor to the profile only if the first caller that creates it is in the profile, and several threads race for that (b/434861604, "Fix race in horizontal merging leading to nondeterministic art profile", fixed in R8 8.11.21). So `r8_art_profile/release/minifyReleaseWithR8/baseline-prof.txt` had 8,738, 8,740 or 8,742 lines from one tree, missing `Lq3/A;-><init>(LU2/h;ZI)V`, `LK2/p;-><init>(IB)V` or both, and F-Droid's build of `v0.1.0` matched our published APK in 431 of 433 entries, all but these two. AGP 8.11.2 bundles R8 8.11.26: every build now writes the full 8,742 lines. The upgrade changes nothing else in the code: against the AGP 8.11.1 build with the full profile, `classes.dex` differs only in the R8 version in its marker string (and so in its checksum and signature fields), and the profiles name the same 1,871 classes and 6,854 hot methods. Keep AGP at 8.11.2 or newer (R8 9.x has another nondeterminism bug, b/554586985, so check a 9.x upgrade with the job above). `baseline.prof` also records the checksum of `classes.dex`, so it differs whenever the dex does; `compare_apks.py` says when that's the case. Dropping the profile was the alternative: on an API 34 emulator, Perrito's cold start (`am start-activity -W`, 45 launches each way) took 585 ms at the median without it and 514 ms with it, 14% longer, and more on slow tablets.
- **`META-INF/version-control-info.textproto`**: AGP 8.3+ writes the git commit into release APKs, so a build from a source tarball wouldn't match one from git. Release builds leave it out (`vcsInfo { include = false }` in `android/app/build.gradle.kts`).
- **Native-lib stripping depends on the build machine.** The APK carries 12 prebuilt `.so` files (`libandroidx.graphics.path` and two CameraX libraries, for four ABIs each). AGP strips their debug symbols only if a matching NDK is installed: GitHub's Ubuntu runners have NDKs, F-Droid's build server has none unless asked, and a local build may have either. Release builds keep them exactly as they come (`keepDebugSymbols` for `**/*.so` in `android/app/build.gradle.kts`).
- **Incremental Kotlin compiles can give a different `classes.dex`.** When only some files are recompiled, the Compose compiler can infer an argument's stability differently and pass different `$changed` flags (on 2026-10-15, six classes, all in files edited since the last full compile). A fresh checkout isn't enough to avoid it: `android/gradle.properties` turns on Gradle's build cache, and the user-level cache (`~/.gradle/caches/build-cache-1`) can hold `compileReleaseKotlin` output from an incremental compile of the same sources. Compare builds made with the build cache off: `clean assembleRelease --no-build-cache`.
- **JDK and build-tools versions** must match: JDK 17 (Temurin in CI, OpenJDK on F-Droid), and build-tools 35.0.0, pinned with `buildToolsVersion`. Zulu 17.0.0 on Windows and Temurin 17.0.20 in WSL gave the same `classes.dex`.
- **Windows builds differ in two files: build on Linux to compare.** R8 writes the `META-INF/services` files it rewrites (two of them, for kotlinx.coroutines) with the line separator of the JVM it runs in, so a Windows build has `r3.b\r\n` where Linux has `r3.b\n` (R8's `StringUtils.LINE_SEPARATOR`, read from `line.separator`). It can't be set for the build: R8 runs inside the Gradle daemon, and Gradle ignores `-Dline.separator` in `org.gradle.jvmargs` (checked: other `-D` properties from there arrive, that one doesn't). CI, the Release workflow and F-Droid all build on Linux, so this only matters for comparing by hand; use WSL. Git's line endings don't matter: `.gitattributes` checks everything out with LF.
- The `minSdk 22` APK is signed with v1 + v2 signatures (set explicitly in the signing config; v3 is off, D7). `verify_apks` and `apksigcopier` handle both.

The two fields for stage 2 are in the draft recipe where `fdroid rewritemeta` puts them: `Binaries:` on the line after `Repo:`, and `AllowedAPKSigningKeys:` after the `Builds:` block, with a blank line before it. The key is the release certificate's SHA-256, lowercase and without colons, as the Release workflow prints it in the Android job summary and the release notes (or `keytool -printcert -jarfile perrito-<version>.apk`):
```yaml
Repo: https://github.com/perritodeck/perritodeck.git
Binaries: https://github.com/perritodeck/perritodeck/releases/download/v%v/perrito-%v.apk

# ... Builds: ...

AllowedAPKSigningKeys: 848c5d3f31f3bef31ecdaf308d4fa1db6a2c0a78b63ce6dacb3d6bacc98f24e5
```

## Google Play
Not for now (D8 in `docs/plan/phase-8.md`, decided 2026-10-17): Android users get the APK from GitHub Releases, and F-Droid comes later. The notes for later are in [`../distribution.md`](../distribution.md#google-play): the 12-tester closed test, the data safety answers, and Play App Signing as D7 settles it (our release key as the upload key). The Release workflow still keeps `perrito-<version>.aab` as the `play-bundle` workflow artifact.
