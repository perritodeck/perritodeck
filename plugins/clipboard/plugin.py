"""Clipboard snippets: copy or paste a piece of text, and keep numbered slots you can
save the clipboard into and paste back later.

Pasting puts the text on the clipboard, presses the paste shortcut in the app in
front, and (if the key says so) puts the old clipboard back a moment later.

Backends, picked by sys.platform:

- Windows: PowerShell Get-Clipboard/Set-Clipboard; the shortcut through keybd_event
  (winapi.py).
- macOS: pbpaste/pbcopy; the shortcut through osascript and System Events (needs
  Accessibility).
- Linux: wl-paste/wl-copy on Wayland, else xclip or xsel; the shortcut through wtype,
  ydotool or xdotool.

Only text is handled: if the clipboard held a picture or files, it can't be put back.
"""

import base64
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()
NOTHING = "–"
TIMEOUT = 5  # seconds for any one helper program
PREVIEW = 20  # characters a slot shows
COMBOS = ("ctrl+v", "ctrl+shift+v", "shift+insert")
CREATE_NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0x08000000)
UTF8_ENV = {"LANG": "en_US.UTF-8", "LC_ALL": "en_US.UTF-8"}


def run(args, data=None, env=None, check=True):
    """Run a helper program, feeding it data (str); its stdout as text."""
    try:
        out = subprocess.run(
            args,
            input=data.encode("utf-8") if data is not None else None,
            stdin=None if data is not None else subprocess.DEVNULL,
            capture_output=True,
            timeout=TIMEOUT,
            env=env,
            creationflags=CREATE_NO_WINDOW if sys.platform == "win32" else 0,
        )
    except FileNotFoundError:
        raise RuntimeError(f"{os.path.basename(args[0])} isn't installed")
    except subprocess.TimeoutExpired:
        raise RuntimeError(f"{os.path.basename(args[0])} didn't answer in {TIMEOUT} s")
    if check and out.returncode != 0:
        detail = out.stderr.decode("utf-8", "replace").strip().splitlines()
        raise RuntimeError(detail[-1] if detail else f"{os.path.basename(args[0])} failed")
    return out


def feed(args, text):
    """Give text to a program that keeps running to serve the clipboard (wl-copy,
    xclip, xsel fork into the background), so never wait on its output pipes."""
    try:
        proc = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    except FileNotFoundError:
        raise RuntimeError(f"{args[0]} isn't installed")
    try:
        proc.communicate(text.encode("utf-8"), timeout=TIMEOUT)
    except subprocess.TimeoutExpired:
        return  # still holding the clipboard in the foreground; that's its job
    if proc.returncode != 0:
        raise RuntimeError(f"{args[0]} couldn't set the clipboard")


# --- Windows ---------------------------------------------------------------------

# Windows PowerShell 5.1. Text goes through a UTF-8 file and comes back as base64,
# so no console code page can garble it and no command line gets too long.
WINDOWS_GET = r"""
$ErrorActionPreference = 'Stop'
$t = Get-Clipboard -Raw
if ($null -ne $t) { [Console]::Out.Write([Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes([string]$t))) }
"""

WINDOWS_SET = r"""
$ErrorActionPreference = 'Stop'
$t = [IO.File]::ReadAllText($path, [Text.Encoding]::UTF8)
if ($t.Length -eq 0) {
    Add-Type -AssemblyName System.Windows.Forms
    [System.Windows.Forms.Clipboard]::Clear()
} else {
    Set-Clipboard -Value $t
}
"""


def powershell(script):
    encoded = base64.b64encode(script.encode("utf-16-le")).decode("ascii")
    return run(["powershell.exe", "-NoProfile", "-NonInteractive", "-Sta", "-EncodedCommand", encoded])


class WindowsClipboard:
    def get(self):
        out = powershell(WINDOWS_GET).stdout.decode("ascii", "replace").strip()
        if not out:
            return None
        return base64.b64decode(out).decode("utf-8", "replace")

    def set(self, text):
        fd, path = tempfile.mkstemp(prefix="clip-", suffix=".txt", dir=plugin.data_dir)
        try:
            with os.fdopen(fd, "wb") as f:
                f.write(text.encode("utf-8"))
            powershell("$path = '" + path.replace("'", "''") + "'\n" + WINDOWS_SET)
        finally:
            try:
                os.remove(path)
            except OSError:
                pass

    def paste(self, combo):
        import winapi  # ctypes.WinDLL: Windows only

        winapi.press(combo)


# --- macOS ------------------------------------------------------------------------

