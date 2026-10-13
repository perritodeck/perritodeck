"""Now playing and media controls.

Each platform has its own backend, picked by sys.platform:

- Linux: playerctl (MPRIS).
- Windows: PowerShell 5.1 and the WinRT Global System Media Transport Controls
  (the session the volume flyout shows).
- macOS: osascript talking to Spotify, or else Music, only if one is already running.

The Windows and macOS backends are untested on real hardware so far; they are
covered by unit tests that mock subprocess only.
"""

import base64
import html
import json
import os
import re
import shutil
import subprocess
import sys
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()
NOTHING = "–"
PLAYERCTL = shutil.which("playerctl")

TIMEOUT = 3  # seconds; no backend call may block longer than this
# A cold Windows PowerShell 5.1 start that loads WinRT (and is scanned by Defender)
# can take several seconds.
WINDOWS_TIMEOUT = 8
CACHE_SECONDS = 1.0
POLL_SECONDS = 2
# Windows starts a PowerShell per poll, so it polls less often once nobody has pressed
# a media key for a while. (The host only tells plugins which values with arguments
# are on keys, so the plugin can't tell whether "status" or "title" is shown.)
IDLE_AFTER_SECONDS = 30
IDLE_POLL_SECONDS = 10
MAX_ERROR = 80  # characters of an error message shown on a key
COMMANDS = ("play-pause", "next", "previous")
# subprocess.CREATE_NO_WINDOW only exists on Windows; the value is fixed.
CREATE_NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0x08000000)


def short(message, limit=MAX_ERROR):
    """One line of at most `limit` characters, for a key's alert."""
    line = " ".join(str(message).split())
    return line if len(line) <= limit else line[: limit - 1] + "…"


def describe(artist, title):
    if artist and title:
        return f"{artist} – {title}"
    return title or artist or NOTHING


# --- Linux: playerctl ------------------------------------------------------


def playerctl(*args):
    if not PLAYERCTL:
        raise RuntimeError("playerctl isn't installed (e.g. sudo apt install playerctl)")
    out = subprocess.run([PLAYERCTL, *args], capture_output=True, text=True, timeout=3)
    if out.returncode != 0:
        raise RuntimeError(out.stderr.strip() or "no player is running")
    return out.stdout.strip()


def linux_state():
    try:
        status = playerctl("status").lower()
        title = describe(playerctl("metadata", "artist"), playerctl("metadata", "title"))
    except subprocess.TimeoutExpired:
        raise
    except Exception:
        status, title = "stopped", NOTHING
    return status, title


def linux_control(command):
    playerctl(command)  # play-pause, next and previous are playerctl's own names


# --- Windows: PowerShell + WinRT SMTC --------------------------------------

# Runs under Windows PowerShell 5.1 (powershell.exe), which can still load WinRT
# types; PowerShell 7 can't. $cmd is prepended by windows_run() from COMMANDS or
# "status", never from user input. It prints exactly one line of ASCII-only JSON:
#   status:   {"ok":true,"session":false}
#             {"ok":true,"session":true,"artist":"..","title":"..","status":"Playing"}
#   controls: {"ok":true} or {"ok":false}  (false: the player refused)
#   failure:  {"ok":false,"error":"message"}
WINDOWS_SCRIPT = r"""
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Out-Json($data) {
    $json = ConvertTo-Json -InputObject $data -Compress
    # Escape everything outside ASCII so the console code page can't garble it.
    $json = [regex]::Replace($json, '[^\x00-\x7F]', { param($m) '\u{0:x4}' -f [int][char]$m.Value })
    [Console]::Out.WriteLine($json)
}

try {
    Add-Type -AssemblyName System.Runtime.WindowsRuntime
    # WindowsRuntimeSystemExtensions.AsTask<T>(IAsyncOperation<T>) turns a WinRT
    # async operation into a .NET Task we can wait on.
    $asTask = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
        $_.Name -eq 'AsTask' -and $_.IsGenericMethodDefinition -and
        $_.GetParameters().Count -eq 1 -and
        $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1'
    } | Select-Object -First 1
    if ($null -eq $asTask) { throw 'AsTask for IAsyncOperation was not found' }

    function Await($operation, [Type]$resultType) {
        $task = $asTask.MakeGenericMethod($resultType).Invoke($null, @($operation))
        if (-not $task.Wait(2000)) { throw 'the media session did not answer in time' }
        return $task.Result
    }

    $managerType = [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager, Windows.Media.Control, ContentType = WindowsRuntime]
    $propertiesType = [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionMediaProperties, Windows.Media.Control, ContentType = WindowsRuntime]

    $manager = Await ($managerType::RequestAsync()) $managerType
    $session = $manager.GetCurrentSession()

    if ($cmd -eq 'status') {
        if ($null -eq $session) {
            Out-Json ([ordered]@{ ok = $true; session = $false })
        } else {
            $properties = Await ($session.TryGetMediaPropertiesAsync()) $propertiesType
            $playback = $session.GetPlaybackInfo()
            Out-Json ([ordered]@{
                ok = $true
                session = $true
                artist = [string]$properties.Artist
                title = [string]$properties.Title
                status = [string]$playback.PlaybackStatus
            })
        }
    } else {
        if ($null -eq $session) { throw 'no player is running' }
        switch ($cmd) {
            'play-pause' { $operation = $session.TryTogglePlayPauseAsync() }
            'next' { $operation = $session.TrySkipNextAsync() }
            'previous' { $operation = $session.TrySkipPreviousAsync() }
            default { throw "unknown command $cmd" }
        }
        $done = Await $operation ([bool])
        Out-Json ([ordered]@{ ok = [bool]$done })
    }
} catch {
    Out-Json ([ordered]@{ ok = $false; error = [string]$_.Exception.GetBaseException().Message })
}
"""


