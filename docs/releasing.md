# Releasing

A release is one tag. Pushing `vX.Y.Z` runs the **Release** workflow, which builds every file and leaves a **draft** GitHub Release for a person to check and publish. The Android APK has to be signed for that: without the release key, a tag build fails (see [Signing secrets](#signing-secrets-none-are-set-yet)). The desktop apps still build without their signing secrets, just unsigned. Releases are on hold until the maintainer says go.

## The checklist

### 1. Before the first release only
- [x] Desktop bundle identifier: `dev.perritodeck.PerritoDeck` (decided 2026-10-11; matches the Flatpak app-id).
- [ ] Android release key, its three secrets and the two variables `PERRITO_KEY_ALIAS` and `PERRITO_CERT_SHA256` (below). A tag build fails without the secrets and `PERRITO_KEY_ALIAS`.
- [ ] The release certificate's fingerprint on the download page: fill in the placeholder under "Check your download" in `website/download.html`.
- [ ] **Release blocker:** third-party licence texts in the APK. The command-line archives and the desktop installers carry `third-party-licenses.txt`, but the APK doesn't carry any third-party licence texts yet and the app has no licences screen. Solve this before the first signed release; it's a release blocker in [`launch-checklist.md`](launch-checklist.md).
- [ ] Optional: Apple Developer ID and notarisation secrets (below). Without them the Mac app is ad-hoc signed and people have to use "Open Anyway".
- [ ] Optional: Windows code signing (below). Without it, SmartScreen warns.
- [ ] Make the repository public before launch. The website's download links and every store need it.
- [ ] Once it's public: the `release` environment for the Android signing secrets ([below](#once-the-repository-is-public-a-release-environment)).

### 2. Bump the version
```sh
python3 scripts/release-version.py 0.2.0     # sets every version below, then lists them
python3 scripts/release-version.py           # just list them (fails if they differ)
```
It changes these, all of which must match the tag (the workflow runs `--check`):

| File | Field |
|---|---|
| `host/Cargo.toml` | `[workspace.package] version` (every host crate inherits it) |
| `host/Cargo.lock`, `host/app/src-tauri/Cargo.lock` | the `perritodeck-*` entries, so `--locked` builds keep working |
| `host/app/src-tauri/Cargo.toml` | `[package] version` |
| `host/app/src-tauri/tauri.conf.json` | top-level `version` (installer and bundle versions) |
| `host/app/package.json`, `host/app/package-lock.json` | `version` |
| `android/app/build.gradle.kts` | `versionName`, and **`versionCode` + 1** |
| `ios/project.yml` | `MARKETING_VERSION`, and **`CURRENT_PROJECT_VERSION` + 1** |
| `website/download.html` | the version in every download link |

Not bumped automatically:
- **Plugins** each have their own version in `plugins/<id>/perritodeck-plugin.json`. Bump one when its code changes: the catalogue zip is named after the version, and a changed zip under an old name breaks the SHA-256 that existing catalogues pin. Its copy of the Python SDK (`perritodeck_plugin.py`) is part of its code, so a change to the SDK means bumping every Python plugin (`git diff --stat <last release>.. -- plugins/` lists them). `plugins/sdk-node/package.json` is versioned on its own as well.
- **Protocol fixtures** (`protocol/fixtures/*.json`) contain example versions. Leave them alone.
- **Flatpak, AUR and F-Droid recipes**: they're updated after the tag (see `packaging/README.md` and `docs/fdroid/README.md`).

Pre-release tags like `v0.3.0-rc.1` work, but MSI needs numeric versions, so a pre-release ships only the NSIS `.exe` for Windows. Every `0.x` and every `-suffix` version is marked pre-release on GitHub.

### 3. Write the changelog
- [ ] `fastlane/metadata/android/en-US/changelogs/<new versionCode>.txt`: 500 characters at most, plain text. F-Droid, IzzyOnDroid and Play show it, and the release notes include it. The workflow fails without it.
- [ ] Optional: a `<release>` entry in `packaging/flatpak/dev.perritodeck.PerritoDeck.metainfo.xml`.

### 4. Check locally, commit
```sh
python3 scripts/release-version.py --check 0.2.0
cd host && cargo test && cd app/src-tauri && cargo check --locked
```
Commit ("release: v0.2.0"), push to `main` and let CI pass. This push also redeploys the website with the new download links, which don't work until the release is published (step 7), so go on to the tag straight away.

### 5. Tag (maintainer only)
```sh
git tag -a v0.2.0 -m "PerritoDeck 0.2.0" && git push origin v0.2.0
```
To try the whole build **without** a release, go to Actions → Release → Run workflow, give an existing tag and leave "publish" unticked. The files are then only kept as workflow artifacts. Without the Android key, such a run builds `perrito-X.Y.Z-unsigned.apk`; with "publish" ticked, it needs the key just like a tag build. Once the `release` environment is in use, pick the tag under "Use workflow from" as well.

### 6. What the workflow does
| Job | Output |
|---|---|
| Check versions | fails early if any version or the changelog is missing |
| Android APK | `perrito-X.Y.Z.apk`, signed and checked (see [what CI checks](#android-the-release-key)). A manual run without the key and without "publish" gives `perrito-X.Y.Z-unsigned.apk`. The APK doesn't carry the third-party licence texts yet (step 1). The Play bundle `perrito-X.Y.Z.aab` stays a workflow artifact (`play-bundle`), not a release asset |
| Third-party licence texts | `third-party-licenses.txt`: every dependency's licence text, from `scripts/third-party-notices.py --check --full` (run after `cargo fetch` and `npm ci`, since it reads each package's own licence files). `--check` fails the release if `THIRD_PARTY_NOTICES.md` is out of date, and a fixed `PYTHONHASHSEED` gives the same file on every run |
| Command-line host ×3 | `perritodeck-host-X.Y.Z-linux-x86_64.tar.gz`, `-windows-x86_64.zip`, `-macos-universal.tar.gz` (`perritodeck-host`, `perritodeck-mcp` and `perritodeck-fakedeck`, plus LICENSE, `third-party-licenses.txt` and README) |
| Desktop app ×3 | `perritodeck-X.Y.Z-windows-x64-setup.exe` (NSIS, per user, no admin), `-windows-x64.msi` (per machine), `-macos-universal.dmg`, `-linux-x86_64.AppImage`, `-linux-amd64.deb`, `-linux-x86_64.rpm`. `third-party-licenses.txt` goes in the app's resource folder, and the first-party plugins inside come with their MIT licence (`plugins/LICENSE`) |
| Plugin catalogue | `perritodeck-X.Y.Z-plugin-catalogue.zip`, what `plugins/build_catalog.py` produces (index.json, index.html, one zip per plugin) |
| Draft GitHub Release | all of the above plus `SHA256SUMS`, with notes made from the Android changelog and the Android certificate's SHA-256, plus GitHub's generated notes (only a "Full Changelog" link while changes land without pull requests) |

Linux builds run on Ubuntu 22.04 so they work with glibc 2.35 and newer. A published (non-draft) release is never overwritten: the job stops instead. The workflow never creates tags (`--verify-tag`). Every job has a time limit, so a stuck runner can't use up minutes for hours. Every action in `.github/workflows/` is pinned to a full commit SHA, with its version in a comment after it, so a moved or hijacked tag can't change what runs; `.github/dependabot.yml` checks for new versions weekly and opens one pull request for minor and patch updates and one for new major versions.

Cost: 7 Linux (6 when nothing is published), 2 Windows (2× rate) and 2 macOS (10× rate) jobs per run. Don't re-run it casually.

### 7. Publish
- [ ] Open the draft. Check the asset list (the APK must not be `-unsigned`), check that the certificate SHA-256 in the notes is the one on the download page, try the installers on each OS and the APK on a tablet, and compare a couple of checksums.
- [ ] Press **Publish release**.
- [ ] The website needs no upload once it's on Cloudflare Pages as `website/README.md` sets it up: every push to `main` that touches `website/` or `plugins/` redeploys it and builds the plugin catalogue as part of the deploy. That includes the release commit from step 4, which bumped the links in `download.html`. From that push until **Publish release**, the live download page links to files that don't exist yet and return 404, so tag and publish promptly. If the site is ever served without that build step, unzip the release's `perritodeck-X.Y.Z-plugin-catalogue.zip` into `website/plugins/` before deploying.
- [ ] First release only: work through "Launch day" under "Deploy" in `website/README.md`, then commit and push to `main` so Pages redeploys.
- [ ] Stores: F-Droid picks up new tags by itself once listed; IzzyOnDroid picks up the APK; upload the `play-bundle` artifact to Play; update the Flathub and AUR recipes (`packaging/README.md`).

## Signing secrets (none are set yet)

Secrets are set with `gh secret set NAME -R perritodeck/perritodeck`, and variables, which hold public values, with `gh variable set NAME -R perritodeck/perritodeck --body <value>` (maintainer only). Give a key or password on standard input or at the hidden prompt, never after `--body`, where it ends up in the shell history and the process list. The Android secrets and `PERRITO_KEY_ALIAS` are required for a release. The Apple and Windows ones are optional: whatever is missing there is skipped with a warning.

### Android: the release key
It signs every APK published outside Google Play (GitHub Releases, IzzyOnDroid, Obtainium, and F-Droid once builds are reproducible). **If it's lost, existing users can't update** and would have to uninstall first. Back it up offline (password manager plus an encrypted USB stick) and check that the backup opens. The key doesn't exist yet: the maintainer generates it before the first signed release, and nothing real has been signed so far.

| Name | Kind | What |
|---|---|---|
| `PERRITO_KEYSTORE_BASE64` | secret | the `.p12` keystore, base64 |
| `PERRITO_KEYSTORE_PASSWORD` | secret | its password |
| `PERRITO_KEY_PASSWORD` | secret | the key's password: the same one, in a PKCS12 keystore |
| `PERRITO_KEY_ALIAS` | variable | the key's alias, `perrito` |
| `PERRITO_CERT_SHA256` | variable, optional but recommended | the certificate's SHA-256, which the signed APK must match |

The alias is a variable because it isn't secret, and because GitHub hides every secret's value in the logs: as a secret, `perrito` would turn every "perrito" in the job log into `***`.

```sh
# Once (valid for 30 years). Use JDK 17's keytool. A PKCS12 keystore has a single password,
# for both the store and the key, so keytool asks for just that one (twice, to confirm it).
keytool -genkeypair -v -storetype PKCS12 -keystore perrito-release.p12 -alias perrito \
  -keyalg RSA -keysize 4096 -validity 10950 -dname "CN=PerritoDeck, O=PerritoDeck"
keytool -list -v -keystore perrito-release.p12 -alias perrito    # note the SHA256 line

base64 -w0 perrito-release.p12 | gh secret set PERRITO_KEYSTORE_BASE64 -R perritodeck/perritodeck
gh secret set PERRITO_KEYSTORE_PASSWORD -R perritodeck/perritodeck   # hidden prompt
gh secret set PERRITO_KEY_PASSWORD -R perritodeck/perritodeck        # the same password again
# Public, not secrets: the alias and the certificate's SHA-256 (colons and case don't matter).
gh variable set PERRITO_KEY_ALIAS -R perritodeck/perritodeck --body perrito
gh variable set PERRITO_CERT_SHA256 -R perritodeck/perritodeck --body <SHA256 from keytool>
```
On macOS use `base64 -i perrito-release.p12 | gh secret set ...`. In PowerShell: `[Convert]::ToBase64String([IO.File]::ReadAllBytes((Resolve-Path perrito-release.p12))) | gh secret set PERRITO_KEYSTORE_BASE64 -R perritodeck/perritodeck` (the line ending PowerShell adds is ignored).

**The three secrets and `PERRITO_KEY_ALIAS`: all of them, or none.** If only some are set, the Android job stops with an error naming the missing ones instead of building an unsigned or wrongly signed APK. `PERRITO_CERT_SHA256` isn't part of that: without it the job only warns (below). A local build behaves the same way: with `PERRITO_KEYSTORE` set, Gradle stops with an error until the password, alias and key password are set too. A blank `PERRITO_KEYSTORE` counts as unset.

What CI checks, so a bad key stops the release rather than reaching users:
- the keystore opens with the password and has the alias, before anything is built;
- Gradle runs with `--no-configuration-cache`, so the passwords aren't stored in its cache, and the decoded keystore is deleted afterwards whether the build worked or not;
- `apksigner verify --verbose --print-certs --min-sdk-version 22` passes, with exactly one signer and both v1 and v2 signatures verified (Android 5.1 and 6 check v1, Android 7 and newer check v2). v3, which allows moving to a new key later, stays off until that's decided;
- if the repository variable `PERRITO_CERT_SHA256` is set, the certificate must match it, so the wrong key in the secrets fails the build. Without the variable the job only warns.

The certificate's SHA-256 goes in the Android job summary and the release notes. Publish it once on the download page (step 1) for people who verify APKs; it stays the same for every release.

To try a signed build locally (for testing only; never publish a locally built APK): set the four environment variables Gradle reads (`PERRITO_KEYSTORE`, the path to the `.p12`, and `PERRITO_KEYSTORE_PASSWORD`, `PERRITO_KEY_ALIAS` and `PERRITO_KEY_PASSWORD`) in that shell only, run `./gradlew --no-configuration-cache assembleRelease` in `android/`, check it with `apksigner verify --print-certs app/build/outputs/apk/release/app-release.apk`, and clear the variables afterwards. The release app (`dev.perritodeck.client`) installs next to the debug one (`dev.perritodeck.client.debug`) and pairs separately.

Google Play uses a separate **upload key** registered in the Play Console; Play re-signs with its own app-signing key. Keep the two apart. For now the workflow signs the `play-bundle` artifact with the release key, so decide on the upload key before the first Play upload.

#### Once the repository is public: a release environment
Repository secrets reach every workflow run, from any branch. A GitHub environment holds them back until a reviewer approves a run from a version tag. A private repository on the Free plan can't have environments; once the repository is public they cost nothing. Then:

1. Settings → Environments → **New environment**, named `release`.
2. Deployment branches and tags: **Selected branches and tags**, with one tag rule, `v*`, and no branches.
3. **Required reviewers**: the maintainer. Leave "Prevent self-review" off while there's only one reviewer, or the maintainer can't approve the runs their own tags start.
4. Put the three secrets in the environment: the commands above with `--env release` added, e.g. `gh secret set PERRITO_KEY_PASSWORD --env release -R perritodeck/perritodeck`. If they're already repository secrets, set them in the environment and then delete the repository copies (`gh secret delete NAME -R perritodeck/perritodeck`), or the environment protects nothing. The two variables stay repository variables.
5. At the same time, remove the `#` before `environment: release` in the Android job of `release.yml`. Only a job that names the environment can read its secrets, so without the line the job still sees the `PERRITO_KEY_ALIAS` variable but none of the three secrets, and every run, manual ones included, stops with "Only 1 of the four Android signing settings are set", naming the three secrets as missing.

From then on the Android job waits for approval on every run, and a manual run has to be started from the tag (Run workflow → "Use workflow from" → the tag): a run started from `main` isn't let into the environment and fails. The line is commented out until then because the environment doesn't exist yet.

### macOS: Developer ID and notarisation
Needs an Apple Developer Program membership (99 USD a year).

| Secret | What |
|---|---|
| `APPLE_CERTIFICATE` | "Developer ID Application" certificate exported as `.p12`, base64 |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12` export password |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: <name> (<TEAMID>)` |
| `APPLE_API_ISSUER`, `APPLE_API_KEY` | App Store Connect API key: issuer ID and key ID (for notarisation) |
| `APPLE_API_KEY_P8_BASE64` | the `AuthKey_<id>.p8` file, base64 |

If only the certificate is set, the app is signed but not notarised. If nothing is set, `"signingIdentity": "-"` in `tauri.conf.json` ad-hoc signs it, which Apple silicon needs before it will run the app at all. The command-line host binaries are plain executables, ad-hoc signed by the linker, and aren't notarised.

### Windows: code signing (not set up)
Unsigned installers trigger SmartScreen until they build up reputation. Options, none chosen yet:
- **SignPath Foundation**: free certificates for open-source projects, used through their GitHub Action. Needs an application and a public repository.
- **Azure Trusted Signing**: about 10 USD a month, with identity validation. Used through `bundle.windows.signCommand` or Microsoft's signing action.
- OV/EV certificates on hardware tokens can't be used from hosted CI.

Once one is chosen, add a Windows-only step in the desktop job before "Build and bundle", behind a secret check like the macOS one.

## Not automated yet
winget manifest (`wingetcreate new` after the first release), Homebrew cask, Flathub and AUR submissions (drafts in `packaging/`), Play upload (`fastlane supply`), signed `SHA256SUMS` (minisign, or GitHub artifact attestations, which need a public repository), Linux ARM64 builds (GitHub's free ARM runners are for public repositories).
