# Submitting PerritoDeck to winget (v0.1.0)

The manifests for [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) are in
[`manifests/p/PerritoDeck/PerritoDeck/0.1.0/`](manifests/p/PerritoDeck/PerritoDeck/0.1.0/),
at the same path they take in winget-pkgs. Nothing has been submitted, forked or posted.

| File | What it is |
|---|---|
| `PerritoDeck.PerritoDeck.yaml` | version manifest |
| `PerritoDeck.PerritoDeck.locale.en-US.yaml` | default locale: publisher, licence, URLs, descriptions, tags |
| `PerritoDeck.PerritoDeck.installer.yaml` | the NSIS installer, its SHA-256, scope and Apps & Features entry |

## What was checked (2026-10-25)

- **Schema 1.12.0**, the version winget-pkgs' pull request template asks for.
  `winget validate --manifest <dir>` with winget v1.29.380: "Manifest validation succeeded."
- **Identifier.** `winget search PerritoDeck` and `winget search --id PerritoDeck.PerritoDeck --exact`
  find nothing, there's no `manifests/p/PerritoDeck` in winget-pkgs and no pull request
  mentioning PerritoDeck.
- **Installer.** `perritodeck-0.1.0-windows-x64-setup.exe` downloaded from the release
  (8,548,497 bytes): SHA-256 `0dda603c04ffa3c20cf4930065c94a4fc2aa37cd15b984b39125cc028ea866db`,
  the same as its line in the release's `SHA256SUMS`.