def windows_run(command):
    """Run WINDOWS_SCRIPT for command ("status" or one of COMMANDS); return its JSON."""
    if command != "status" and command not in COMMANDS:
        raise ValueError(f"unknown command {command}")
    script = f"$cmd = '{command}'\n{WINDOWS_SCRIPT}"
    # -EncodedCommand (UTF-16LE, base64) avoids every command-line quoting problem.
    encoded = base64.b64encode(script.encode("utf-16-le")).decode("ascii")
    out = subprocess.run(
        ["powershell.exe", "-NoProfile", "-NonInteractive", "-EncodedCommand", encoded],
        stdin=subprocess.DEVNULL,  # never let PowerShell read the host's messages
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=WINDOWS_TIMEOUT,
        creationflags=CREATE_NO_WINDOW,
    )
    return parse_windows(out.stdout, out.stderr)


def parse_windows(stdout, stderr=""):
    """The last JSON object line PowerShell printed, as a dict."""
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


def windows_status(playback_status):
    # GlobalSystemMediaTransportControlsSessionPlaybackStatus:
    # Closed, Opened, Changing, Stopped, Playing, Paused.
    status = (playback_status or "").lower()
    return status if status in ("playing", "paused") else "stopped"


def windows_state():
    data = windows_run("status")
    if not data.get("ok"):
        raise RuntimeError(data.get("error") or "the media session failed")
    if not data.get("session"):
        return "stopped", NOTHING
    return windows_status(data.get("status")), describe(data.get("artist") or "", data.get("title") or "")


def windows_control(command):
    data = windows_run(command)
    if data.get("error"):
        raise RuntimeError(data["error"])
    if not data.get("ok"):
        raise RuntimeError("the player refused")


# --- macOS: osascript --------------------------------------------------------

MAC_APPS = ("Spotify", "Music")

# Only asks whether an app is running: no `tell`, so nothing gets launched, and
# the app needn't be installed. Prints the first running app, or an empty line.
MAC_FIND_SCRIPT = """
set found to ""
try
    if application "Spotify" is running then set found to "Spotify"
end try
if found is "" then
    try
        if application "Music" is running then set found to "Music"
    end try
end if
return found
"""

# One script per app, so a missing Spotify can't break compiling Music's part.
# The `is running` guard stops `tell` from relaunching an app that quit since
# MAC_FIND_SCRIPT ran. Spotify and Music share these terms. Fields are separated
# by ASCII unit separators (character id 31).
MAC_APP_SCRIPT = """
on run argv
    set cmd to item 1 of argv
    if application "%APP%" is not running then return "none"
    tell application "%APP%"
        if cmd is "status" then
            set sep to character id 31
            set st to ""
            set ar to ""
            set nm to ""
            try
                set st to (player state) as text
            end try
            try
                set ar to (artist of current track) as text
            end try
            try
                set nm to (name of current track) as text
            end try
            return "state" & sep & st & sep & ar & sep & nm
        else if cmd is "playpause" then
            playpause
        else if cmd is "next" then
            next track
        else if cmd is "previous" then
            previous track
        end if
    end tell
    return "ok"
end run
"""

MAC_COMMANDS = {"play-pause": "playpause", "next": "next", "previous": "previous"}

# `player state as text` is normally the word; the raw four-letter codes show up if
# the dictionary terms aren't available, so accept both.
MAC_STATES = {
    "playing": "playing",
    "fast forwarding": "playing",
    "rewinding": "playing",
    "paused": "paused",
    "«constant ****kpsp»": "playing",
    "«constant ****kpsf»": "playing",
    "«constant ****kpsr»": "playing",
}


