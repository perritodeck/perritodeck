"""Soundboard: play sound files from a key, with the players each OS already has.

- Windows: in-process through winmm's MCI "mpegvideo" device (WAV, MP3, WMA, AAC…,
  with volume; see winmm.py). If that can't play a file: Windows PowerShell 5.1 with
  WPF's System.Windows.Media.MediaPlayer, falling back to System.Media.SoundPlayer for
  WAV (no volume).
- macOS: afplay -v.
- Linux: pw-play, paplay, aplay (WAV only), then ffplay -nodisp -autoexit, in turn:
  when one can't play the file (it exits with an error), the next one tries.

Every sound is a child process (or, for MCI, a thread shaped like one), so stopping it
is stopping the process; the `playing:<file>` value follows them. Standard library only.
"""

import base64
import html
import os
import re
import shutil
import subprocess
import sys
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402
import winmm  # noqa: E402  (imports everywhere; loads winmm.dll only when used)

plugin = Plugin()

MODES = ("play", "toggle", "restart", "overlap")
MAX_SOUNDS = 32  # all files together; stops a stuck key from starting hundreds
# subprocess.CREATE_NO_WINDOW only exists on Windows; the value is fixed.
CREATE_NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0x08000000)
MCI_PLAYER = "winmm MCI"  # argv[0] of the in-process Windows player
INSTALL_HINT = "install PipeWire (pw-play), pulseaudio-utils (paplay), alsa-utils (aplay) or ffmpeg (ffplay)"

# Runs under Windows PowerShell 5.1 (-STA, which WPF needs). The file and volume
# come in environment variables, so no path ever has to be quoted into the script.
# MediaPlayer raises its events through a WPF dispatcher, which a console script
# doesn't run, so Pump processes the dispatcher's queue between checks.
# A failure is one line of UTF-8 on stderr, which the plugin logs; uncaught,
# PowerShell would write it as CLIXML in the console's code page.
WINDOWS_SCRIPT = r"""
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$path = $env:PERRITODECK_SOUND
$volume = [double]::Parse($env:PERRITODECK_VOLUME, [Globalization.CultureInfo]::InvariantCulture)
try {
    Add-Type -AssemblyName PresentationCore, WindowsBase
    $dispatcher = [System.Windows.Threading.Dispatcher]::CurrentDispatcher
    function Pump {
        $frame = New-Object System.Windows.Threading.DispatcherFrame
        $done = [System.Windows.Threading.DispatcherOperationCallback]{ param($f) $f.Continue = $false; $null }
        $null = $dispatcher.BeginInvoke([System.Windows.Threading.DispatcherPriority]::Background, $done, $frame)
        [System.Windows.Threading.Dispatcher]::PushFrame($frame)
    }
    $global:soundFailure = $null
    $player = New-Object System.Windows.Media.MediaPlayer
    $player.add_MediaFailed({ param($s, $e) $global:soundFailure = [string]$e.ErrorException.Message })
    $player.Volume = $volume
    $player.Open([Uri]$path)
    $player.Play()
    $clock = [Diagnostics.Stopwatch]::StartNew()
    while ($true) {
        Pump
        if ($global:soundFailure) { throw $global:soundFailure }
        if ($player.NaturalDuration.HasTimeSpan) {
            $length = $player.NaturalDuration.TimeSpan
            if ($player.Position -ge $length.Subtract([TimeSpan]::FromMilliseconds(30))) { break }
            if ($clock.Elapsed -gt $length.Add([TimeSpan]::FromSeconds(3))) { break }
        } elseif ($clock.Elapsed.TotalSeconds -gt 10) {
            throw 'the file could not be opened'
        }
        Start-Sleep -Milliseconds 50
    }
    $player.Close()
} catch {
    $failure = $_.Exception
    if ($path -match '\.wav$') {
        # SoundPlayer can't change the volume, but plays WAV without WPF.
        try {
            (New-Object System.Media.SoundPlayer $path).PlaySync()
            $failure = $null
        } catch {
            $failure = $_.Exception
        }
    }
    if ($null -ne $failure) {
        # Some messages break the line before the file's path.
        [Console]::Error.WriteLine(([string]$failure.GetBaseException().Message -replace '\s+', ' ').Trim())
        exit 1
    }
}
"""


