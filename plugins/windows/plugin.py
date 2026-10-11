"""Windows and apps: switch to an app (or open it), minimize, maximize, close or quit
it, show the desktop, and show which app is in front.

One backend per desktop, picked when the plugin starts:

- Windows: user32 through ctypes (winapi.py).
- macOS: osascript and System Events (needs Accessibility for window titles,
  minimize, maximize and close).
- Linux on X11: wmctrl (best), or xdotool.
- Linux on Wayland: swaymsg (Sway) or hyprctl (Hyprland). GNOME and KDE on Wayland
  don't let other programs move windows; wmctrl then only sees X11 (XWayland) apps.

Which app a key means: the app field matches a process name exactly ("firefox",
"firefox.exe", "Code"), or a window's class (Linux) or title containing it.
"""

import collections
import json
import os
import shlex
import shutil
import subprocess
import sys
import threading

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()
NOTHING = "–"
TIMEOUT = 4  # seconds for any one helper program
POLL_SECONDS = 2
CREATE_NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0x08000000)
DETACHED_PROCESS = 0x00000008
CREATE_NEW_PROCESS_GROUP = 0x00000200

# id: what the window system calls it; app: process name; cls: window class (Linux).
Window = collections.namedtuple("Window", "id pid app title cls")
Window.__new__.__defaults__ = ("",)


class Unsupported(RuntimeError):
    pass


def norm(name):
    """Process names compared without case or .exe/.app: "Firefox.exe" → "firefox"."""
    name = (name or "").strip().lower()
    for ext in (".exe", ".app"):
        if name.endswith(ext):
            name = name[: -len(ext)]
    return name


def rank(window, query):
    """How well a window matches what a key asks for: 0 best, None no match."""
    q = norm(query)
    if not q:
        return None
    if norm(window.app) == q:
        return 0
    if window.cls and q in window.cls.lower():
        return 1
    if q in (window.title or "").lower():
        return 2
    return None


def best_matches(windows, query):
    """Windows matching query, best first, keeping the window system's order (front
    to back where it gives one) within a rank."""
    ranked = [(r, i, w) for i, w in enumerate(windows) for r in [rank(w, query)] if r is not None]
    ranked.sort(key=lambda x: (x[0], x[1]))
    return [w for _, _, w in ranked]


def run(args, timeout=TIMEOUT, check=True):
    """Run a helper program; its stdout, or raise with what it said on stderr."""
    try:
        out = subprocess.run(
            args,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=timeout,
            creationflags=CREATE_NO_WINDOW if sys.platform == "win32" else 0,
        )
    except FileNotFoundError:
        raise RuntimeError(f"{args[0]} isn't installed")
    except subprocess.TimeoutExpired:
        raise RuntimeError(f"{os.path.basename(args[0])} didn't answer in {timeout} s")
    if check and out.returncode != 0:
        detail = (out.stderr or out.stdout).strip().splitlines()
        raise RuntimeError(detail[-1] if detail else f"{os.path.basename(args[0])} failed")
    return out.stdout


def spawn(args, shell=False):
    """Start a program and let it run on after us, with none of our pipes."""
    kwargs = dict(stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, close_fds=True)
    if sys.platform == "win32":
        kwargs["creationflags"] = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    else:
        kwargs["start_new_session"] = True
    return subprocess.Popen(args, shell=shell, **kwargs)


def launch(command, platform=None):
    """Open an app from the "command to open it" field.

    A plain name or a path to a file is opened the way double-clicking would
    (Windows: ShellExecute, so "spotify:" and App Paths names work too; macOS:
    open -a). Anything else is run as a command line."""
    platform = platform or sys.platform
    command = command.strip()
    if platform == "win32":
        if os.path.exists(command) or " " not in command:
            try:
                os.startfile(command)  # noqa: S606 (the user's own launch command)
                return
            except OSError:
                if os.path.exists(command):
                    raise
        spawn(command)  # CreateProcess parses the command line itself
        return
    if platform == "darwin":
        if command.endswith(".app") or ("/" not in command and " " not in command):
            run(["open", "-a", command])
            return
        if os.path.exists(command):
            run(["open", command])
            return
    spawn(shlex.split(command))