MAC_PASTE = 'tell application "System Events" to keystroke "v" using command down'
MAC_ACCESS = (
    "macOS blocked the paste: allow PerritoDeck (or the app that started it) under System Settings → "
    "Privacy & Security → Accessibility"
)


class MacClipboard:
    def env(self):
        # pbcopy/pbpaste follow the locale; started from the Dock there may be none.
        return dict(os.environ, **UTF8_ENV)

    def get(self):
        out = run(["pbpaste"], env=self.env()).stdout.decode("utf-8", "replace")
        return out or None

    def set(self, text):
        run(["pbcopy"], data=text, env=self.env())

    def paste(self, combo):
        out = run(["osascript", "-e", MAC_PASTE], check=False)
        if out.returncode != 0:
            err = out.stderr.decode("utf-8", "replace")
            if "-1719" in err or "-25211" in err or "assistive" in err or "-1743" in err:
                raise RuntimeError(MAC_ACCESS)
            raise RuntimeError(err.strip().splitlines()[-1] if err.strip() else "couldn't press ⌘V")


# --- Linux ------------------------------------------------------------------------

# Paste shortcuts per tool. ydotool takes Linux input key codes:
# 29 left Ctrl, 42 left Shift, 47 V, 110 Insert.
LINUX_KEYS = {
    "wtype": {
        "ctrl+v": ["-M", "ctrl", "v", "-m", "ctrl"],
        "ctrl+shift+v": ["-M", "ctrl", "-M", "shift", "v", "-m", "shift", "-m", "ctrl"],
        "shift+insert": ["-M", "shift", "-k", "Insert", "-m", "shift"],
    },
    "ydotool": {
        "ctrl+v": ["key", "29:1", "47:1", "47:0", "29:0"],
        "ctrl+shift+v": ["key", "29:1", "42:1", "47:1", "47:0", "42:0", "29:0"],
        "shift+insert": ["key", "42:1", "110:1", "110:0", "42:0"],
    },
    "xdotool": {
        "ctrl+v": ["key", "--clearmodifiers", "ctrl+v"],
        "ctrl+shift+v": ["key", "--clearmodifiers", "ctrl+shift+v"],
        "shift+insert": ["key", "--clearmodifiers", "shift+Insert"],
    },
}


class LinuxClipboard:
    def __init__(self, env=None, which=shutil.which):
        env = os.environ if env is None else env
        self.wayland = bool(env.get("WAYLAND_DISPLAY"))
        if self.wayland and which("wl-copy") and which("wl-paste"):
            self.tool = "wl"
        elif which("xclip"):
            self.tool = "xclip"
        elif which("xsel"):
            self.tool = "xsel"
        else:
            self.tool = None
        order = ("wtype", "ydotool", "xdotool") if self.wayland else ("xdotool", "ydotool")
        self.keys = next((t for t in order if which(t)), None)

    def need(self):
        if not self.tool:
            hint = "wl-clipboard" if self.wayland else "xclip or xsel"
            raise RuntimeError(f"install {hint} to use the clipboard")

    def get(self):
        self.need()
        args = {
            "wl": ["wl-paste", "--no-newline", "--type", "text"],
            "xclip": ["xclip", "-selection", "clipboard", "-o", "-t", "UTF8_STRING"],
            "xsel": ["xsel", "--clipboard", "--output"],
        }[self.tool]
        out = run(args, check=False)
        if out.returncode != 0:
            return None  # empty, or nothing that's text
        return out.stdout.decode("utf-8", "replace") or None

    def set(self, text):
        self.need()
        if self.tool == "wl":
            if not text:
                run(["wl-copy", "--clear"])
                return
            feed(["wl-copy", "--type", "text/plain;charset=utf-8"], text)
        elif self.tool == "xclip":
            feed(["xclip", "-selection", "clipboard", "-i"], text)
        else:
            feed(["xsel", "--clipboard", "--input"], text)

    def paste(self, combo):
        if not self.keys:
            hint = "wtype or ydotool" if self.wayland else "xdotool"
            raise RuntimeError(f"install {hint} to press the paste shortcut")
        run([self.keys, *LINUX_KEYS[self.keys][combo]])


def make_clipboard(platform=None):
    platform = platform or sys.platform
    if platform == "win32":
        return WindowsClipboard()
    if platform == "darwin":
        return MacClipboard()
    return LinuxClipboard()


_clipboard = None


def clipboard():
    global _clipboard
    if _clipboard is None:
        _clipboard = make_clipboard()
    return _clipboard