# --- Settings from a key ------------------------------------------------------


def clean_path(path):
    """The file as typed or pasted ("Copy as path" adds quotes), ~ and $VARS expanded."""
    path = (path or "").strip().strip('"').strip("'").strip()
    return os.path.abspath(os.path.expandvars(os.path.expanduser(path))) if path else ""


def sound_key(path):
    """One key per file, so C:\\a.wav and c:/A.WAV match on Windows."""
    path = clean_path(path)
    return os.path.normcase(os.path.normpath(path)) if path else ""


def parse_volume(value):
    if value is None or value == "":
        return 100
    try:
        volume = float(value)
    except (TypeError, ValueError):
        raise ValueError(f"the volume “{value}” isn't a number from 0 to 100")
    return int(round(min(100.0, max(0.0, volume))))


# --- Players ------------------------------------------------------------------


def player_commands(path, volume, device="", platform=None, which=shutil.which):
    """The command lines that could play `path`, best first, as (argv, extra env)."""
    platform = platform or sys.platform
    level = volume / 100.0
    if platform == "win32":
        encoded = base64.b64encode(WINDOWS_SCRIPT.encode("utf-16-le")).decode("ascii")
        argv = ["powershell.exe", "-NoProfile", "-NonInteractive", "-STA", "-EncodedCommand", encoded]
        return [
            ([MCI_PLAYER, path, str(volume)], {}),
            (argv, {"PERRITODECK_SOUND": path, "PERRITODECK_VOLUME": f"{level:.3f}"}),
        ]
    if platform == "darwin":
        return [([which("afplay") or "/usr/bin/afplay", "-v", f"{level:.3f}", path], {})]
    commands = []
    pw_play = which("pw-play")
    if pw_play:
        target = ["--target", device] if device else []
        commands.append(([pw_play, "--volume", f"{level:.3f}", *target, path], {}))
    paplay = which("paplay")
    if paplay:
        target = [f"--device={device}"] if device else []
        # PulseAudio volume: 65536 is 100 %.
        commands.append(([paplay, f"--volume={int(round(level * 65536))}", *target, path], {}))
    aplay = which("aplay")
    if aplay and path.lower().endswith(".wav"):
        commands.append(([aplay, "-q", path], {}))  # no volume control
    ffplay = which("ffplay")
    if ffplay:
        commands.append(([ffplay, "-nodisp", "-autoexit", "-loglevel", "error", "-volume", str(volume), path], {}))
    return commands


def spawn(argv, extra_env):
    if argv[0] == MCI_PLAYER:
        return winmm.MciSound(argv[1], int(argv[2]))
    env = None
    if extra_env:
        env = dict(os.environ)
        env.update(extra_env)
    return subprocess.Popen(
        argv,
        stdin=subprocess.DEVNULL,  # never let a player read the host's messages
        stdout=subprocess.DEVNULL,  # stdout is the protocol
        stderr=subprocess.PIPE,
        env=env,
        creationflags=CREATE_NO_WINDOW if sys.platform == "win32" else 0,
    )


def error_text(stderr):
    """The message in what a player wrote on stderr, or "": its last line, except
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
    # A character outside the BMP (an emoji) comes as its two UTF-16 halves, each
    # escaped on its own; join them, or they'd print as "??".
    text = text.encode("utf-16-le", "surrogatepass").decode("utf-16-le", "replace")
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    return lines[0] if lines else ""


class Sound:
    def __init__(self, key, path, commands):
        self.key, self.path = key, path
        self.commands = list(commands)  # what's left to try
        self.proc = None
        self.player = None
        self.stopped = False


_lock = threading.RLock()
_sounds = {}  # sound_key -> [Sound]
_published = {}  # watched argument -> "on"/"off"


def _start_next(sound):
    """Start the next player that can be spawned; False when there's none left."""
    while sound.commands:
        argv, env = sound.commands.pop(0)
        try:
            sound.proc = spawn(argv, env)
            sound.player = os.path.basename(argv[0])
            return True
        except OSError as e:
            plugin.log(f"couldn't start {os.path.basename(argv[0])}: {e}", "warn")
    return False