# --- Linux helpers -------------------------------------------------------------


def proc_names(root="/proc"):
    """Names of running processes from /proc: each comm (cut to 15 characters by the
    kernel) and the first word of its command line, lower case."""
    names = set()
    try:
        entries = os.listdir(root)
    except OSError:
        return names
    for pid in entries:
        if not pid.isdigit():
            continue
        try:
            with open(os.path.join(root, pid, "comm"), encoding="utf-8", errors="replace") as f:
                names.add(norm(f.read().strip()))
            with open(os.path.join(root, pid, "cmdline"), "rb") as f:
                argv0 = f.read().split(b"\0", 1)[0].decode("utf-8", "replace")
            if argv0:
                names.add(norm(os.path.basename(argv0.split(" ", 1)[0])))
        except OSError:
            continue
    names.discard("")
    return names


def comm(pid, root="/proc"):
    try:
        with open(os.path.join(root, str(pid), "comm"), encoding="utf-8", errors="replace") as f:
            return f.read().strip()
    except (OSError, ValueError):
        return ""


def parse_wmctrl(output, comm_of=None):
    """Windows from `wmctrl -lpx`: id, desktop, pid, WM_CLASS, client host, title.

        0x03a00003  0 4321   Navigator.firefox     laptop Mozilla Firefox

    Desktop -1 means the window is on every desktop (panels, the desktop itself)."""
    comm_of = comm_of or comm
    windows = []
    for line in output.splitlines():
        parts = line.split(None, 4)
        if len(parts) < 4:
            continue
        wid, desktop, pid, cls = parts[:4]
        rest = parts[4] if len(parts) > 4 else ""
        title = rest.split(None, 1)[1] if " " in rest else ""
        try:
            wid_int = int(wid, 16)
            pid_int = int(pid)
            desktop_int = int(desktop)
        except ValueError:
            continue
        if desktop_int < 0:
            continue
        windows.append(Window(wid_int, pid_int, comm_of(pid_int) if pid_int else "", title, cls))
    return windows


def parse_active_xprop(output):
    """The active window id from `xprop -root _NET_ACTIVE_WINDOW`, or None."""
    # _NET_ACTIVE_WINDOW(WINDOW): window id # 0x3a00003
    word = output.strip().rsplit(None, 1)[-1] if output.strip() else ""
    try:
        value = int(word, 16)
    except ValueError:
        return None
    return value or None


def parse_showing_desktop(output):
    """True/False from `wmctrl -m` ("showing the desktop" mode: ON), None if unknown."""
    for line in output.splitlines():
        if "showing the desktop" in line.lower():
            return line.strip().upper().endswith("ON")
    return None


def sway_windows(tree):
    """(windows, focused window) from `swaymsg -t get_tree` JSON."""
    found, focused = [], None
    stack = [tree]
    while stack:
        node = stack.pop()
        if node.get("pid") and node.get("type") in ("con", "floating_con"):
            props = node.get("window_properties") or {}
            cls = node.get("app_id") or props.get("class") or ""
            w = Window(node.get("id"), node.get("pid"), comm(node.get("pid")) or cls, node.get("name") or "", cls)
            found.append(w)
            if node.get("focused"):
                focused = w
        stack.extend(reversed((node.get("nodes") or []) + (node.get("floating_nodes") or [])))
    return found, focused


def hypr_windows(clients):
    """Windows from `hyprctl clients -j`, most recently focused first."""
    rows = [c for c in clients if c.get("mapped", True) and c.get("pid", 0) > 0]
    rows.sort(key=lambda c: c.get("focusHistoryID", 99))
    return [Window(c.get("address"), c.get("pid"), comm(c.get("pid")) or c.get("class", ""), c.get("title", ""), c.get("class", "")) for c in rows]


