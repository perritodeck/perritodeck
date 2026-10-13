"""Screenshot: capture the whole screen, an area you pick, or the window in front,
save it as a PNG with a timestamped name, and optionally copy it to the clipboard.

Backends, picked by sys.platform:

- Windows: PowerShell 5.1 and System.Drawing (CopyFromScreen) for the screen and
  the window in front. An area goes through Windows' own snipping overlay
  (ms-screenclip:), which only puts the picture on the clipboard.
- macOS: screencapture (-x quiet, -i pick an area, -R the window's bounds).
- Linux: the first of grim (+ slurp), spectacle, gnome-screenshot, scrot or
  ImageMagick's import that suits the desktop; the clipboard through wl-copy or xclip.

Picking an area waits for you, so it runs in the background and the key answers
straight away; `last` changes once the file is saved.
"""

import base64
import datetime
import html
import json
import os
import re
import shutil
import subprocess
import sys
import threading

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()
NOTHING = "–"
TIMEOUT = 8  # seconds for a capture that doesn't wait for the user
PICK_TIMEOUT = 120  # seconds someone has to pick an area
CREATE_NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0x08000000)
MODES = ("screen", "region", "window")
MAX_ERROR = 80  # characters of an error message shown on a key


def short(message, limit=MAX_ERROR):
    """One line of at most `limit` characters, for a key's alert."""
    line = " ".join(str(message).split())
    return line if len(line) <= limit else line[: limit - 1] + "…"


def run(args, timeout=TIMEOUT, **kwargs):
    """Run a helper program; its stdout, or raise with what it said on stderr."""
    try:
        out = subprocess.run(
            args,
            stdin=kwargs.pop("stdin", subprocess.DEVNULL),
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=timeout,
            creationflags=CREATE_NO_WINDOW if sys.platform == "win32" else 0,
            **kwargs,
        )
    except FileNotFoundError:
        raise RuntimeError(f"{os.path.basename(args[0])} isn't installed")
    except subprocess.TimeoutExpired:
        raise RuntimeError(f"{os.path.basename(args[0])} didn't finish in {timeout} s")
    if out.returncode != 0:
        detail = (out.stderr or out.stdout or "").strip().splitlines()
        raise RuntimeError(detail[-1] if detail else f"{os.path.basename(args[0])} failed")
    return out.stdout


# --- Where files go ----------------------------------------------------------------


