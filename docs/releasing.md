# Releasing

A release is one tag. Pushing `vX.Y.Z` runs the **Release** workflow, which builds the release files and leaves a **draft** GitHub Release for a person to check and publish. Which platforms a release carries is the `SHIP` list at the top of `release.yml` ([What ships](#what-ships)): v0.1.0 is Windows and Android only. The Android APK has to be signed: without the release key, a tag build fails (see [Signing secrets](#signing-secrets-none-are-set-yet)). The desktop apps still build without their signing secrets, just unsigned. Releases are on hold until the maintainer says go.

## What ships

The first release is v0.1.0, for Windows and Android only (decision D5, 2026-10-15). Its draft gets five files:

| File | What |
|---|---|
| `perritodeck-0.1.0-windows-x64-setup.exe` | the desktop app's NSIS installer (per user, no admin needed) |
| `perrito-0.1.0.apk` | Perrito, signed with the release key |
| `perritodeck-0.1.0-plugin-catalogue.zip` | the plugin catalogue, what perritodeck.dev/plugins/ serves |
| `third-party-licenses.txt` | the licence texts of every dependency |
| `SHA256SUMS` | a checksum for each file above |

Not in the first release: the MSI, the macOS `.dmg`, the Linux AppImage, `.deb` and `.rpm`, and the command-line host archives. They join a later release once they've been tested on real hardware. Until then Linux and macOS still build and test in CI (weekly, or by hand) once CI runs again, which is after the repository goes public, and Linux can still be built from source.

How the workflow does it: `SHIP`, at the top of `.github/workflows/release.yml`, lists what a release publishes besides the APK, the catalogue, the licences and `SHA256SUMS`, which every release has. For v0.1.0 it's `windows-nsis`. A tag build runs only the build jobs for what's listed: the Windows desktop job builds the NSIS installer alone, the command-line job is skipped, and no Linux or macOS build job runs, so a platform that isn't listed can't fail the release. (The other jobs, such as the APK and the licences, run on Linux as usual.) Before it makes the draft, the publish job checks that the files are exactly those, and the release notes have a row only for them.

To test what doesn't ship, run the workflow by hand ([step 5](#5-tag-maintainer-only)) with **untested** ticked and **publish** unticked. It builds everything (the MSI next to the NSIS installer, Linux, macOS and the three command-line archives) and keeps the files as workflow artifacts. Ticking both stops the run at Check versions, so a draft never gets an untested file.

**To add a platform back:**
1. Test it on real hardware, from an untested run's artifacts.
2. Add its name to `SHIP` in `release.yml`: `windows-msi`, `macos`, `linux`, `cli-windows`, `cli-linux` or `cli-macos`. The comment above `SHIP` lists each one's files. That one edit turns its jobs back on for a tag, adds its files to the draft and to the publish job's check, and adds its row to the release notes. A version with a suffix (like `0.3.0-rc.1`) never gets the MSI, even when `SHIP` lists it, because MSI versions must be numeric.
3. Update the website: give the platform its download links on `website/download.html`, and change every line that says it's missing. "Launch day" in [`docs/website.md`](website.md) lists those pages (the D5 item). Update the table above and the one in step 6 too.
4. Record the decision in [`DECISIONS.md`](DECISIONS.md).

## The checklist

### 1. Before the first release only
- [x] Desktop bundle identifier: `dev.perritodeck.PerritoDeck` (decided 2026-10-11; matches the Flatpak app-id).
- [ ] Android release key, its three secrets and the two variables `PERRITO_KEY_ALIAS` and `PERRITO_CERT_SHA256` (below). A tag build fails without the secrets and `PERRITO_KEY_ALIAS`.
- [ ] The release certificate's fingerprint on the download page: fill in the placeholder under "Check your download" in `website/download.html`.
- [x] Third-party licence texts in the APK: it carries `assets/third-party-licenses.txt`, generated at build time from the release build's dependencies, and Perrito shows it under Open-source licences (done 2026-10-15; not yet looked at on a device). The desktop installers carry `third-party-licenses.txt` too.
- [ ] Check that `website/download.html` offers only what ships ([What ships](#what-ships)), for v0.1.0 the Windows installer and the APK, and that the deployed page does too.
- [ ] Optional, and not needed while macOS isn't released: Apple Developer ID and notarisation secrets (below). Without them the Mac app is ad-hoc signed and people have to use "Open Anyway".
- [ ] Optional: Windows code signing (below). The maintainer is waiting for the Azure Artifact Signing certificate; v0.1.0 ships unsigned unless it arrives first. Without it, SmartScreen warns.
- [ ] Make the repository public before launch. The website's download links and every store need it.
- [ ] Right after that, run every workflow once by hand, the weekly ones included. All of them must pass before the first tag. Every Actions run since 2026-10-11 has stopped before a job started, and CI stays blocked until the repository is public, so a release must not be the first thing to meet the CI machines ([`DECISIONS.md`](DECISIONS.md), 2026-10-15).
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
- **Plugins** each have their own version in `plugins/<id>/perritodeck-plugin.json`. Bump one when its code changes: the catalogue zip is named after the version, and a changed zip under an old name breaks the SHA-256 that existing catalogues pin. Its copy of the Python SDK (`perritodeck_plugin.py`) is part of its code, so a change to the SDK means bumping every Python plugin (`git diff --stat <last release>.. -- plugins/` lists them). `plugins/sdk-node/package.json` is versioned on its own as well; after bumping it, run `npm install --package-lock-only` in `plugins/sdk-node` so `package-lock.json` matches. (The SDK has no dependencies, but F-Droid's scanner refuses to build a source tree with a `package.json` that has no lockfile.)
- **Protocol fixtures** (`protocol/fixtures/*.json`) contain example versions. Leave them alone.
- **Flatpak, AUR and F-Droid recipes**: they're updated after the tag (see `packaging/README.md` and `docs/fdroid/README.md`).

Pre-release tags like `v0.3.0-rc.1` work, but MSI needs numeric versions, so a version with a suffix never gets the MSI, even once `SHIP` lists it. Every `0.x` and every `-suffix` version is marked pre-release on GitHub, but only the suffix drops the MSI: `v0.2.0` gets one if `SHIP` lists `windows-msi`.

### 3. Write the changelog
- [ ] `fastlane/metadata/android/en-US/changelogs/<new versionCode>.txt`: 500 characters at most, plain text. F-Droid and IzzyOnDroid show it, and the release notes include it. The workflow fails without it.
- [ ] Optional: a `<release>` entry in `packaging/flatpak/dev.perritodeck.PerritoDeck.metainfo.xml`.

### 4. Check locally, commit
```sh
python3 scripts/release-version.py --check 0.2.0
cd host && cargo test && cd app/src-tauri && cargo check --locked
```
Commit ("release: v0.2.0"), push to `main` and let CI pass. The push changes `website/download.html`, so the Site publish workflow publishes the website with the new download links, and they return 404 until the release is published: go on to the tag straight away.

### 5. Tag (maintainer only)
```sh
git tag -a v0.2.0 -m "PerritoDeck 0.2.0" && git push origin v0.2.0
```
To try the build **without** a release, go to Actions → Release → Run workflow, give an existing tag and leave "publish" unticked. It builds what a tag would, and the files are only kept as workflow artifacts. Tick "untested" as well to also build what doesn't ship ([What ships](#what-ships)). Without the Android key, such a run builds `perrito-X.Y.Z-unsigned.apk`; with "publish" ticked, it needs the key just like a tag build. Once the `release` environment is in use, pick the tag under "Use workflow from" as well.

### 6. What the workflow does
| Job | Output |
|---|---|
| Check versions | fails early if any version or the changelog is missing, and works out from `SHIP` which desktop and command-line builds run |
| Android APK | `perrito-X.Y.Z.apk`, signed and checked (see [what CI checks](#android-the-release-key)). A manual run without the key and without "publish" gives `perrito-X.Y.Z-unsigned.apk`. The APK carries `assets/third-party-licenses.txt`, generated at build time, which Perrito shows under Open-source licences. The Play bundle `perrito-X.Y.Z.aab` stays a workflow artifact (`play-bundle`), not a release asset; it's kept for Google Play, which isn't used for now (D8) |
| Third-party licence texts | `third-party-licenses.txt`: every dependency's licence text, from `scripts/third-party-notices.py --check --full` (run after `cargo fetch` and `npm ci`, since it reads each package's own licence files). `--check` fails the release if `THIRD_PARTY_NOTICES.md` is out of date, and a fixed `PYTHONHASHSEED` gives the same file on every run |
| Command-line host | Skipped on a v0.1.0 tag: `SHIP` lists no `cli-*`. With "untested": `perritodeck-host-X.Y.Z-linux-x86_64.tar.gz`, `-windows-x86_64.zip`, `-macos-universal.tar.gz` (`perritodeck-host`, `perritodeck-mcp` and `perritodeck-fakedeck`, plus LICENSE, `third-party-licenses.txt` and README) |
| Desktop app | On a v0.1.0 tag, Windows only: `perritodeck-X.Y.Z-windows-x64-setup.exe` (NSIS, per user, no admin). With "untested", also `-windows-x64.msi` (per machine), `-macos-universal.dmg`, `-linux-x86_64.AppImage`, `-linux-amd64.deb` and `-linux-x86_64.rpm`. `third-party-licenses.txt` goes in the app's resource folder, and the first-party plugins inside come with their MIT licence (`plugins/LICENSE`) |
| Plugin catalogue | `perritodeck-X.Y.Z-plugin-catalogue.zip`, what `plugins/build_catalog.py` produces (index.json, index.html, one zip per plugin) |
| Draft GitHub Release | checks that the files are exactly what `SHIP` lists plus the APK, the catalogue and the licences, and attaches them with `SHA256SUMS`. The notes have a downloads row for each of them, the Android certificate's SHA-256 and the Android changelog, plus GitHub's generated notes (only a "Full Changelog" link while changes land without pull requests). If the draft already exists, files of the same name are replaced and any others stay, so delete a stale one by hand |

Linux builds run on Ubuntu 22.04 so they work with glibc 2.35 and newer. A published (non-draft) release is never overwritten: the job stops instead. The workflow never creates tags (`--verify-tag`). Every job has a time limit, so a stuck runner can't use up minutes for hours. Every action in `.github/workflows/` is pinned to a full commit SHA, with its version in a comment after it, so a moved or hijacked tag can't change what runs; `.github/dependabot.yml` checks for new versions weekly and opens one pull request for minor and patch updates and one for new major versions.

Cost: a v0.1.0 tag runs 5 Linux jobs and 1 Windows job (2× rate); a manual run without "publish" runs 4 Linux and 1 Windows. With "untested" it's 6 Linux, 2 Windows and 2 macOS (10× rate). Don't re-run it casually.

### 7. Publish
- [ ] Open the draft. Check the asset list against [What ships](#what-ships) (the APK must not be `-unsigned`), check that the certificate SHA-256 in the notes is the one on the download page, try the Windows installer on a PC and the APK on a tablet, and compare a couple of checksums.
- [ ] Press **Publish release**.
- [ ] The website: GitHub Pages serves it from `perritodeck/perritodeck.github.io` ([`docs/website.md`](website.md#deploy)), and it changes only when a publish runs, not with every push to `main`. After the flip that's the Site publish workflow, on a push to `main` that touches `website/` or `plugins/`, or by hand (Actions → Site publish → Run workflow); before the flip, `scripts/publish_site.py` run by hand from WSL. So once Site publish has run on the release commit from step 4, which bumped the links in `download.html`, they're live, and until **Publish release** the download page links to files that return 404: tag and publish promptly. Check that the run published ("site: no change" would mean it didn't see the commit). Each publish builds the plugin catalogue itself from `main` (`website/plugins/` isn't committed).
- [ ] First release only: work through "Launch day" under "Deploy" in `docs/website.md`, then commit, push to `main`, check that Site publish ran, and check that the live site shows the change.
- [ ] Stores: F-Droid picks up new tags by itself once listed; IzzyOnDroid picks up the APK. There's no Google Play for now (D8), so the `play-bundle` artifact isn't uploaded anywhere. The Flathub and AUR recipes (`packaging/README.md`) wait until Linux ships ([What ships](#what-ships)).

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
- `apksigner verify --verbose --print-certs --min-sdk-version 22` passes, with exactly one signer and both v1 and v2 signatures verified (Android 5.1 and 6 check v1, Android 7 and newer check v2). v3, which allows moving to a new key later, stays off (D7, 2026-10-17);
- if the repository variable `PERRITO_CERT_SHA256` is set, the certificate must match it, so the wrong key in the secrets fails the build. Without the variable the job only warns.

The certificate's SHA-256 goes in the Android job summary and the release notes. Publish it once on the download page (step 1) for people who verify APKs; it stays the same for every release.

To try a signed build locally (for testing only; never publish a locally built APK): set the four environment variables Gradle reads (`PERRITO_KEYSTORE`, the path to the `.p12`, and `PERRITO_KEYSTORE_PASSWORD`, `PERRITO_KEY_ALIAS` and `PERRITO_KEY_PASSWORD`) in that shell only, run `./gradlew --no-configuration-cache assembleRelease` in `android/`, check it with `apksigner verify --print-certs app/build/outputs/apk/release/app-release.apk`, and clear the variables afterwards. The release app (`dev.perritodeck.client`) installs next to the debug one (`dev.perritodeck.client.debug`) and pairs separately.

Google Play isn't used for now (D8). If it ever is, D7 (2026-10-17) settles the keys: Play App Signing with the app-signing key Google generates, and this release key as the **upload key**. That's how the workflow already signs the `play-bundle` artifact, so nothing changes. Google's key signs what Play installs, so a Play install and an install from GitHub, IzzyOnDroid or F-Droid can't update each other.

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

### Windows: code signing (waiting for a certificate)
Unsigned installers trigger SmartScreen until they build up reputation. The maintainer keeps the **Azure Artifact Signing** account and is waiting for its identity validation and certificate (decision D1, 2026-10-15). v0.1.0 ships unsigned unless the certificate arrives first.
- **Before wiring it in**, check which name the certificate shows as the publisher. An individual validation shows a legal name, and that name then appears in every signature and in the SmartScreen prompt.
- Azure Artifact Signing is used through `bundle.windows.signCommand` or Microsoft's signing action.
- **SignPath Foundation** stays the alternative: free certificates for open-source projects, used through their GitHub Action. Needs an application and a public repository.
- OV/EV certificates on hardware tokens can't be used from hosted CI.

Once it's set up, add a Windows-only step in the desktop job before "Build and bundle", behind a secret check like the macOS one, so a missing secret means an unsigned build and never a failed release.

## Not automated yet
winget manifest (`wingetcreate new` after the first release), Homebrew cask, Flathub and AUR submissions (drafts in `packaging/`), a Play upload (`fastlane supply`) if Play is ever used (D8), signed `SHA256SUMS` (minisign, or GitHub artifact attestations, which need a public repository), Linux ARM64 builds (GitHub's free ARM runners are for public repositories).