# --- Backends -------------------------------------------------------------------


class Backend:
    """What every backend can do. Window operations take a Window from windows()."""

    name = "none"

    def windows(self):
        return []

    def active(self):
        return None

    def find(self, query):
        return best_matches(self.windows(), query)

    def running(self, query):
        q = norm(query)
        return q in self.process_names() or bool(self.find(query))

    def process_names(self):
        return proc_names()

    def activate(self, w):
        raise Unsupported(self.why())

    def minimize(self, w):
        raise Unsupported(self.why())

    def maximize(self, w):
        raise Unsupported(self.why())

    def restore(self, w):
        raise Unsupported(self.why())

    def close(self, w):
        raise Unsupported(self.why())

    def quit(self, query, matches):
        """Close every window of the apps that match, as their X buttons would."""
        pids = {w.pid for w in matches if rank(w, query) == rank(matches[0], query)}
        targets = [w for w in self.windows() if w.pid in pids] or matches
        for w in targets:
            self.close(w)

    def show_desktop(self):
        raise Unsupported(self.why())

    def why(self):
        return "this desktop doesn't let PerritoDeck control windows"


class WindowsBackend(Backend):
    name = "windows"

    def __init__(self):
        import winapi  # only on Windows: ctypes.WinDLL

        self.api = winapi

    def windows(self):
        return [Window(h, pid, exe, title) for h, pid, exe, title in self.api.list_windows()]

    def active(self):
        hwnd = self.api.foreground()
        if not hwnd:
            return None
        h, pid, exe, title = self.api.info(hwnd)
        return Window(h, pid, exe, title)

    def process_names(self):
        return {norm(n) for n in self.api.process_names()}

    def activate(self, w):
        if not self.api.focus(w.id):
            return "Windows didn't let the window come to the front; it may be flashing in the taskbar"

    def minimize(self, w):
        self.api.show(w.id, self.api.SW_MINIMIZE)

    def maximize(self, w):
        self.api.show(w.id, self.api.SW_MAXIMIZE)

    def restore(self, w):
        self.api.show(w.id, self.api.SW_RESTORE)

    def close(self, w):
        self.api.close(w.id)

    def show_desktop(self):
        self.api.show_desktop()


MAC_ACCESS = (
    "macOS blocked it: allow PerritoDeck (or the app that started it) under System Settings → "
    "Privacy & Security → Accessibility"
)
MAC_AUTOMATION = (
    "macOS blocked it: allow PerritoDeck to control System Events under System Settings → "
    "Privacy & Security → Automation"
)

# Every app with a window in the Dock, one line per window:
#   process <US> pid <US> window number (0: none) <US> title
# A first line "!access" means macOS refused window titles (no Accessibility).
MAC_LIST_SCRIPT = """
set sep to character id 31
set out to ""
set denied to false
tell application "System Events"
    repeat with p in (every application process whose background only is false)
        set pn to name of p
        set pid to unix id of p
        set wins to {}
        try
            set wins to name of every window of p
        on error errMsg number errNum
            if errNum is -1719 or errNum is -25211 then set denied to true
        end try
        if (count of wins) is 0 then
            set out to out & pn & sep & pid & sep & "0" & sep & "" & linefeed
        else
            repeat with i from 1 to count of wins
                set t to item i of wins
                if t is missing value then set t to ""
                set out to out & pn & sep & pid & sep & i & sep & t & linefeed
            end repeat
        end if
    end repeat
end tell
if denied then set out to "!access" & linefeed & out
return out
"""

# The front app, then every process name: front <US> name <US> name …
MAC_POLL_SCRIPT = """
set sep to character id 31
tell application "System Events"
    set frontName to ""
    try
        set frontName to name of first application process whose frontmost is true
    end try
    set procNames to name of every application process
end tell
set out to frontName
repeat with n in procNames
    set out to out & sep & n
end repeat
return out
"""

