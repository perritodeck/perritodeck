# Submitting Perrito to F-Droid (v0.1.0)

Everything here is ready for the maintainer to send. Nothing has been submitted, forked or
posted. The recipe to submit is [`dev.perritodeck.client.yml`](dev.perritodeck.client.yml),
unchanged.

## Where it stands (checked 2026-10-25)

| Check | Result |
|---|---|
| Recipe | `commit:` is the full hash of `v0.1.0` (`c396920de64705018c07d57771dfb6cfd62ababd`, `git rev-parse v0.1.0^{commit}`), `CurrentVersion: 0.1.0`, `CurrentVersionCode: 1`, the `sudo:` JDK 17 lines kept, `Binaries`/`AllowedAPKSigningKeys` left out (D11) |
| `fdroid readmeta`, `lint -v`, `lint -f`, `rewritemeta --list` | clean with fdroidserver 2.4.5 (PyPI) and git master `2d1e622` with ruamel.yaml 0.18.10 (what fdroiddata's CI runs) |
| fdroiddata CI's rewritemeta job (`fdroid rewritemeta`, then `git diff --exit-code`) | no change, with both versions |
| yamllint with fdroiddata's `.yamllint`; `check-jsonschema` with fdroiddata's `schemas/metadata.json` | clean (fdroiddata at `0a6edd29`, 2026-10-25) |
| `fdroid build -v -l dev.perritodeck.client` (2.4.5) of the recipe as is: `Repo:` the public GitHub repo, the tag's hash, no patch, Temurin 17 on the PATH (`fdroid build` skips `sudo:` off the build server) | **builds**: "Successfully built version 0.1.0 of dev.perritodeck.client from c396920…". The source scan removed only `gradle-wrapper.jar`; `fdroid scanner --exit-code` on the APK finds nothing |
| Unsigned APK from that build | 2,209,009 bytes, SHA-256 `c36dca71618c7341a3269eb6e415eb65b3ae1e879d28da956c1db253cadd1977`; `dev.perritodeck.client`, versionCode 1, versionName 0.1.0, minSdk 22, targetSdk 36 |
| Compared with the published `perrito-0.1.0.apk` (SHA-256 `6bc15a42…a512154`, signer `848c5d3f…cc98f24e5`) | Same 433 entries in the same order, same compression; **431 identical**, only `assets/dexopt/baseline.prof` (4,938 vs 4,929 bytes) and `baseline.profm` (603 vs 607) differ. `classes.dex`, resources and all 12 `.so` files match. F-Droid's own check (`fdroidserver.common.verify_apks`: copy our signature onto its build, then `apksigner verify`) fails on exactly those two files |

So stage 2 (`Binaries:` + `AllowedAPKSigningKeys:`, F-Droid publishing our signed APK) would
**not** work for v0.1.0 today: the baseline profile is the only thing in the way, as the
2026-10-24 measurements predicted (D11, reopened). The logs are in WSL under
`~/pd/fdroid/logs/60-lint-release.log`, `61-build-release-venv.log` and
`62-compare-release.log`.

## Decide first: F-Droid's signature or ours

The merge request template says: if reproducible builds aren't enabled, F-Droid signs the
APK with its own key, and you can't switch to yours later. F-Droid users would then have a
differently signed Perrito from GitHub and IzzyOnDroid users, and moving them to our key
later means they uninstall and reinstall. Two ways to go:

1. **Submit v0.1.0 now** as it is. F-Droid signs it. Fine if the F-Droid listing may stay
   F-Droid-signed for good. Answer the reproducible-builds box with the reason below.
2. **Settle D11 first**, release v0.1.1 with a deterministic profile (the options are in
   [README.md](README.md#reproducible-builds)), check it with a second `fdroid build` and the
   comparison above, then submit with `Binaries:` and `AllowedAPKSigningKeys:
   848c5d3f31f3bef31ecdaf308d4fa1db6a2c0a78b63ce6dacb3d6bacc98f24e5` added where the README
   says. This is the cleaner long-term option; it delays the listing by one release.

The steps below work for either; for option 2 swap in the v0.1.1 recipe.

## Steps (maintainer)

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
4. Optional local check, if fdroidserver is installed (`pip install git+https://gitlab.com/fdroid/fdroidserver.git`):
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
- https://raw.githubusercontent.com/perritodeck/perritodeck/v0.1.0/fastlane/metadata/android/en-US/images/phoneScreenshots/1.png
- https://raw.githubusercontent.com/perritodeck/perritodeck/v0.1.0/fastlane/metadata/android/en-US/images/phoneScreenshots/2.png
- https://raw.githubusercontent.com/perritodeck/perritodeck/v0.1.0/fastlane/metadata/android/en-US/images/tenInchScreenshots/1.png
- https://raw.githubusercontent.com/perritodeck/perritodeck/v0.1.0/fastlane/metadata/android/en-US/images/tenInchScreenshots/2.png
- https://raw.githubusercontent.com/perritodeck/perritodeck/v0.1.0/fastlane/metadata/android/en-US/images/tenInchScreenshots/3.png

### Please elaborate why you wrote this app. Please highlight the unique functions it provides over other similar apps in F-Droid.

Hardware button decks are expensive and the popular tablet alternatives are proprietary, need an account or a cloud service, or don't run on old tablets. PerritoDeck gives an old tablet a new job with a fully FOSS stack (Apache-2.0, the plugins MIT) and a documented protocol. Compared with remote-control apps in F-Droid such as KDE Connect's "Run command", Perrito is a dedicated control surface: the computer draws the keys (titles, icons, animated GIFs, live values such as CPU, the time or the OBS scene), there are sliders, toggles that follow live state, pages, and profiles that switch with the app in front; the desktop side has OBS Studio and Home Assistant built in and plugins in Python or Node.js. It works back to API 22, so Fire OS 5 tablets are supported.

## Checklist

### Policy

* [x] The app complies with the [inclusion criteria](https://f-droid.org/docs/Inclusion_Policy).
* [x] The original app author has been notified (and does not oppose the inclusion). If you are not the author, please paste the link of the reply from the author.
  I am the upstream maintainer (https://github.com/perritodeck/perritodeck).
* [x] The upstream app source code repo contains the app metadata in a [Fastlane](https://gitlab.com/snippets/1895688) or [Triple-T](https://gitlab.com/snippets/1901490) folder structure. The summary and description must be included and images, icon, and changelog should also be provided for better user experience. The `en-US` locale must be included.
  `fastlane/metadata/android/en-US/`: title, short and full description, icon, feature graphic, phone and ten-inch screenshots, `changelogs/1.txt`.

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
* [ ] Enable [Reproducible Builds](https://f-droid.org/docs/Reproducible_Builds). We'll use your signature for improved security/reliability, also allowing users to switch between different channels. Do note that if you don't enable reproducible build then the apk will be signed with our key so you can't enable it later. If you can't enable this, please add the reasons here.
  Not yet. An `fdroid build` of v0.1.0 matches our signed release APK (https://github.com/perritodeck/perritodeck/releases/download/v0.1.0/perrito-0.1.0.apk) in every entry except the ART baseline profile (`assets/dexopt/baseline.prof` and `baseline.profm`): R8's rewriting of the profile isn't deterministic, and it differs between runs on one machine too. `classes.dex`, resources and native libraries are identical. We're fixing the profile and would rather switch to `Binaries`/`AllowedAPKSigningKeys` once a release builds identically.
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

If you go with option 2 (D11 settled, `Binaries:` in the recipe), tick the reproducible-builds
box and replace its note with "Enabled: `Binaries` and `AllowedAPKSigningKeys` are set; an
`fdroid build` of the tag matches our signed APK."

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
