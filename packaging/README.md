# Packaging drafts

**The Flathub and AUR files are drafts: not built, linted or submitted.** They're starting
points for the Linux channels the Release workflow doesn't cover. The workflow already makes
the AppImage, `.deb` and `.rpm` (see [`docs/releasing.md`](../docs/releasing.md)).

Ready to send for v0.1.0 (prepared 2026-10-25, not submitted; the maintainer submits):
- **winget:** [`winget/manifests/p/PerritoDeck/PerritoDeck/0.1.0/`](winget/manifests/p/PerritoDeck/PerritoDeck/0.1.0/),
  passes `winget validate`; how to submit in [`winget/SUBMIT.md`](winget/SUBMIT.md).
- **F-Droid:** [`docs/fdroid/SUBMIT.md`](../docs/fdroid/SUBMIT.md), with the recipe next to it.

| Channel | Files | Builds from |
|---|---|---|
| Flathub | [`flatpak/`](flatpak/): manifest, `.desktop`, AppStream metainfo | source, offline |
| AUR (`perritodeck`) | [`aur/PKGBUILD`](aur/PKGBUILD) | source tarball of the tag |

Both need the repository to be public and a release tag.

## Flathub

1. Make a release, then generate the offline dependency lists from that tag with [flatpak-builder-tools](https://github.com/flatpak/flatpak-builder-tools):
   ```sh
   python3 cargo/flatpak-cargo-generator.py host/app/src-tauri/Cargo.lock -o cargo-sources.json
   flatpak-node-generator npm host/app/package-lock.json -o node-sources.json
   ```
2. Put the full commit hash next to `tag:` in the manifest.
3. Build and lint locally:
   ```sh
   flatpak run org.flatpak.Builder --force-clean --user --install --install-deps-from=flathub build dev.perritodeck.PerritoDeck.yml
   flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest dev.perritodeck.PerritoDeck.yml
   flatpak run --command=flatpak-builder-lint org.flatpak.Builder repo repo
   ```
4. Submit through a pull request to `flathub/flathub` (the `new-pr` branch), with `shared-modules` as a git submodule.

Open questions before submitting:
- **App ID.** `dev.perritodeck.PerritoDeck`, the same as the Tauri identifier. Flathub checks that the publisher controls `perritodeck.dev`.
- **Bundled plugins.** Outside an AppImage, Tauri looks for resources in `/usr/lib/PerritoDeck`, which is the runtime inside a Flatpak. The app likely needs a small fallback to `/app/lib/PerritoDeck` when `FLATPAK_ID` is set.
- **Running programs.** "Run command" keys and plugins (Python is in the GNOME runtime; Node.js is not) run inside the sandbox. Reaching the real system needs `flatpak-spawn --host` (`--talk-name=org.freedesktop.Flatpak`), which Flathub reviewers want justified.
- **Start at login.** The autostart plugin writes `~/.config/autostart`, which the sandbox can't see. The Background portal is the Flatpak way.
- **USB.** `adb` isn't in the sandbox, so USB tablets need adb on the host and `adb reverse` run there.
- **Input.** Wayland input uses the Remote Desktop portal, which works sandboxed. X11 XTest works through `fallback-x11`. Focus detection (X11 `WM_CLASS`) works only under X11, as outside Flatpak.
- **Rust version.** The pinned toolchain is 1.97, and the Flathub `rust-stable` extension must be at least that.

## AUR

The `perritodeck` package builds the tray app, the command-line host and fakedeck from the tag's source tarball.

```sh
cd packaging/aur
updpkgsums                       # fills in sha256sums from the real tarball
makepkg -si                      # or: extra-x86_64-build for a clean chroot
namcap PKGBUILD *.pkg.tar.zst
makepkg --printsrcinfo > .SRCINFO
```
Then push `PKGBUILD` and `.SRCINFO` to `ssh://aur@aur.archlinux.org/perritodeck.git`, which needs an AUR account with an SSH key. A `perritodeck-bin` package that repackages the release `.deb` is a quicker alternative, and the two can coexist.

The PKGBUILD reproduces the `.deb` layout: `/usr/bin/perritodeck-app`, plugins in `/usr/lib/PerritoDeck/plugins`, icons named `perritodeck-app`. **Unverified:** the paths assume Tauri's defaults for the binary name (the crate name, `perritodeck-app`) and the resource folder (`/usr/lib/<productName>`). Compare with the release `.deb` (`dpkg -c`) before publishing.