# argv: process name ("" = the front app), window number (0 = the first), what to do.
MAC_WINDOW_SCRIPT = """
on run argv
    set pn to item 1 of argv
    set idx to (item 2 of argv) as integer
    set act to item 3 of argv
    if act is "maximize" then
        tell application "Finder" to set b to bounds of window of desktop
    end if
    tell application "System Events"
        if pn is "" then set pn to name of first application process whose frontmost is true
        set p to application process pn
        if act is "focus" then
            set frontmost of p to true
            if idx > 0 then perform action "AXRaise" of window idx of p
            return "ok"
        end if
        if act is "quit" then
            return bundle identifier of p
        end if
        if idx < 1 then set idx to 1
        if (count of windows of p) < idx then return "nowindow"
        set w to window idx of p
        if act is "minimize" then
            set value of attribute "AXMinimized" of w to true
        else if act is "restore" then
            set value of attribute "AXMinimized" of w to false
            set frontmost of p to true
            perform action "AXRaise" of w
        else if act is "close" then
            click (first button of w whose subrole is "AXCloseButton")
        else if act is "maximize" then
            set position of w to {item 1 of b, item 2 of b}
            set size of w to {(item 3 of b) - (item 1 of b), (item 4 of b) - (item 2 of b)}
        end if
    end tell
    return "ok"
end run
"""

MAC_QUIT_SCRIPT = """
on run argv
    tell application id (item 1 of argv) to quit
    return "ok"
end run
"""

MISSION_CONTROL = (
    "/System/Applications/Mission Control.app/Contents/MacOS/Mission Control",
    "/Applications/Mission Control.app/Contents/MacOS/Mission Control",
)


def mac_error(message):
    """Turn osascript's error text into something a person can act on."""
    if "-1719" in message or "-25211" in message or "assistive access" in message:
        return MAC_ACCESS
    if "-1743" in message or "Not authorized" in message:
        return MAC_AUTOMATION
    return message


def parse_mac_list(output):
    """(windows, denied) from MAC_LIST_SCRIPT. Window ids are (process, number)."""
    windows, denied = [], False
    for line in output.splitlines():
        if line == "!access":
            denied = True
            continue
        parts = line.split("\x1f")
        if len(parts) != 4:
            continue
        name, pid, idx, title = parts
        try:
            windows.append(Window((name, int(idx)), int(pid), name, title))
        except ValueError:
            continue
    return windows, denied


def parse_mac_poll(output):
    """(front app name, set of process names) from MAC_POLL_SCRIPT."""
    parts = output.rstrip("\r\n").split("\x1f")
    return parts[0], {norm(n) for n in parts[1:] if n}


class MacBackend(Backend):
    name = "macos"

    def __init__(self):
        self.warned = False

    def osa(self, script, *args):
        try:
            return run(["osascript", "-e", script, *args]).rstrip("\r\n")
        except RuntimeError as e:
            raise RuntimeError(mac_error(str(e)))

    def windows(self):
        windows, denied = parse_mac_list(self.osa(MAC_LIST_SCRIPT))
        if denied and not self.warned:
            self.warned = True
            plugin.log(MAC_ACCESS + " (window titles can't be matched until then)", "warn")
        return windows

    def poll(self):
        return parse_mac_poll(self.osa(MAC_POLL_SCRIPT))

    def active(self):
        front, _ = self.poll()
        return Window((front, 0), 0, front, "") if front else None

    def process_names(self):
        names = self.poll()[1]
        try:  # executables too, e.g. "firefox" for Firefox; needs no permission
            names |= {norm(os.path.basename(n)) for n in run(["ps", "-axco", "comm="]).splitlines() if n.strip()}
        except RuntimeError:
            pass
        return names

    def window_op(self, w, act):
        name, idx = w.id if w is not None else ("", 0)
        if self.osa(MAC_WINDOW_SCRIPT, name, str(idx), act) == "nowindow":
            return f"{name or 'the app in front'} has no window"

    def activate(self, w):
        return self.window_op(w, "focus")

    def minimize(self, w):
        return self.window_op(w, "minimize")

    def maximize(self, w):
        return self.window_op(w, "maximize")

    def restore(self, w):
        return self.window_op(w, "restore")

    def close(self, w):
        return self.window_op(w, "close")

    def quit(self, query, matches):
        for name in sorted({w.app for w in matches}):
            bundle = self.osa(MAC_WINDOW_SCRIPT, name, "0", "quit")
            if not bundle or bundle == "missing value":
                raise RuntimeError(f"{name} can't be asked to quit (it isn't a regular app)")
            self.osa(MAC_QUIT_SCRIPT, bundle)

    def show_desktop(self):
        for path in MISSION_CONTROL:
            if os.path.exists(path):
                spawn([path, "1"])  # Mission Control's own "show desktop"
                return
        raise RuntimeError("couldn't find Mission Control to show the desktop")