def _follow(sound):
    """Wait for a sound's player; try the next player if it failed; tidy up."""
    while True:
        proc = sound.proc
        try:
            _, err = proc.communicate()
        except Exception:
            err = b""
        detail = error_text((err or b"").decode("utf-8", "replace"))
        with _lock:
            if sound.stopped or proc.returncode == 0:
                break
            plugin.log(f"{sound.player} couldn't play {sound.path}: {detail or 'exit code %s' % proc.returncode}", "warn")
            if not _start_next(sound):
                plugin.log(f"no player could play {sound.path}", "error")
                break
    with _lock:
        playing = _sounds.get(sound.key, [])
        if sound in playing:
            playing.remove(sound)
        if not playing:
            _sounds.pop(sound.key, None)
    publish()


def _stop(sounds):
    for sound in sounds:
        sound.stopped = True
        try:
            if sound.proc is not None and sound.proc.poll() is None:
                sound.proc.terminate()
        except OSError:
            pass


def is_playing(key):
    with _lock:
        return bool(_sounds.get(key))


def play(path, volume=100, mode="play"):
    """Start (or stop, for mode "toggle") a sound; returns "started", "stopped" or "playing"."""
    if mode not in MODES:
        raise ValueError(f"unknown mode {mode}")
    full = clean_path(path)
    if not full:
        raise ValueError("choose a sound file first")
    if not os.path.isfile(full):
        raise FileNotFoundError(f"can't find the sound file {full}")
    key = sound_key(full)
    device = str(plugin.settings.get("device") or "").strip()
    with _lock:
        playing = list(_sounds.get(key, []))
        if playing and mode == "play":
            return "playing"
        if playing and mode == "toggle":
            _stop(playing)
            return "stopped"
        if playing and mode == "restart":
            _stop(playing)
        if sum(len(s) for s in _sounds.values()) >= MAX_SOUNDS:
            raise RuntimeError(f"{MAX_SOUNDS} sounds are already playing; stop some first")
        commands = player_commands(full, volume, device)
        if not commands:
            raise RuntimeError(f"there's no sound player on this computer; {INSTALL_HINT}")
        sound = Sound(key, full, commands)
        if not _start_next(sound):
            raise RuntimeError(f"no sound player could be started; {INSTALL_HINT}")
        _sounds.setdefault(key, []).append(sound)
    threading.Thread(target=_follow, args=(sound,), daemon=True).start()
    publish()
    return "started"


def stop_all():
    with _lock:
        everything = [s for sounds in _sounds.values() for s in sounds]
    _stop(everything)
    return len(everything)


# --- Values -------------------------------------------------------------------


def publish():
    """Publish playing:<file> for every file keys show, when it changed."""
    with _lock:
        for arg in plugin.watched("playing"):
            state = "on" if is_playing(sound_key(arg)) else "off"
            if _published.get(arg) != state:
                _published[arg] = state
                plugin.set_value("playing", state, arg=arg)


@plugin.on_watch
def watch_changed(values):
    with _lock:
        watched = set(plugin.watched("playing"))
        for arg in list(_published):
            if arg not in watched:
                del _published[arg]
    publish()


@plugin.on_settings
def settings_changed(settings):
    device = str(settings.get("device") or "").strip()
    if device and sys.platform in ("win32", "darwin"):
        plugin.log("the output device setting is only used on Linux (pw-play and paplay); sounds play on the default device", "warn")


@plugin.every(5)
def keep_in_step():
    publish()  # a safety net; sounds also publish the moment they start and end


# --- Actions ------------------------------------------------------------------


@plugin.action("play")
def play_action(settings):
    mode = settings.get("mode") or "play"
    play(settings.get("file"), parse_volume(settings.get("volume")), mode)


@plugin.action("stop-all")
def stop_all_action(settings):
    stop_all()
    deadline = time.monotonic() + 2
    while time.monotonic() < deadline:  # so a toggle following playing:… flips back now
        with _lock:
            if not _sounds:
                break
        time.sleep(0.05)
    publish()


if __name__ == "__main__":
    plugin.run()
