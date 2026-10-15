# Screenshot

Takes a screenshot of the whole screen, an area you pick, or the window in front, and
saves it as a PNG named with the date and time (`Screenshot 2026-10-11 14.03.22.png`).
It can also copy the picture to the clipboard.

## Setup (Plugins page)

- **Save screenshots in**: a folder; `~` works. Empty means `Pictures/Screenshots` (on
  Windows the real Pictures folder, even if OneDrive moved it; on Linux the translated
  one from `user-dirs.dirs`, e.g. `~/Bilder`). It's created if missing.

Per system:

- **Windows**: nothing to install (uses Windows PowerShell 5.1, built in).
- **macOS**: macOS asks for **Screen Recording** permission the first time; allow it for
  PerritoDeck (or the app that started the host). "The window in front" also needs
  **Accessibility**; without it you click the window to capture instead.
- **Linux**: one of `grim` + `slurp` (Sway, Hyprland and other wlroots desktops),
  `spectacle` (KDE), `gnome-screenshot` (GNOME), `scrot` or ImageMagick's `import` (X11).
  Copying needs `wl-clipboard` (Wayland) or `xclip` (X11).

## Actions

| Action | Fields | Notes |
|---|---|---|
| Take a screenshot | What to capture (the whole screen, an area I pick, the window in front); also copy it to the clipboard | Picking an area waits for you on the computer; the key answers straight away and `last` changes when the file is saved. Pressing Esc cancels. |
| Open the screenshot folder | | |

On Windows, "an area I pick" opens Windows' own snipping overlay, which only puts the
picture on the clipboard: it isn't saved in the folder, and the key says so.
"The whole screen" covers every monitor on Windows, and the main display on macOS.

## Values

| Title | Shows |
|---|---|
| `{plugin:dev.perritodeck.screenshot:last}` | the last file's name, `–` before the first |

## Verified vs untested

- **Verified** on Linux X11 (Xvfb, Openbox, ImageMagick `import`, `xclip`): whole screen
  (a 1280×800 PNG), the window in front (found with `xdotool`), and copying the PNG to the
  clipboard, over the real plugin protocol.
- **Run on Windows 11** (2026-10-12, in the desktop app with a tablet, one monitor): the
  window in front, through the PowerShell script. Not tried on Windows yet: the whole
  screen, an area (Windows' snipping overlay), copying to the clipboard, and monitors
  set to different scaling (the plugin asks Windows for per-monitor DPI awareness for
  those).
- **Unit-tested only**: macOS (`screencapture` arguments), and on Linux `grim`/`slurp`,
  `spectacle`, `gnome-screenshot`, `scrot` and `wl-copy` (arguments only). Picking an area
  was tested with a fake program, not a real one.
- Known limits: GNOME on Wayland may block `gnome-screenshot` from capturing without a
  prompt, depending on the version.