# --- Slots ------------------------------------------------------------------------

_slots_lock = threading.Lock()


def slots_path():
    return os.path.join(plugin.data_dir, "slots.json")


def load_slots():
    try:
        with open(slots_path(), encoding="utf-8") as f:
            data = json.load(f)
        return {str(k): str(v) for k, v in data.items()} if isinstance(data, dict) else {}
    except (OSError, ValueError):
        return {}


def store_slot(slot, text):
    with _slots_lock:
        slots = load_slots()
        slots[slot] = text
        tmp = slots_path() + ".tmp"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(slots, f, ensure_ascii=False, indent=1)
        os.replace(tmp, slots_path())


def slot_number(settings):
    raw = str(settings.get("slot") if settings.get("slot") is not None else "").strip()
    try:
        n = int(float(raw))
    except ValueError:
        raise ValueError("set a slot number (1, 2, 3…)")
    if n < 1:
        raise ValueError("slot numbers start at 1")
    return str(n)


def preview(text):
    """What a key shows for a slot: the first 20 characters on one line."""
    if not text:
        return NOTHING
    flat = " ".join(text.split())
    if not flat:
        return NOTHING
    return flat if len(flat) <= PREVIEW else flat[:PREVIEW].rstrip() + "…"


def publish_slots(only=None):
    slots = load_slots()
    for arg in plugin.watched("slot"):
        try:
            key = slot_number({"slot": arg})
        except ValueError:
            if only is None:
                plugin.set_value("slot", NOTHING, arg=arg)
            continue
        if only is None or key == only:
            plugin.set_value("slot", preview(slots.get(key)), arg=arg)


# --- Pasting ----------------------------------------------------------------------

_paste_lock = threading.Lock()
_restore = {"timer": None, "old": None, "ours": None}


def restore_delay():
    try:
        delay = float(plugin.settings.get("restore_after", 0.5))
    except (TypeError, ValueError):
        delay = 0.5
    return min(max(delay, 0.1), 5.0)


def put_back(timer):
    """Put the old clipboard back, unless something new was copied since. A timer a
    later paste took over (it fired while that paste ran) does nothing."""
    with _paste_lock:
        if _restore["timer"] is not timer:
            return
        old, ours = _restore["old"], _restore["ours"]
        _restore.update(timer=None, old=None, ours=None)
        try:
            cb = clipboard()
            if cb.get() == ours:
                cb.set(old)
        except Exception as e:
            plugin.log(f"couldn't put the clipboard back: {e}", "warn")


def paste(text, settings):
    combo = str(settings.get("keys") or "ctrl+v")
    if combo not in COMBOS:
        raise ValueError(f"unknown paste shortcut {combo}")
    restore = settings.get("restore", True)
    restore = restore not in (False, "false", "0", 0)
    cb = clipboard()
    with _paste_lock:
        old = None
        if _restore["timer"] is not None:
            # An earlier paste is still waiting to put the clipboard back: keep its
            # original, not the snippet it pasted.
            _restore["timer"].cancel()
            old = _restore["old"]
        elif restore:
            old = cb.get()
        _restore.update(timer=None, old=None, ours=None)
        cb.set(text)
        time.sleep(0.05)  # let the clipboard owner settle before the app asks for it
        cb.paste(combo)
        if restore and old is not None and old != text:
            timer = threading.Timer(restore_delay(), put_back)
            timer.args = (timer,)
            timer.daemon = True
            _restore.update(timer=timer, old=old, ours=text)
            timer.start()


def snippet(settings):
    text = settings.get("text")
    if text is None or str(text) == "":
        raise ValueError("type the text first")
    return str(text)


@plugin.action("copy")
def copy_text(settings):
    clipboard().set(snippet(settings))


@plugin.action("paste")
def paste_text(settings):
    paste(snippet(settings), settings)


@plugin.action("save-slot")
def save_slot(settings):
    slot = slot_number(settings)
    text = clipboard().get()
    if text is None:
        return "the clipboard has no text to save"
    store_slot(slot, text)
    publish_slots(only=slot)
    return None


def slot_text(settings):
    slot = slot_number(settings)
    text = load_slots().get(slot)
    if not text:
        raise RuntimeError(f"slot {slot} is empty")
    return text


@plugin.action("paste-slot")
def paste_slot(settings):
    paste(slot_text(settings), settings)


@plugin.action("copy-slot")
def copy_slot(settings):
    clipboard().set(slot_text(settings))


@plugin.on_watch
def watching(_values):
    publish_slots()


if __name__ == "__main__":
    plugin.run()