def osascript(script, *args):
    out = subprocess.run(
        ["osascript", "-e", script, *args],
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=TIMEOUT,
    )
    if out.returncode != 0:
        raise RuntimeError(out.stderr.strip() or "osascript failed")
    return out.stdout.rstrip("\r\n")


def mac_find_player():
    found = osascript(MAC_FIND_SCRIPT).strip()
    return found if found in MAC_APPS else None


def mac_status(state):
    state = (state or "").strip()
    if state == "«constant ****kPSp»":  # paused; differs from playing only by case
        return "paused"
    return MAC_STATES.get(state.lower(), "stopped")


def parse_mac(output):
    """(status, title) from MAC_APP_SCRIPT's status output."""
    if not output.startswith("state\x1f"):
        return "stopped", NOTHING
    _, state, artist, name = (output.split("\x1f", 3) + ["", "", ""])[:4]
    clean = [x.strip() for x in (artist, name)]
    clean = ["" if x == "missing value" else x for x in clean]
    return mac_status(state), describe(*clean)


def mac_state():
    app = mac_find_player()
    if not app:
        return "stopped", NOTHING
    return parse_mac(osascript(MAC_APP_SCRIPT.replace("%APP%", app), "status"))


def mac_control(command):
    app = mac_find_player()
    if not app:
        raise RuntimeError("neither Spotify nor Music is running")
    if osascript(MAC_APP_SCRIPT.replace("%APP%", app), MAC_COMMANDS[command]) == "none":
        raise RuntimeError(f"{app} isn't running")


# --- Dispatch ----------------------------------------------------------------


def backend(platform=None):
    """(state, control) functions for this platform."""
    platform = platform or sys.platform
    if platform == "win32":
        return windows_state, windows_control
    if platform == "darwin":
        return mac_state, mac_control
    return linux_state, linux_control


_cache_lock = threading.Lock()
# "forgotten" counts forget_state() calls. A backend call that one happened during may
# have read the state from before a key press, so its answer isn't cached.
_cache = {"at": None, "state": None, "forgotten": 0}


def current_state():
    """(status, title), at most CACHE_SECONDS old. The backend runs without the lock
    held: a status query on Windows takes about 2 s, and a key press (forget_state)
    mustn't wait for one."""
    with _cache_lock:
        if _cache["at"] is not None and time.monotonic() - _cache["at"] < CACHE_SECONDS:
            return _cache["state"]
        forgotten = _cache["forgotten"]
    state = backend()[0]()
    with _cache_lock:
        if _cache["forgotten"] == forgotten:
            _cache["at"], _cache["state"] = time.monotonic(), state
    return state


def forget_state():
    with _cache_lock:
        _cache["at"] = None
        _cache["forgotten"] += 1


_last_action = time.monotonic()
_woken = threading.Event()  # set by an action, so the keys update right away


def control(command):
    global _last_action
    _last_action = time.monotonic()
    try:
        backend()[1](command)
    except subprocess.TimeoutExpired as e:
        # str(e) is the whole command line, a base64 script on Windows.
        raise RuntimeError(f"the player didn't answer in {e.timeout:g} s") from None
    except Exception as e:
        raise RuntimeError(short(e)) from None
    finally:
        forget_state()  # the next poll shows the change
        _woken.set()


def poll_interval(platform=None, now=None):
    """Seconds until the next poll: longer on Windows after a quiet spell."""
    platform = platform or sys.platform
    now = time.monotonic() if now is None else now
    if platform == "win32" and now - _last_action > IDLE_AFTER_SECONDS:
        return IDLE_POLL_SECONDS
    return POLL_SECONDS


def poll():
    last = None
    while True:
        try:
            status, title = current_state()
        except subprocess.TimeoutExpired:
            status, title = last or ("stopped", NOTHING)  # slow this time; keep what we had
        except Exception:
            status, title = "stopped", NOTHING
        if (status, title) != last:
            plugin.set_value("status", status)
            plugin.set_value("title", title)
            last = (status, title)
        _woken.wait(poll_interval())
        _woken.clear()


@plugin.on_start
def start():
    if backend()[0] is linux_state and not PLAYERCTL:
        plugin.log("playerctl isn't installed, so nothing can be shown or controlled", "warn")
    threading.Thread(target=poll, daemon=True).start()


@plugin.action("play-pause")
def play_pause(settings):
    control("play-pause")


@plugin.action("next")
def next_track(settings):
    control("next")


@plugin.action("previous")
def previous_track(settings):
    control("previous")


if __name__ == "__main__":
    plugin.run()