def xdg_pictures(home, config_dir):
    """XDG_PICTURES_DIR from user-dirs.dirs (it's often translated: ~/Bilder)."""
    try:
        with open(os.path.join(config_dir, "user-dirs.dirs"), encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if line.startswith("XDG_PICTURES_DIR="):
                    value = line.split("=", 1)[1].strip().strip('"')
                    value = value.replace("$HOME", home)
                    if value and value.rstrip("/") != home.rstrip("/"):
                        return value
    except OSError:
        pass
    return os.path.join(home, "Pictures")


def pictures_dir(platform=None, env=None):
    platform = platform or sys.platform
    env = os.environ if env is None else env
    home = os.path.expanduser("~")
    if platform == "win32":
        try:
            import winapi

            folder = winapi.pictures_folder()
            if folder:
                return folder
        except Exception:
            pass
        return os.path.join(env.get("USERPROFILE") or home, "Pictures")
    if platform == "darwin":
        return os.path.join(home, "Pictures")
    return xdg_pictures(home, env.get("XDG_CONFIG_HOME") or os.path.join(home, ".config"))


def folder(settings=None, platform=None):
    """The folder to save in: the setting (~ expanded), or Pictures/Screenshots."""
    settings = plugin.settings if settings is None else settings
    chosen = str(settings.get("folder") or "").strip()
    path = os.path.expandvars(os.path.expanduser(chosen)) if chosen else os.path.join(pictures_dir(platform), "Screenshots")
    os.makedirs(path, exist_ok=True)
    return path


def new_name(directory, now=None):
    """A free file name like "Screenshot 2026-10-11 14.03.22.png"."""
    now = now or datetime.datetime.now()
    base = "Screenshot " + now.strftime("%Y-%m-%d %H.%M.%S")
    name = base + ".png"
    n = 2
    while os.path.exists(os.path.join(directory, name)):
        name = f"{base} ({n}).png"
        n += 1
    return os.path.join(directory, name)


# --- The last picture ----------------------------------------------------------------

_last_lock = threading.Lock()


def remember(path):
    """Publish `last` and keep it for next time the plugin starts."""
    name = os.path.basename(path)
    with _last_lock:
        try:
            with open(os.path.join(plugin.data_dir, "last.txt"), "w", encoding="utf-8") as f:
                f.write(name)
        except OSError as e:
            plugin.log(f"couldn't remember the last screenshot: {e}", "warn")
        plugin.set_value("last", name)


def remembered():
    try:
        with open(os.path.join(plugin.data_dir, "last.txt"), encoding="utf-8") as f:
            return f.read().strip() or NOTHING
    except OSError:
        return NOTHING


# --- Windows ---------------------------------------------------------------------------

# Windows PowerShell 5.1. The mode, file and copy flag come in environment variables
# (see windows_env), so no path ever has to be quoted into the script: PowerShell ends
# a '…' string at ‘ ’ ‚ ‛ as well as at ', and folder names can contain those.
# The process must be DPI aware before System.Windows.Forms loads, or CopyFromScreen
# sees scaled-down coordinates on a 125 %/150 % display. Prints one line of JSON.
WINDOWS_SCRIPT = r"""
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$mode = $env:PERRITODECK_SHOT_MODE
$path = $env:PERRITODECK_SHOT_PATH
$copy = $env:PERRITODECK_SHOT_COPY -eq '1'
function Out-Json($data) {
    $json = ConvertTo-Json -InputObject $data -Compress
    $json = [regex]::Replace($json, '[^\x00-\x7F]', { param($m) '\u{0:x4}' -f [int][char]$m.Value })
    [Console]::Out.WriteLine($json)
}
try {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class PerritoShot {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr value);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr hWnd, int attribute, out RECT rect, int size);
}
'@
    # Per-monitor v2 (DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 = -4, Windows 10 1703+)
    # gives real pixels on every monitor when their scaling differs. Older Windows has
    # no such function (EntryPointNotFoundException): fall back to system awareness.
    $aware = $false
    try { $aware = [PerritoShot]::SetProcessDpiAwarenessContext([IntPtr](-4)) } catch { }
    if (-not $aware) { [void][PerritoShot]::SetProcessDPIAware() }
    Add-Type -AssemblyName System.Windows.Forms, System.Drawing
    if ($mode -eq 'window') {
        $hwnd = [PerritoShot]::GetForegroundWindow()
        if ($hwnd -eq [IntPtr]::Zero) { throw 'no window is in front' }
        $r = New-Object PerritoShot+RECT
        # DWMWA_EXTENDED_FRAME_BOUNDS (9): the visible frame, without the invisible resize border
        if ([PerritoShot]::DwmGetWindowAttribute($hwnd, 9, [ref]$r, 16) -ne 0) {
            [void][PerritoShot]::GetWindowRect($hwnd, [ref]$r)
        }
        $left = $r.Left; $top = $r.Top; $width = $r.Right - $r.Left; $height = $r.Bottom - $r.Top
    } else {
        $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
        $left = $b.X; $top = $b.Y; $width = $b.Width; $height = $b.Height
    }
    if ($width -le 0 -or $height -le 0) { throw 'the window has no size (is it minimized?)' }
    $bmp = New-Object System.Drawing.Bitmap $width, $height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($left, $top, 0, 0, $bmp.Size)
    $g.Dispose()
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    if ($copy) { [System.Windows.Forms.Clipboard]::SetImage($bmp) }
    $bmp.Dispose()
    Out-Json ([ordered]@{ ok = $true; width = $width; height = $height })
} catch {
    Out-Json ([ordered]@{ ok = $false; error = [string]$_.Exception.GetBaseException().Message })
}
"""


def windows_env(mode, path, copy):
    """This process's environment plus the variables WINDOWS_SCRIPT reads."""
    if mode not in ("screen", "window"):
        raise ValueError(f"unknown mode {mode}")
    return dict(os.environ, PERRITODECK_SHOT_MODE=mode, PERRITODECK_SHOT_PATH=path, PERRITODECK_SHOT_COPY="1" if copy else "0")


def error_text(stderr):
    """The message in what a helper wrote on stderr, or "": its last line, except
    for Windows PowerShell's CLIXML. With -EncodedCommand and stderr redirected,
    PowerShell writes an uncaught error as a "#< CLIXML" line, then XML with each
    line of the error in an <S S="Error"> element and line breaks written as
    _x000D__x000A_. The first of those lines is the message; the rest say where in
    the script it happened."""
    text = (stderr or "").strip()
    if not text.startswith("#< CLIXML"):
        lines = [line.strip() for line in text.splitlines() if line.strip()]
        return lines[-1] if lines else ""
    text = html.unescape("".join(re.findall(r'<S S="Error">(.*?)</S>', text, re.S)))
    text = re.sub(r"_x([0-9A-Fa-f]{4})_", lambda m: chr(int(m.group(1), 16)), text)
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    return lines[0] if lines else ""


def parse_ps(stdout, stderr=""):
    """The last JSON object line PowerShell printed."""
    for line in reversed((stdout or "").splitlines()):
        line = line.strip()
        if line.startswith("{"):
            try:
                data = json.loads(line)
            except ValueError:
                continue
            if isinstance(data, dict):
                return data
    raise RuntimeError(error_text(stderr) or "PowerShell gave no answer")


def windows_capture(mode, path, copy):
    if mode == "region":
        # Windows' own snipping overlay. It can't be told where to save.
        spawn(["explorer.exe", "ms-screenclip:"])
        return "Pick an area: Windows puts it on the clipboard, not in the folder"
    env = windows_env(mode, path, copy)
    encoded = base64.b64encode(WINDOWS_SCRIPT.encode("utf-16-le")).decode("ascii")
    try:
        out = subprocess.run(
            ["powershell.exe", "-NoProfile", "-NonInteractive", "-Sta", "-EncodedCommand", encoded],
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=TIMEOUT,
            env=env,
            creationflags=CREATE_NO_WINDOW,
        )
    except subprocess.TimeoutExpired:
        # str() of it is the whole command line, with the base64 script.
        raise RuntimeError(f"Windows didn't take the screenshot in {TIMEOUT} s") from None
    try:
        data = parse_ps(out.stdout, out.stderr)
    except RuntimeError as e:
        raise RuntimeError(short(e)) from None
    if not data.get("ok"):
        raise RuntimeError(short(data.get("error") or "the screenshot failed"))
    remember(path)
    return None


# --- macOS -------------------------------------------------------------------------------

# Position and size of the front app's first window: "x,y,w,h". Needs Accessibility.
MAC_BOUNDS_SCRIPT = """
tell application "System Events"
    set p to first application process whose frontmost is true
    set w to window 1 of p
    set {x, y} to position of w
    set {ww, hh} to size of w
end tell
return (x as text) & "," & (y as text) & "," & (ww as text) & "," & (hh as text)
"""

MAC_COPY_SCRIPT = """
on run argv
    set the clipboard to (read (POSIX file (item 1 of argv)) as «class PNGf»)
end run
"""


def parse_bounds(text):
    """"12,38,1280,800" → "12,38,1280,800" if it's four numbers with a size, else None."""
    parts = [p.strip() for p in (text or "").strip().split(",")]
    if len(parts) != 4:
        return None
    try:
        x, y, w, h = (int(float(p)) for p in parts)
    except ValueError:
        return None
    if w <= 0 or h <= 0:
        return None
    return f"{x},{y},{w},{h}"


def mac_copy(path):
    run(["osascript", "-e", MAC_COPY_SCRIPT, path])


def mac_capture(mode, path, copy):
    finish = lambda: (mac_copy(path) if copy else None)  # noqa: E731
    if mode == "screen":
        run(["screencapture", "-x", path])
        finish()
        remember(path)
        return None
    if mode == "window":
        try:
            bounds = parse_bounds(run(["osascript", "-e", MAC_BOUNDS_SCRIPT]))
        except RuntimeError as e:
            plugin.log(f"couldn't find the window in front ({e}); asking you to click one", "warn")
            bounds = None
        if bounds:
            run(["screencapture", "-x", "-R", bounds, path])
            finish()
            remember(path)
            return None
        pick(["screencapture", "-x", "-i", "-w", path], path, copy, mac_copy)
        return "Click the window to capture (allow Accessibility for PerritoDeck to skip this)"
    pick(["screencapture", "-x", "-i", path], path, copy, mac_copy)
    return None


# --- Linux ---------------------------------------------------------------------------------

LINUX_TOOLS = ("grim", "spectacle", "gnome-screenshot", "scrot", "import")


def linux_tool(env=None, which=shutil.which):
    """The screenshot program that suits this desktop, or None."""
    env = os.environ if env is None else env
    desktop = (env.get("XDG_CURRENT_DESKTOP") or "").lower()
    wayland = bool(env.get("WAYLAND_DISPLAY")) or env.get("XDG_SESSION_TYPE") == "wayland"
    order = []
    if "kde" in desktop:
        order.append("spectacle")
    if "gnome" in desktop or "unity" in desktop or "cinnamon" in desktop:
        order.append("gnome-screenshot")
    if wayland:
        # grim only works on wlroots desktops (Sway, Hyprland, river…); scrot and
        # import would only see X11 apps there.
        order += ["grim", "spectacle", "gnome-screenshot"]
    else:
        order += ["spectacle", "gnome-screenshot", "scrot", "import"]
    for tool in order:
        if which(tool):
            return tool
    return None


def wayland_window_geometry():
    """The focused window as grim's "x,y wxh", from Sway or Hyprland; None elsewhere."""
    if os.environ.get("SWAYSOCK") and shutil.which("swaymsg"):
        stack = [json.loads(run(["swaymsg", "-t", "get_tree"]))]
        while stack:
            node = stack.pop()
            if node.get("focused") and node.get("rect"):
                r = node["rect"]
                return f"{r['x']},{r['y']} {r['width']}x{r['height']}"
            stack.extend((node.get("nodes") or []) + (node.get("floating_nodes") or []))
    if os.environ.get("HYPRLAND_INSTANCE_SIGNATURE") and shutil.which("hyprctl"):
        data = json.loads(run(["hyprctl", "activewindow", "-j"]) or "{}")
        if data.get("at") and data.get("size"):
            (x, y), (w, h) = data["at"], data["size"]
            return f"{x},{y} {w}x{h}"
    return None


def x11_active_window():
    """The active X11 window id as a string, or None."""
    if shutil.which("xdotool"):
        try:
            return run(["xdotool", "getactivewindow"]).strip() or None
        except RuntimeError:
            return None
    if shutil.which("xprop"):
        word = run(["xprop", "-root", "_NET_ACTIVE_WINDOW"]).strip().rsplit(None, 1)[-1]
        return word if word.startswith("0x") and int(word, 16) else None
    return None


def linux_command(tool, mode, path):
    """(argv, waits for the user) to capture with `tool`."""
    if tool == "spectacle":
        flag = {"screen": "-f", "region": "-r", "window": "-a"}[mode]
        return ["spectacle", "-b", "-n", flag, "-o", path], mode == "region"
    if tool == "gnome-screenshot":
        flag = {"screen": [], "region": ["-a"], "window": ["-w"]}[mode]
        return ["gnome-screenshot", *flag, "-f", path], mode == "region"
    if tool == "scrot":
        flag = {"screen": [], "region": ["-s"], "window": ["-u"]}[mode]
        return ["scrot", *flag, path], mode == "region"
    if tool == "import":
        if mode == "screen":
            return ["import", "-window", "root", path], False
        if mode == "region":
            return ["import", path], True  # drag a rectangle, or click a window
        wid = x11_active_window()
        if not wid:
            raise RuntimeError("can't find the window in front (install xdotool)")
        return ["import", "-window", wid, path], False
    if tool == "grim":
        if mode == "screen":
            return ["grim", path], False
        if mode == "window":
            geometry = wayland_window_geometry()
            if not geometry:
                raise RuntimeError("can't find the window in front on this desktop")
            return ["grim", "-g", geometry, path], False
        if not shutil.which("slurp"):
            raise RuntimeError("picking an area needs slurp (e.g. sudo apt install slurp)")
        return ["sh", "-c", 'g="$(slurp)" || exit 1; exec grim -g "$g" "$1"', "grim", path], True
    raise RuntimeError(f"unknown screenshot tool {tool}")


def linux_copy(path):
    """Put the PNG on the clipboard. wl-copy and xclip stay running to serve it, so
    their output must not be a pipe we wait on."""
    if (os.environ.get("WAYLAND_DISPLAY")) and shutil.which("wl-copy"):
        args = ["wl-copy", "--type", "image/png"]
    elif shutil.which("xclip"):
        args = ["xclip", "-selection", "clipboard", "-t", "image/png", "-i"]
    else:
        raise RuntimeError("saved, but copying needs wl-clipboard or xclip")
    with open(path, "rb") as f:
        proc = subprocess.Popen(args, stdin=f, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    try:
        if proc.wait(timeout=TIMEOUT) != 0:
            raise RuntimeError(f"{args[0]} couldn't copy the picture")
    except subprocess.TimeoutExpired:
        pass  # still serving the clipboard: that's fine


def linux_capture(mode, path, copy):
    if not os.environ.get("DISPLAY") and not os.environ.get("WAYLAND_DISPLAY"):
        raise RuntimeError("there's no desktop session to capture (no X11 or Wayland display)")
    tool = linux_tool()
    if not tool:
        raise RuntimeError("install a screenshot tool: grim and slurp, spectacle, gnome-screenshot or scrot")
    args, waits = linux_command(tool, mode, path)
    if waits:
        pick(args, path, copy, linux_copy)
        return None
    run(args)
    if not os.path.exists(path):
        raise RuntimeError(f"{tool} didn't save the picture")
    remember(path)
    if copy:
        try:
            linux_copy(path)
        except RuntimeError as e:
            return str(e)
    return None


# --- Shared --------------------------------------------------------------------------------


def spawn(args):
    kwargs = dict(stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, close_fds=True)
    if sys.platform == "win32":
        kwargs["creationflags"] = CREATE_NO_WINDOW
    else:
        kwargs["start_new_session"] = True
    return subprocess.Popen(args, **kwargs)


_picking = threading.Lock()


def pick(args, path, copy, copier):
    """Run a capture that waits for the user, in the background."""
    if not _picking.acquire(blocking=False):
        raise RuntimeError("already waiting for you to pick an area")
    proc = subprocess.Popen(args, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)

    def wait():
        try:
            try:
                _, err = proc.communicate(timeout=PICK_TIMEOUT)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.communicate()
                plugin.log("gave up waiting for an area to be picked", "warn")
                return
            if not os.path.exists(path) or os.path.getsize(path) == 0:
                plugin.log("no screenshot taken (cancelled)" + (f": {err.strip()}" if err and err.strip() else ""))
                return
            remember(path)
            if copy:
                try:
                    copier(path)
                except Exception as e:
                    plugin.log(f"couldn't copy the screenshot: {e}", "warn")
        finally:
            _picking.release()

    threading.Thread(target=wait, daemon=True).start()


def capturer(platform=None):
    platform = platform or sys.platform
    if platform == "win32":
        return windows_capture
    if platform == "darwin":
        return mac_capture
    return linux_capture


@plugin.action("capture")
def capture(settings):
    mode = str(settings.get("what") or "screen")
    if mode not in MODES:
        raise ValueError(f"unknown capture {mode}")
    copy = bool(settings.get("copy"))
    try:
        directory = folder()
    except OSError as e:
        raise RuntimeError(f"can't use the screenshot folder: {e.strerror or e}")
    return capturer()(mode, new_name(directory), copy)


@plugin.action("open-folder")
def open_folder(settings):
    directory = folder()
    if sys.platform == "win32":
        os.startfile(directory)  # noqa: S606
    elif sys.platform == "darwin":
        run(["open", directory])
    else:
        spawn(["xdg-open", directory])


@plugin.on_start
def started():
    plugin.set_value("last", remembered())


if __name__ == "__main__":
    plugin.run()