- **Apps & Features entry.** From `host/app/src-tauri/tauri.conf.json` (unchanged since the
  tag) and Tauri's generated `installer.nsi`: the uninstall key is
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\PerritoDeck`, `DisplayName`
  "PerritoDeck", `Publisher` "PerritoDeck" (`bundle.publisher`), `DisplayVersion` 0.1.0,
  installed to `%LocalAppData%\PerritoDeck` without elevation (`installMode: currentUser`,
  `RequestExecutionLevel user`). The maintainer's own install of 0.1.0 shows the same in
  `winget list --name PerritoDeck --details`. Hence `ProductCode: PerritoDeck`,
  `AppsAndFeaturesEntries` with `Publisher: PerritoDeck` (the ARP publisher differs from the
  manifest's "PerritoDeck contributors", so winget needs it to match the install), and
  `DefaultInstallLocation`.
- **Silent install.** Tauri's NSIS installer takes NSIS's `/S`, which winget passes for
  `nullsoft` by default, and starts the app afterwards only when `/R` is given, so nothing
  is left running in the validation VM.
- **Not tested: `winget install --manifest`.** PerritoDeck 0.1.0 is installed on this PC
  (`ARP\User\X64\PerritoDeck`), and a test install would overwrite it and an uninstall would
  remove it. Run the test on a PC or VM without PerritoDeck (Windows Sandbox works):
  ```powershell
  winget settings --enable LocalManifestFiles   # once, from an elevated prompt
  winget install --manifest .\packaging\winget\manifests\p\PerritoDeck\PerritoDeck\0.1.0
  winget list PerritoDeck                       # should show PerritoDeck.PerritoDeck 0.1.0, not an ARP\ entry
  winget uninstall PerritoDeck.PerritoDeck
  ```

## The unsigned installer

- winget allows unsigned installers. A winget install doesn't show SmartScreen's "Windows
  protected your PC" prompt, which only appears for files downloaded and run through
  Explorer.
- The validation pipeline scans the installer with several antivirus engines, installs it
  in a VM and scans the installed files with Microsoft Defender. An unsigned NSIS installer
  is more likely to set off heuristics. If a scan flags it, submit the file to Microsoft as
  a false positive (https://www.microsoft.com/wdsi/filesubmission) with the pull request's
  URL; a moderator then reruns validation (`@wingetbot run`).
- The URL check also asks SmartScreen whether the URL is malicious or has poor reputation.
  The installer is on github.com's release CDN, so this should pass; if it doesn't, the
  pull request gets `URL-Validation-Error` and needs a moderator.
- Once the installer is signed (Azure Artifact Signing), later versions are unchanged
  manifests with a new URL and hash.

## Submitting

Use the project's GitHub account (`dmang-dev`). The first pull request to winget-pkgs asks
you to sign Microsoft's Contributor License Agreement (a bot comment with a link); until
then the pull request has `Needs-CLA` and can't merge.

**With wingetcreate** (the simplest way; it forks, branches, commits and opens the pull
request for you):
```powershell
winget install Microsoft.WingetCreate
wingetcreate submit --prtitle "New package: PerritoDeck.PerritoDeck version 0.1.0" --token <a GitHub token with public_repo> .\packaging\winget\manifests\p\PerritoDeck\PerritoDeck\0.1.0
```
Without `--token` it signs in through the browser. Later versions can use
`wingetcreate update PerritoDeck.PerritoDeck --version <v> --urls <installer URL> --submit`.

**By hand:**
1. Fork `microsoft/winget-pkgs` and make a branch, e.g. `PerritoDeck.PerritoDeck-0.1.0`.
2. Copy the three files to `manifests/p/PerritoDeck/PerritoDeck/0.1.0/` in the fork (same
   names, LF or CRLF both fine, UTF-8 without a BOM).
3. Commit (`New package: PerritoDeck.PerritoDeck version 0.1.0`), push, and open a pull
   request to `microsoft/winget-pkgs:master` with the title
   **`New package: PerritoDeck.PerritoDeck version 0.1.0`**.
4. Fill in the template:
   ```markdown
   ## 📖 Description
   New package: PerritoDeck (PerritoDeck.PerritoDeck) version 0.1.0, the Windows tray app of PerritoDeck, an open-source (Apache-2.0) app that turns an Android tablet into a programmable button deck. Installer: the per-user NSIS installer from the project's GitHub release, https://github.com/perritodeck/perritodeck/releases/tag/v0.1.0 (unsigned for now).

   ## ✅ Checklist
   - [x] Signed the [Contributor License Agreement](https://cla.opensource.microsoft.com)
   - [ ] Linked to an issue (if applicable)

   ## 📦 Manifest Checklist
   - [x] Checked that there aren't other open [pull requests](https://github.com/microsoft/winget-pkgs/pulls) for the same manifest update/change
   - [x] This PR only modifies one (1) manifest
   - [x] Validated manifest locally with `winget validate --manifest <path>` ([validation guide](https://github.com/microsoft/winget-pkgs/blob/master/doc/ValidationFailureGuide.md))
   - [ ] Tested manifest locally with `winget install --manifest <path>`
   - [x] Manifest conforms to the [1.12 schema](https://github.com/microsoft/winget-pkgs/tree/master/doc/manifest/schema/1.12.0)
   ```
   Tick "Signed the CLA" once you have, and "Tested manifest locally" only after the sandbox
   test above. Recheck the "other open pull requests" box on the day.

## What to expect

- The `wingetbot` pipeline runs within minutes to hours: manifest and path checks, URL
  checks, the installer download and hash, antivirus scans, an unattended install and
  uninstall in a VM, and a check that the installed Apps & Features entry matches
  `AppsAndFeaturesEntries` and `DefaultInstallLocation`. Success adds `Validation-Completed`.
- Labels to watch: `Needs-CLA`; `Validation-Defender-Error` or `Binary-Validation-Error`
  (scan detections, see above); `Manual-Validation` or a `Policy-Test-2.x` label when a
  moderator has to look at it by hand, which is more likely for an unsigned installer from
  a new publisher and only means a longer wait; `Needs-Author-Feedback` when a moderator
  asks something (answer in the pull request).
- After a moderator approves and it merges, the package is usually in the winget source
  within an hour: `winget install PerritoDeck.PerritoDeck`.
- Each new release needs a new pull request (new folder `.../PerritoDeck/<version>/`);
  `wingetcreate update` does it from the installer URL.

## Notes

- **WebView2.** The installer downloads and runs Microsoft's WebView2 bootstrapper itself if
  WebView2 is missing (`webviewInstallMode: downloadBootstrapper`). Windows 11 and current
  Windows 10 have it, so the manifest declares no dependency on
  `Microsoft.EdgeWebView2Runtime`.
- **Publisher name.** The manifest's `Publisher` is "PerritoDeck contributors"; the installer
  writes "PerritoDeck" to Apps & Features (`tauri.conf.json`'s `bundle.publisher`).
  `AppsAndFeaturesEntries` bridges the two. Changing `bundle.publisher` later means updating
  that entry in the next manifest.