class WmctrlBackend(Backend):
    """X11 through wmctrl, with xprop/xdotool for the active window and minimizing."""

    name = "wmctrl"

    def __init__(self, wayland=False):
        self.wayland = wayland
        self.xdotool = shutil.which("xdotool")
        self.xprop = shutil.which("xprop")

    def windows(self):
        return parse_wmctrl(run(["wmctrl", "-lpx"]))

    def active(self):
        wid = None
        if self.xprop:
            wid = parse_active_xprop(run([self.xprop, "-root", "_NET_ACTIVE_WINDOW"]))
        elif self.xdotool:
            try:
                wid = int(run([self.xdotool, "getactivewindow"]).strip())
            except (ValueError, RuntimeError):
                wid = None
        if wid is None:
            return None
        for w in self.windows():
            if w.id == wid:
                return w
        return None

    def wm(self, *args):
        run(["wmctrl", *args])

    def activate(self, w):
        self.wm("-i", "-a", hex(w.id))

    def minimize(self, w):
        if self.xdotool:
            run([self.xdotool, "windowminimize", str(w.id)])
        else:
            self.wm("-i", "-r", hex(w.id), "-b", "add,hidden")

    def maximize(self, w):
        self.wm("-i", "-r", hex(w.id), "-b", "add,maximized_vert,maximized_horz")

    def restore(self, w):
        self.wm("-i", "-r", hex(w.id), "-b", "remove,maximized_vert,maximized_horz")
        self.wm("-i", "-a", hex(w.id))  # also brings a minimized window back

    def close(self, w):
        self.wm("-i", "-c", hex(w.id))

    def show_desktop(self):
        showing = parse_showing_desktop(run(["wmctrl", "-m"]))
        self.wm("-k", "off" if showing else "on")


class XdotoolBackend(Backend):
    """X11 with only xdotool: searches instead of a full window list."""

    name = "xdotool"

    def x(self, *args, check=True):
        return run(["xdotool", *args], check=check)

    def window(self, wid):
        wid = int(wid)
        title = self.x("getwindowname", str(wid), check=False).strip()
        try:
            pid = int(self.x("getwindowpid", str(wid), check=False).strip() or 0)
        except ValueError:
            pid = 0
        return Window(wid, pid, comm(pid) if pid else "", title)

    def find(self, query):
        import re

        pattern = re.escape(query.strip())
        ids = []
        for how in ("--classname", "--class", "--name"):
            out = self.x("search", "--onlyvisible", how, pattern, check=False)
            ids.extend(i for i in out.split() if i.isdigit() and i not in ids)
        return best_matches([self.window(i) for i in ids[:20]], query)

    def active(self):
        try:
            return self.window(self.x("getactivewindow").strip())
        except (ValueError, RuntimeError):
            return None

    def activate(self, w):
        self.x("windowactivate", str(w.id))

    def minimize(self, w):
        self.x("windowminimize", str(w.id))

    def maximize(self, w):
        raise Unsupported("maximizing needs wmctrl (e.g. sudo apt install wmctrl)")

    def restore(self, w):
        self.x("windowactivate", str(w.id))

    def close(self, w):
        try:
            self.x("windowclose", str(w.id))  # xdotool 3.2021 and later
        except RuntimeError:
            raise Unsupported("closing windows needs wmctrl or a newer xdotool")

    def quit(self, query, matches):
        for w in matches:
            if rank(w, query) == rank(matches[0], query):
                self.close(w)

    def show_desktop(self):
        self.x("key", "super+d")


