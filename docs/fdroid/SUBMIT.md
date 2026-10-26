# Submitting Perrito to F-Droid (v0.1.1, with our signature)

F-Droid is submitted with **v0.1.1** and `Binaries:` (maintainer decision 2026-10-25; D11 in
[`../plan/phase-8.md`](../plan/phase-8.md)): F-Droid builds the tag from source, checks that
its build matches our signed `perrito-0.1.1.apk` and publishes ours, so F-Droid, GitHub and
IzzyOnDroid users all have the same signature and can update across them. v0.1.0 isn't
submitted: its baseline profile came out differently from build to build, so F-Droid's build
can't match it ([README.md](README.md#reproducible-builds)).

Nothing has been submitted, forked or posted. The recipe is
[`dev.perritodeck.client.yml`](dev.perritodeck.client.yml), a draft for v0.1.1 until the tag
exists; step 1 below finishes it.

## Where it stands (checked 2026-10-25)

| Check | Result |
|---|---|
| Recipe | Draft for v0.1.1: `Binaries: https://github.com/perritodeck/perritodeck/releases/download/v%v/perrito-%v.apk` after `Repo:`, `AllowedAPKSigningKeys: 848c5d3f31f3bef31ecdaf308d4fa1db6a2c0a78b63ce6dacb3d6bacc98f24e5` (the release certificate's SHA-256, the same key that signed `perrito-0.1.0.apk`) after `Builds:`, `versionName: 0.1.1`, `versionCode: 2`, `CurrentVersion: 0.1.1`, `CurrentVersionCode: 2`, the `sudo:` JDK 17 lines kept. **`commit: v0.1.1` is a placeholder** for the tag's full hash, which only exists once the tag does (step 1); the recipe can't carry a comment saying so, because rewritemeta drops comments and fdroiddata's CI then fails |
| `fdroid readmeta`, `lint -v`, `lint -f`, `rewritemeta --list`; CI's rewritemeta job (`fdroid rewritemeta`, then `git diff --exit-code`); yamllint with fdroiddata's `.yamllint`; `check-jsonschema` with `schemas/metadata.json` | all clean, no change, with fdroidserver 2.4.5 (PyPI) and git master `2d1e622` with ruamel.yaml 0.18.10, fdroiddata at `0a6edd29` |
| Reproducible: our own clean release builds (AGP 8.11.2), nine in WSL in nine folders | byte-identical: 2,209,025 bytes, SHA-256 `8a3d06dcf34c051bcd9975718985baf2e534013abc0ef762dfe71c5ae6052654` |
| `fdroid build -v -l` of a test copy (`Repo:` a local clone of the branch, its commit's hash, 0.1.0/1 as that commit builds), 2.4.5 and master | the same APK, byte for byte; `fdroid scanner` finds nothing |
| The same, **with `Binaries:`** pointing at that commit's release build signed by AGP with a throwaway key (served from localhost) and `AllowedAPKSigningKeys:` that key | `fdroid build` downloads it and runs its check: "compared built binary to supplied reference binary successfully" |
| v0.1.0, for the record: `fdroid build` of the tag compared with the published `perrito-0.1.0.apk` | 431 of 433 entries identical; only `baseline.prof` and `baseline.profm` differ, which is why v0.1.0 isn't submitted |

The logs are in WSL under `~/pd/repro/logs/` (`fix.log`, `base.log`, `fdroid-branch-*.log`,
`signed-verify.log`, `lint-draft.log`) and, for v0.1.0, `~/pd/fdroid/logs/60-*` to `62-*`.

## At the v0.1.1 tag, before submitting (maintainer)

1. **Release v0.1.1** as usual ([`../releasing.md`](../releasing.md)): `scripts/release-version.py 0.1.1`
   (versionName `0.1.1`, versionCode `2`), `fastlane/metadata/android/en-US/changelogs/2.txt`, the tag,
   and the Release workflow attaching `perrito-0.1.1.apk` to the GitHub release. Then finish
   the recipe in this repository: replace `commit: v0.1.1` with the tag's full hash
   (`git rev-parse v0.1.1^{commit}`), and check that `versionName`/`versionCode` and
   `CurrentVersion`/`CurrentVersionCode` are what the tag's `build.gradle.kts` says (if the
   release ends up with another version, change all four, and the hash).
2. **Check the stage-2 match before opening the merge request.** In WSL, with fdroidserver
   and JDK 17 on the PATH, in a fdroiddata checkout with the finished recipe:
   ```sh
   fdroid readmeta && fdroid rewritemeta dev.perritodeck.client && git diff --exit-code && fdroid lint dev.perritodeck.client
   fdroid build -v -l dev.perritodeck.client
   ```
   With `Binaries:` set, `fdroid build` downloads `perrito-0.1.1.apk`, compares it with its own
   build and fails if they differ ("compared built binary to supplied reference binary
   successfully" is the line to look for). If it fails, compare the two by hand:
   `python3 scripts/compare_apks.py perrito-0.1.1.apk unsigned/dev.perritodeck.client_2.apk`
   names the entries that differ ([README.md](README.md#reproducible-builds)).

## Steps (maintainer)

After the two steps above.

1. Sign in to gitlab.com with the project's account (a handle is fine; no real name is
   needed anywhere: `AuthorName` in the recipe is "PerritoDeck contributors").
2. Fork [fdroid/fdroiddata](https://gitlab.com/fdroid/fdroiddata). Keep the fork **public**,
   and don't work on `master` (it is protected by default; a merge request from a protected
   branch fails).
3. Clone the fork shallowly (fdroiddata is large) and make a branch named after the app id:
   ```sh
   git clone --depth 1 https://gitlab.com/<your-handle>/fdroiddata.git
   cd fdroiddata
   git switch -c dev.perritodeck.client
   cp <perritodeck>/docs/fdroid/dev.perritodeck.client.yml metadata/dev.perritodeck.client.yml
   ```
   Copy the file byte for byte (LF line endings; on Windows copy it from WSL or a checkout
   with `.gitattributes` applied, never through an editor that adds CRLF or a BOM).
4. Check again in the fork's checkout, if fdroidserver is installed (`pip install git+https://gitlab.com/fdroid/fdroidserver.git`); the `fdroid build` of step 2 above is the one that matters:
   ```sh
   fdroid readmeta && fdroid rewritemeta dev.perritodeck.client && git diff --exit-code && fdroid lint dev.perritodeck.client
   ```
5. Commit and push. fdroiddata wants one commit per app (squashed) with a plain message;
   the usual form is the merge request title:
   ```sh
   git add metadata/dev.perritodeck.client.yml
   git commit -m "New app: Perrito"
   git push -u origin dev.perritodeck.client
   ```
   Commit under the project identity (`dmang-dev <dev@perritodeck.dev>`), not a personal one.
6. In the fork, open **Build → Pipelines** and wait for the pipeline. The `fdroid build` job
   is the one that runs the `sudo:` lines on F-Droid's build-server image. If GitLab asks for
   a phone number or a credit card to run CI, don't give one: open the merge request anyway
   and say so in it (the template says F-Droid will trigger CI).
7. Open a merge request from `<your-handle>/fdroiddata:dev.perritodeck.client` to
   `fdroid/fdroiddata:master`, template **Default**, title **`New app: Perrito`**, and
   replace the body with the text below. Tick the boxes left open once they're true.
8. Answer review comments in the merge request (the answers to the likely ones are below).
   Don't rebase unless there's a conflict. Reviews take from days to weeks. Later releases
   need nothing: `UpdateCheckMode: Tags` and `AutoUpdateMode: Version` pick up new `vX.Y.Z`
   tags.

## Merge request description (paste as is)

The boxes left open are statements only the person submitting can make (that they read the
guides) or that are only known after the pipeline runs.

```markdown
## Abstract for new app inclusion

### Please introduce the functions and usage of your app in summary, and also attach some screenshots.

Perrito turns an Android tablet or phone into a full-screen grid of programmable buttons and sliders for your computer: mute the mic, switch OBS scenes, press keyboard shortcuts, type text, open apps and websites, control media and volume, with one tap. It is the tablet half of PerritoDeck (https://perritodeck.dev): the free desktop app (Windows now, Linux from source) is where you lay out the buttons and where their actions run. You pair the two once with a QR code or a six-digit code; the connection is TLS pinned to that computer, over Wi-Fi (found by mDNS) or a USB cable. It runs on Android 5.1 and newer, including Amazon Fire tablets.

No account, no cloud, no analytics: the app talks only to the computer it is paired with. The camera is used only to scan the pairing QR code.

Screenshots (from the fastlane folder in the repo):
- https://raw.githubusercontent.com/perritodeck/perritodeck/v0.1.1/fastlane/metadata/android/en-US/images/phoneScreenshots/1.png
- https://raw.githubusercontent.com/perritodeck/perritodeck/v0.1.1/fastlane/metadata/android/en-US/images/phoneScreenshots/2.png
- https://raw.githubusercontent.com/perritodeck/perritodeck/v0.1.1/fastlane/metadata/android/en-US/images/tenInchScreenshots/1.png
- https://raw.githubusercontent.com/perritodeck/perritodeck/v0.1.1/fastlane/metadata/android/en-US/images/tenInchScreenshots/2.png
- https://raw.githubusercontent.com/perritodeck/perritodeck/v0.1.1/fastlane/metadata/android/en-US/images/tenInchScreenshots/3.png

### Please elaborate why you wrote this app. Please highlight the unique functions it provides over other similar apps in F-Droid.

Hardware button decks are expensive and the popular tablet alternatives are proprietary, need an account or a cloud service, or don't run on old tablets. PerritoDeck gives an old tablet a new job with a fully FOSS stack (Apache-2.0, the plugins MIT) and a documented protocol. Compared with remote-control apps in F-Droid such as KDE Connect's "Run command", Perrito is a dedicated control surface: the computer draws the keys (titles, icons, animated GIFs, live values such as CPU, the time or the OBS scene), there are sliders, toggles that follow live state, pages, and profiles that switch with the app in front; the desktop side has OBS Studio and Home Assistant built in and plugins in Python or Node.js. It works back to API 22, so Fire OS 5 tablets are supported.

## Checklist

### Policy

* [x] The app complies with the [inclusion criteria](https://f-droid.org/docs/Inclusion_Policy).
* [x] The original app author has been notified (and does not oppose the inclusion). If you are not the author, please paste the link of the reply from the author.
  I am the upstream maintainer (https://github.com/perritodeck/perritodeck).
* [x] The upstream app source code repo contains the app metadata in a [Fastlane](https://gitlab.com/snippets/1895688) or [Triple-T](https://gitlab.com/snippets/1901490) folder structure. The summary and description must be included and images, icon, and changelog should also be provided for better user experience. The `en-US` locale must be included.
  `fastlane/metadata/android/en-US/`: title, short and full description, icon, feature graphic, phone and ten-inch screenshots, `changelogs/1.txt` and `2.txt`.

### Docs

* [ ] Please read [the guide](https://gitlab.com/fdroid/fdroiddata/-/blob/master/CONTRIBUTING.md) first if this is your first contribution.
* [ ] Please make sure your metadata follows the best practice in [our templates](https://gitlab.com/fdroid/fdroiddata/tree/master/templates).
* [ ] Please read the [Build Metadata Reference](https://f-droid.org/docs/Build_Metadata_Reference/) and make sure your metadata is valid.
* [ ] Please read the [Quick Start Guide](https://f-droid.org/en/docs/Submitting_to_F-Droid_Quick_Start_Guide/).

### Merge Request Setup

* [x] The title of this merge request should follow "New app: app name" format.
* [x] Please make sure your fdroiddata fork is public and your branch is not protected. See <https://docs.gitlab.com/user/project/repository/branches/protected/>.
* [ ] Please read [our Git guide](https://gitlab.com/fdroid/wiki/-/wikis/Tips-for-fdroiddata-contributors/Git-Usage) if you don't know how to rebase your branch. Don't rebase your branch if there is no conflict.
* [x] All related [fdroiddata](https://gitlab.com/fdroid/fdroiddata/issues) and [RFP issues](https://gitlab.com/fdroid/rfp/issues) have been referenced in this merge request
  There are none.
* [x] Please only submit one app in one MR.

### Metadata

* [x] Metadata must be put in `metadata/<applicationId>.yml`.
* [x] Metadata must be a valid YAML file.
* [x] Metadata must use LF as line ending.
* [x] Don't add summary/description/changelog/images or anything that should be provided in upstream repo. Please check the Changes tab to make sure there is no other unrelated files added in the MR.
* [x] Releases are tagged and auto update is enabled unless there is a special reason.
* [x] There is an issue tracker and contact info of the author so that we can report bugs and contact the author.
* [x] An AuthorName must be added. It doesn't need to be the real name.
* [x] External repos are added as git submodules instead of srclibs. You can update git submodules without opening an MR in this repo and the submodule is covered by our scanner.
  No external repos, submodules or srclibs.
* [x] Enable [Reproducible Builds](https://f-droid.org/docs/Reproducible_Builds). We'll use your signature for improved security/reliability, also allowing users to switch between different channels. Do note that if you don't enable reproducible build then the apk will be signed with our key so you can't enable it later. If you can't enable this, please add the reasons here.
  Enabled: `Binaries` and `AllowedAPKSigningKeys` are set. Our release workflow builds the tag on Linux with JDK 17 and the build cache off, and an `fdroid build` of v0.1.1 matches our signed `perrito-0.1.1.apk` byte for byte (the comparison `fdroid build` makes passes). v0.1.0 is left out because its ART baseline profile wasn't deterministic (an R8 race, fixed in AGP 8.11.2).
* [x] Setup abi split if the APK is large and the splitted ones can be much smaller.
  Not needed: the APK is about 2.2 MB.
* [x] Only the latest versions should be kept in the metadata before it's merged. If you update the metadata, please replace the old versions with the new ones.
* [x] Don't add any disabled versions in the metadata.
* [x] The `commit` field should be the full hash. Please don't use tag or branch in commit.

### Pipeline

* [ ] All pipelines should pass.
* [ ] All warnings and errors in the Reports tab should be fixed or explained.
* [x] F-Droid CI runners are under GitLab's FOSS program, so there's no need for you to pay for any CI time. If Gitlab starts asking for phone numbers or credit cards don't submit anything, just leave a note in the MR so we know we need to trigger the CI.

---------------------

/label ~"New App"
```

## Anti-features: none

- **NonFreeNet**: no. That anti-feature is for apps that promote or depend on a non-free
  network service. Perrito connects only to the PerritoDeck host the user runs on their own
  computer, which is the same Apache-2.0 project; there is no server of ours or anyone
  else's in between. Plugins on the desktop side can talk to services such as Twitch or
  Spotify if the user installs and sets them up, but that's the desktop app, optional, and
  not in the APK.
- **Tracking / Ads / NonFreeDep / NonFreeAdd / NonFreeAssets / UpstreamNonFree**: no. No
  analytics, crash reporting, Play Services or proprietary SDKs. All 86 libraries in the APK
  (AndroidX, Compose, CameraX, OkHttp, ZXing, kotlinx; listed in its
  `assets/third-party-licenses.txt`) are Apache-2.0, two of them with BSD-3-Clause (libyuv)
  or MPL-2.0 (the Public Suffix List data) parts. The fdroid scanner finds nothing in the
  source or the APK.
- **KnownVuln, NSFW, DisabledAlgorithm, NoSourceSince**: no.
- `CAMERA` is requested only for the pairing QR code, after the user taps Scan.

## Likely reviewer questions

**Why the `sudo:` block?** The app's build uses `kotlin { jvmToolchain(17) }`. F-Droid's
build server is Debian trixie, whose only JDK is 21, and the build server turns off Gradle's
toolchain downloads, so Gradle finds no JDK 17. The four lines add bookworm's
`openjdk-17-jdk-headless` and make it the default `java`, the pattern many fdroiddata recipes
use. Staying on JDK 17 matches our CI's JDK, which matters for reproducible builds. (Locally,
`fdroid build` skips `sudo:`, so the local build ran with Temurin 17 on the PATH.)

**What is `plugins/sdk-node/package-lock.json`?** The scanner scans the whole repository and
stops on a `package.json` without a lockfile. The Node plugin SDK has no dependencies, so its
lockfile is empty; it's there only so the scanner passes. Nothing under `plugins/` is part of
the APK.

**What are `host/` and `plugins/`?** The desktop side of the same project: the host and tray
app in Rust (`host/`, with `Cargo.lock` and the app's `package-lock.json`) and the
first-party plugins in Python and Node.js (`plugins/`). The APK is built only from
`android/` (`subdir: android/app`; `settings.gradle.kts` is one level up). `host/vendor/enigo`
is a vendored Rust crate's source, not a binary. The only binary files in the tree are the
Gradle wrapper jar and fonts used by the desktop renderer and the website
(`host/crates/render/assets/*.otf`, `website/fonts/*.woff2`); the scanner flags none of them.

**The Gradle wrapper?** `android/gradle/wrapper/gradle-wrapper.jar` is the official wrapper.
The scanner removes it and `fdroid build` runs `gradlew-fdroid` instead, which has Gradle
8.14.3 (our `distributionUrl`) in its checksum table.

**Signing?** The release build signs only when `PERRITO_KEYSTORE` is set, so it's unsigned
for F-Droid; `fdroid build` also strips the `signingConfigs` block. `dependenciesInfo` is off
(no encrypted dependency blob) and `vcsInfo` is off for release builds.

**Why minSdk 22?** Amazon Fire tablets on Fire OS 5 (API 22) are a main target.

**`AutoName: Perrito` but the store title is longer?** The launcher label is "Perrito";
`title.txt` is "Perrito: tablet control deck".

## Screenshots

`fastlane/metadata/android/en-US/images/` has what F-Droid shows: `icon.png` (512×512),
`phoneScreenshots/1.png` and `2.png` (1080×1920), `tenInchScreenshots/1.png` to `3.png`
(1920×1200), and `featureGraphic.png` (1024×500). Nothing is missing. The shot list in
[README.md](README.md#screenshot-shot-list) has four phone and four ten-inch shots; the ones
not taken yet (phone: a key press with the ✓ feedback, the host list on Wi-Fi; ten-inch: the
Fire 7 at 1024×600) are optional extras, not requirements. `sevenInchScreenshots/` is
optional too.
