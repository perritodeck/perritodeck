# Windows and apps

Switch to an app with one press (or open it if it isn't running), minimize, maximize or
close windows, quit apps, show the desktop, and show which app is in front.

## Which app a key means

The **App** field matches, in this order:

1. a process name, exactly, without case or `.exe`: `firefox`, `Code`, `spotify.exe`;
2. on Linux, part of the window class (`Navigator.firefox`, `org.gnome.Nautilus`);
3. part of a window title: `YouTube`, `Inbox`.

## Setup

Nothing to fill in on the Plugins page. Per system:

- **Windows**: nothing to install.
- **macOS**: the first time, macOS asks whether PerritoDeck may control *System Events*;
  allow it. Window titles, minimize, maximize and close also need **System Settings →
  Privacy & Security → Accessibility** for PerritoDeck (or for the app that started the
  host, such as Terminal). Without it the key shows a message saying so.
- **Linux on X11**: install `wmctrl` (`sudo apt install wmctrl`); `xdotool` is used too
  if present (for minimizing). With only `xdotool`, everything but maximize works.
- **Linux on Wayland**: Sway (`swaymsg`) and Hyprland (`hyprctl`) work. GNOME and KDE on
  Wayland don't let other programs switch or move windows; there `wmctrl` only sees
  apps running under XWayland, and the log says so.

## Actions

| Action | Fields | Notes |
|---|---|---|
| Switch to an app | App; command to open it (optional) | Brings its window to the front. Pressed again while the app is in front, it goes to the app's next window. If nothing matches, it runs the command, or opens the app by name (macOS `open -a`, or a program of that name on the PATH). The command can be a program path, a command line, or on Windows a link such as `spotify:` or `ms-settings:`. |
| Minimize, maximize or close a window | What to do (minimize, maximize, restore, close); App (empty: the window in front) | Close is a polite request, like the X button: the app can still ask to save. |
| Quit an app | App | Closes all the app's windows politely (Windows, Linux) or asks it to quit (macOS, like ⌘Q). Warns if it isn't running. |
| Show the desktop | | Windows: Win+D. macOS: Mission Control's "show desktop". X11: toggles the "showing the desktop" mode. Not available on Sway or Hyprland. |

## Values

| Title | Shows |
|---|---|
| `{plugin:dev.perritodeck.windows:running:spotify}` | `on`/`off`, for a toggle to follow |
| `{plugin:dev.perritodeck.windows:focused_app}` | the app in front, e.g. `firefox`, `Code` |

Both are checked every 2 seconds. `running` is checked only for apps a key shows; on
macOS it matches app and program names only, not window titles.

## How it works

- Windows: `user32` through ctypes ([winapi.py](winapi.py)): EnumWindows (only windows
  Alt+Tab would show), SetForegroundWindow with the AttachThreadInput and Alt-key
  workaround for the foreground lock, ShowWindow, and WM_CLOSE.
- macOS: `osascript` and System Events.
- Linux: `wmctrl -lpx`, `xprop`/`xdotool`, `swaymsg -t get_tree`, `hyprctl clients -j`;
  process names from `/proc`.

## Verified vs untested

- **Verified** on Linux X11 (Xvfb with the Openbox window manager, `wmctrl` 1.07,
  `xdotool` 3): switching, going through an app's windows, maximize, restore, minimize,
  close, quit, show the desktop, launching a command, `running` and `focused_app`, over
  the real plugin protocol.
- **Run on Windows 11** (2026-10-12, in the desktop app with a tablet): Switch to an app,
  including opening it when it wasn't running, and Quit an app; an app the plugin opened
  kept running through a plugin restart. Not tried on Windows yet: minimize, maximize,
  restore, close and Show the desktop. Checked outside the app: with Calculator open,
  the window list and the window in front report `CalculatorApp.exe`, not the frame
  Store apps run in. On Windows the test suite also lists the real windows (read-only);
  the rest of it mocks the ctypes calls.
- **Unit-tested only** (no real system): macOS (AppleScript is checked for the arguments
  it gets, not run), Sway and Hyprland (parsed from sample JSON).
- Known limits: on Windows, Windows may still refuse to bring a window forward and flash
  its taskbar button instead (the key then warns). macOS "maximize" fills the main screen
  rather than entering full screen. Windows of elevated (administrator) apps can't be
  controlled unless the host runs elevated too.