class SwayBackend(Backend):
    name = "sway"

    def tree(self):
        return sway_windows(json.loads(run(["swaymsg", "-t", "get_tree"])))

    def windows(self):
        return self.tree()[0]

    def active(self):
        return self.tree()[1]

    def cmd(self, w, command):
        run(["swaymsg", f"[con_id={int(w.id)}] {command}"])

    def activate(self, w):
        self.cmd(w, "focus")

    def minimize(self, w):
        self.cmd(w, "move scratchpad")  # Sway has no minimize; the scratchpad is closest

    def maximize(self, w):
        self.cmd(w, "fullscreen enable")

    def restore(self, w):
        self.cmd(w, "fullscreen disable")

    def close(self, w):
        self.cmd(w, "kill")  # asks the window to close, like its X button

    def why(self):
        return "Sway has no desktop to show"


class HyprlandBackend(Backend):
    name = "hyprland"

    def windows(self):
        return hypr_windows(json.loads(run(["hyprctl", "clients", "-j"])))

    def active(self):
        data = json.loads(run(["hyprctl", "activewindow", "-j"]) or "{}")
        if not data or not data.get("address"):
            return None
        return hypr_windows([data])[0] if data.get("pid", 0) > 0 else None

    def dispatch(self, *args):
        run(["hyprctl", "dispatch", *args])

    def activate(self, w):
        self.dispatch("focuswindow", f"address:{w.id}")

    def minimize(self, w):
        self.dispatch("movetoworkspacesilent", f"special:minimized,address:{w.id}")

    def maximize(self, w):
        self.activate(w)
        self.dispatch("fullscreen", "1")  # 1 = maximize, keeping gaps and bars; toggles

    def restore(self, w):
        self.dispatch("movetoworkspace", f"e+0,address:{w.id}")
        self.activate(w)

    def close(self, w):
        self.dispatch("closewindow", f"address:{w.id}")

    def why(self):
        return "Hyprland has no desktop to show"


class NoBackend(Backend):
    def __init__(self, reason):
        self.reason = reason

    def why(self):
        return self.reason


def pick_backend(platform=None, env=None, which=shutil.which):
    platform = platform or sys.platform
    env = os.environ if env is None else env
    if platform == "win32":
        return WindowsBackend()
    if platform == "darwin":
        return MacBackend()
    wayland = bool(env.get("WAYLAND_DISPLAY")) or env.get("XDG_SESSION_TYPE") == "wayland"
    if wayland:
        if env.get("SWAYSOCK") and which("swaymsg"):
            return SwayBackend()
        if env.get("HYPRLAND_INSTANCE_SIGNATURE") and which("hyprctl"):
            return HyprlandBackend()
    if env.get("DISPLAY") and which("wmctrl"):
        return WmctrlBackend(wayland=wayland)
    if env.get("DISPLAY") and which("xdotool"):
        return XdotoolBackend()
    if wayland:
        return NoBackend(
            "this Wayland desktop doesn't let apps switch or move windows "
            "(works on X11, Sway and Hyprland)"
        )
    return NoBackend("install wmctrl to control windows (e.g. sudo apt install wmctrl)")


_backend = None
_backend_lock = threading.Lock()


def backend():
    global _backend
    with _backend_lock:
        if _backend is None:
            _backend = pick_backend()
            if isinstance(_backend, WmctrlBackend) and _backend.wayland:
                plugin.log("on Wayland wmctrl only sees X11 apps; native Wayland windows can't be controlled", "warn")
            elif isinstance(_backend, NoBackend):
                plugin.log(_backend.why(), "warn")
        return _backend


# --- Actions ---------------------------------------------------------------------


def need(settings, field="app", label="the app or window"):
    value = str(settings.get(field) or "").strip()
    if not value:
        raise ValueError(f"set {label} first")
    return value


@plugin.action("focus")
def focus(settings):
    app = need(settings)
    command = str(settings.get("launch") or "").strip()
    b = backend()
    try:
        matches = b.find(app)
    except Unsupported:
        matches = []
    if matches:
        target = matches[0]
        current = b.active() if len(matches) > 1 else None
        if current is not None:  # already in front: cycle through the app's windows
            ids = [w.id for w in matches]
            if current.id in ids:
                target = matches[(ids.index(current.id) + 1) % len(matches)]
        return b.activate(target)
    if command:
        launch(command)
        return None
    if sys.platform == "darwin":
        launch(app)  # open -a: opens it, or brings it forward
        return None
    found = shutil.which(app)
    if found:
        spawn([found])
        return None
    if isinstance(b, NoBackend):
        raise RuntimeError(b.why())
    raise RuntimeError(f"no window matches “{app}”; add a command to open it")


WINDOW_OPS = ("minimize", "maximize", "restore", "close")


@plugin.action("window")
def window(settings):
    op = str(settings.get("do") or "minimize")
    if op not in WINDOW_OPS:
        raise ValueError(f"unknown window action {op}")
    app = str(settings.get("app") or "").strip()
    b = backend()
    if app:
        matches = b.find(app)
        if not matches:
            raise RuntimeError(f"no window matches “{app}”")
        target = matches[0]
    elif isinstance(b, MacBackend):
        target = None  # the script finds the front app itself
    else:
        target = b.active()
        if target is None:
            raise RuntimeError(b.why() if isinstance(b, NoBackend) else "no window is in front")
    return getattr(b, op)(target)


@plugin.action("quit")
def quit_app(settings):
    app = need(settings, label="the app to quit")
    b = backend()
    matches = b.find(app)
    if not matches:
        return f"“{app}” isn't running"
    b.quit(app, matches)
    return None


@plugin.action("desktop")
def desktop(settings):
    backend().show_desktop()


# --- Values ----------------------------------------------------------------------

_published = {}


def publish(value_id, value, arg=None):
    key = value_id if arg is None else f"{value_id}:{arg}"
    if _published.get(key) != value:
        _published[key] = value
        plugin.set_value(value_id, value, arg=arg)


def friendly(window):
    """What the focused_app value shows: the process name without .exe."""
    if window is None:
        return NOTHING
    cls = window.cls.split(".", 1)[-1] if window.cls else ""  # WM_CLASS "instance.Class"
    name = window.app or cls or window.title
    if name.lower().endswith(".exe"):
        name = name[:-4]
    return name or NOTHING


@plugin.every(POLL_SECONDS)
def poll():
    b = backend()
    try:
        publish("focused_app", friendly(b.active()))
    except Exception as e:  # never stop polling; say why once
        if _published.get("focused_app") != NOTHING:
            plugin.log(f"couldn't tell which app is in front: {e}", "warn")
        publish("focused_app", NOTHING)
    apps = plugin.watched("running")
    if not apps:
        return
    try:
        names = b.process_names()
        windows = None
        for app in apps:
            on = norm(app) in names
            if not on:
                if windows is None:
                    try:
                        windows = b.windows()
                    except Exception:
                        windows = []
                on = bool(best_matches(windows, app))
            publish("running", "on" if on else "off", arg=app)
    except Exception as e:
        plugin.log(f"couldn't list running apps: {e}", "warn")
        for app in apps:
            publish("running", NOTHING, arg=app)


@plugin.on_watch
def watching(_values):
    for key in [k for k in _published if k.startswith("running:")]:
        if key[len("running:"):] not in plugin.watched("running"):
            del _published[key]


if __name__ == "__main__":
    plugin.run()
