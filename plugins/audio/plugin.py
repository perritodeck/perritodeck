"""Audio devices: volume, mute, default devices and per-app volume.

One backend per OS, picked by sys.platform:

- Linux: wpctl (PipeWire/WirePlumber), or pactl (PulseAudio, or PipeWire's
  pipewire-pulse) when wpctl isn't there. Per-app volume uses pactl's sink inputs,
  or wpctl's streams when pactl is missing.
- macOS: osascript for the volume, mute and input volume; the microphone is "muted"
  by setting its input volume to 0 and restoring the old level afterwards. Switching
  devices needs the optional SwitchAudioSource tool. There's no per-app volume.
- Windows: Core Audio COM through ctypes, in win_audio.py.

Standard library only.
"""

import os
import re
import shutil
import subprocess
import sys
import threading

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()

UNKNOWN = "–"
TIMEOUT = 4  # seconds for each command; actions answer well within the host's 10 s
POLL_SECONDS = 2
DEFAULT_STEP = 5
MAC_DEFAULT_INPUT = 75  # what unmuting restores when the old level isn't known
FIXED_VALUES = ("volume", "output_muted", "mic_muted", "output_device", "input_device")
SWITCH_AUDIO_HINT = "switching devices on macOS needs SwitchAudioSource (brew install switchaudio-osx)"


class Unsupported(Exception):
    """This computer can't do it; actions show it as a warning, not an error."""


def run(argv, timeout=TIMEOUT):
    """Run a command and return its output; raise with its last error line."""
    env = dict(os.environ, LC_ALL="C", LANG="C")  # pactl translates its output
    out = subprocess.run(
        argv,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout,
        env=env,
    )
    if out.returncode != 0:
        detail = out.stderr.strip().splitlines()
        raise RuntimeError(f"{os.path.basename(argv[0])}: {detail[-1] if detail else 'exit code %d' % out.returncode}")
    return out.stdout


def clamp(percent):
    return int(round(min(100.0, max(0.0, float(percent)))))


def app_matches(candidates, wanted):
    """Does any of a stream's names ("spotify", "Spotify") match what a key asks for?"""
    def plain(name):
        name = (name or "").strip().lower()
        return name[:-4] if name.endswith(".exe") else name

    wanted = plain(wanted)
    return bool(wanted) and any(plain(c) == wanted for c in candidates if c)


# --- Linux: parsing -------------------------------------------------------------

_WPCTL_VOLUME = re.compile(r"Volume:\s*([0-9.]+)(.*)")
_TREE = "│├└─ \t"
_WPCTL_ITEM = re.compile(r"^(\*)?\s*(\d+)\.\s+(.*?)\s*(?:\[([^\]]*)\])?\s*$")


def parse_wpctl_volume(text):
    """'Volume: 0.40 [MUTED]' -> (40, True)."""
    m = _WPCTL_VOLUME.search(text or "")
    if not m:
        raise RuntimeError(f"wpctl gave an unexpected answer: {text.strip()!r}")
    return clamp(float(m.group(1)) * 100), "MUTED" in m.group(2)


def parse_wpctl_status(text):
    """The Audio part of `wpctl status`.

    Returns {"sinks": [...], "sources": [...], "streams": [...]}, each a list of
    {"id", "name", "default"}; ports listed under a stream are skipped."""
    found = {"sinks": [], "sources": [], "streams": []}
    top = section = None
    for line in (text or "").splitlines():
        if not line.strip():
            continue
        if not line[0].isspace() and line[0] not in _TREE:
            top, section = line.strip(), None  # "Audio", "Video", "Settings", "PipeWire …"
            continue
        body = line.lstrip(_TREE)
        if body.endswith(":") and not re.match(r"^\*?\s*\d+\.", body):
            section = body[:-1].strip().lower()  # "sinks", "sources", "streams", …
            continue
        if top != "Audio" or section not in found:
            continue
        m = _WPCTL_ITEM.match(body)
        if not m:
            continue
        name = m.group(3)
        if section == "streams" and (" > " in name or " < " in name):
            continue  # a port of the stream above, e.g. "output_FL > Built-in Audio:playback_FL"
        found[section].append({"id": m.group(2), "name": name, "default": bool(m.group(1))})
    return found


def parse_pactl_volume(text):
    """'Volume: front-left: 26214 /  40% / -23.88 dB, …' -> 40 (the first channel)."""
    m = re.search(r"(\d+)%", text or "")
    if not m:
        raise RuntimeError(f"pactl gave an unexpected answer: {(text or '').strip()!r}")
    return clamp(int(m.group(1)))


def parse_pactl_mute(text):
    """'Mute: yes' -> True."""
    m = re.search(r"Mute:\s*(\w+)", text or "")
    if not m:
        raise RuntimeError(f"pactl gave an unexpected answer: {(text or '').strip()!r}")
    return m.group(1).lower() in ("yes", "true", "1")


def parse_pactl_blocks(text, header):
    """Split `pactl list sinks|sources|sink-inputs` into one dict per block.

    Plain fields ("Name", "Description", "Mute", "Volume") keep their text;
    properties ("application.name = "…"") go under "props"."""
    blocks = []
    current = None
    in_props = False
    for line in (text or "").splitlines():
        m = re.match(r"^%s #(\d+)\s*$" % re.escape(header), line)
        if m:
            current = {"index": m.group(1), "props": {}}
            blocks.append(current)
            in_props = False
            continue
        stripped = line.strip()
        if current is None or not stripped:
            continue
        # pactl indents fields with one tab and properties, ports and the like with
        # two; measure it so output pasted with spaces parses the same.
        indent = len(line.expandtabs(8)) - len(line.expandtabs(8).lstrip())
        if in_props and indent > 8:
            p = re.match(r'^([\w.\-]+)\s*=\s*"(.*)"$', stripped)
            if p:
                current["props"][p.group(1)] = p.group(2)
            continue
        in_props = False
        if indent > 8:
            continue  # "balance 0.00", port lists, formats
        if stripped == "Properties:":
            in_props = True
            continue
        key, sep, value = stripped.partition(":")
        if sep and key not in current:
            current[key] = value.strip()
    return blocks


def parse_pactl_devices(text, header):
    """(name, description) of each sink or source; monitors of outputs are left out."""
    devices = []
    for block in parse_pactl_blocks(text, header):
        name = block.get("Name", "")
        if not name or name.endswith(".monitor") or block["props"].get("device.class") == "monitor":
            continue
        devices.append((name, block.get("Description") or name))
    return devices


def parse_pactl_sink_inputs(text):
    """Each app stream: {"id", "names" (process and app name), "volume", "muted"}."""
    streams = []
    for block in parse_pactl_blocks(text, "Sink Input"):
        props = block["props"]
        try:
            volume = parse_pactl_volume(block.get("Volume", ""))
        except RuntimeError:
            volume = None
        streams.append({
            "id": block["index"],
            "names": [props.get("application.process.binary"), props.get("application.name")],
            "volume": volume,
            "muted": (block.get("Mute", "").lower() == "yes"),
        })
    return streams


# --- Linux: backend -------------------------------------------------------------


class LinuxAudio:
    SINK, SOURCE = "@DEFAULT_AUDIO_SINK@", "@DEFAULT_AUDIO_SOURCE@"
    PA_SINK, PA_SOURCE = "@DEFAULT_SINK@", "@DEFAULT_SOURCE@"

    def __init__(self, which=shutil.which, runner=None):
        self.wpctl = which("wpctl")
        self.pactl = which("pactl")
        self.run = runner or run

    def _need(self):
        if not self.wpctl and not self.pactl:
            raise RuntimeError("needs wpctl (PipeWire) or pactl (PulseAudio); install wireplumber or pulseaudio-utils")

    # volume and mute

    def output_volume(self):
        """(volume 0–100, muted)."""
        self._need()
        if self.wpctl:
            return parse_wpctl_volume(self.run([self.wpctl, "get-volume", self.SINK]))
        return (parse_pactl_volume(self.run([self.pactl, "get-sink-volume", self.PA_SINK])),
                parse_pactl_mute(self.run([self.pactl, "get-sink-mute", self.PA_SINK])))

    def set_volume(self, percent):
        self._need()
        if self.wpctl:
            self.run([self.wpctl, "set-volume", self.SINK, f"{clamp(percent) / 100:.2f}"])
        else:
            self.run([self.pactl, "set-sink-volume", self.PA_SINK, f"{clamp(percent)}%"])

    def set_output_mute(self, muted):
        self._need()
        if self.wpctl:
            self.run([self.wpctl, "set-mute", self.SINK, "1" if muted else "0"])
        else:
            self.run([self.pactl, "set-sink-mute", self.PA_SINK, "1" if muted else "0"])

    def input_muted(self):
        self._need()
        if self.wpctl:
            return parse_wpctl_volume(self.run([self.wpctl, "get-volume", self.SOURCE]))[1]
        return parse_pactl_mute(self.run([self.pactl, "get-source-mute", self.PA_SOURCE]))

    def set_input_mute(self, muted):
        self._need()
        if self.wpctl:
            self.run([self.wpctl, "set-mute", self.SOURCE, "1" if muted else "0"])
        else:
            self.run([self.pactl, "set-source-mute", self.PA_SOURCE, "1" if muted else "0"])

    # devices

    def devices(self, kind):
        """([(id, name)], name of the default) for "output" or "input"."""
        self._need()
        if self.wpctl:
            status = parse_wpctl_status(self.run([self.wpctl, "status"]))
            items = status["sinks" if kind == "output" else "sources"]
            current = next((d["name"] for d in items if d["default"]), None)
            return [(d["id"], d["name"]) for d in items], current
        header = "Sink" if kind == "output" else "Source"
        found = parse_pactl_devices(self.run([self.pactl, "list", header.lower() + "s"]), header)
        default = self.run([self.pactl, "get-default-" + header.lower()]).strip()
        current = next((desc for name, desc in found if name == default), None)
        return found, current

    def set_default(self, kind, device_id):
        self._need()
        if self.wpctl:
            self.run([self.wpctl, "set-default", device_id])
        else:
            self.run([self.pactl, "set-default-" + ("sink" if kind == "output" else "source"), device_id])

    def snapshot(self):
        if not self.wpctl:
            return _snapshot(self)
        # One `wpctl status` for both device names instead of two.
        values = dict.fromkeys(FIXED_VALUES)
        try:
            values["volume"], values["output_muted"] = self.output_volume()
        except Exception as e:
            _log_once(f"couldn't read the volume: {e}")
        try:
            values["mic_muted"] = self.input_muted()
        except Exception as e:
            _log_once(f"couldn't read the microphone: {e}")
        try:
            status = parse_wpctl_status(self.run([self.wpctl, "status"]))
            for kind, section in (("output", "sinks"), ("input", "sources")):
                values[kind + "_device"] = next((d["name"] for d in status[section] if d["default"]), None)
        except Exception as e:
            _log_once(f"couldn't read the devices: {e}")
        return values

    # apps

    def _streams(self, app):
        if self.pactl:
            streams = parse_pactl_sink_inputs(self.run([self.pactl, "list", "sink-inputs"]))
            return [s for s in streams if app_matches(s["names"], app)]
        self._need()
        status = parse_wpctl_status(self.run([self.wpctl, "status"]))
        return [{"id": s["id"]} for s in status["streams"] if app_matches([s["name"]], app)]

    def app_volume(self, app):
        """(volume 0–100, muted) of the app's first stream, or None if it isn't playing."""
        streams = self._streams(app)
        if not streams:
            return None
        first = streams[0]
        if "volume" in first:
            return first["volume"], first["muted"]
        return parse_wpctl_volume(self.run([self.wpctl, "get-volume", first["id"]]))

    def set_app(self, app, percent=None, muted=None):
        streams = self._streams(app)
        for s in streams:
            if self.pactl:
                if percent is not None:
                    self.run([self.pactl, "set-sink-input-volume", s["id"], f"{clamp(percent)}%"])
                if muted is not None:
                    self.run([self.pactl, "set-sink-input-mute", s["id"], "1" if muted else "0"])
            else:
                if percent is not None:
                    self.run([self.wpctl, "set-volume", s["id"], f"{clamp(percent) / 100:.2f}"])
                if muted is not None:
                    self.run([self.wpctl, "set-mute", s["id"], "1" if muted else "0"])
        return len(streams)


# --- macOS ----------------------------------------------------------------------


def parse_mac_settings(text):
    """'output volume:44, input volume:75, alert volume:100, output muted:false' -> dict.

    Numbers become ints, true/false bools, and 'missing value' None (a device whose
    volume macOS can't change, like many HDMI and USB outputs)."""
    found = {}
    for part in (text or "").strip().split(","):
        key, sep, value = part.partition(":")
        if not sep:
            continue
        value = value.strip()
        if value == "missing value":
            parsed = None
        elif value in ("true", "false"):
            parsed = value == "true"
        else:
            try:
                parsed = int(float(value))
            except ValueError:
                parsed = value
        found[key.strip()] = parsed
    if "output volume" not in found:
        raise RuntimeError(f"osascript gave an unexpected answer: {(text or '').strip()!r}")
    return found


def parse_switch_audio_list(text, kind):
    """Device names from `SwitchAudioSource -a -t <kind>`.

    Current versions print one name per line for the type asked for; old ones
    printed every device as "Name (input)" / "Name (output)", so filter on that."""
    names = []
    for line in (text or "").splitlines():
        line = line.strip()
        if not line:
            continue
        m = re.match(r"^(.*) \((input|output)\)$", line)
        if m:
            if m.group(2) != kind:
                continue
            line = m.group(1)
        names.append(line)
    return names


class MacAudio:
    SEARCH = ("/opt/homebrew/bin", "/usr/local/bin")  # Homebrew; the host may not have a login PATH

    def __init__(self, which=shutil.which, runner=None, state_file=None):
        self.run = runner or run
        self.osascript = which("osascript") or "/usr/bin/osascript"
        self.switch = which("SwitchAudioSource") or next(
            (os.path.join(d, "SwitchAudioSource") for d in self.SEARCH
             if os.access(os.path.join(d, "SwitchAudioSource"), os.X_OK)), None)
        self._state_file = state_file

    def state_file(self):
        return self._state_file or os.path.join(plugin.data_dir, "mac-input-volume.txt")

    def _script(self, script):
        return self.run([self.osascript, "-e", script])

    def settings(self):
        return parse_mac_settings(self._script("get volume settings"))

    def output_volume(self):
        s = self.settings()
        if s.get("output volume") is None:
            raise Unsupported("macOS can't change this device's volume")
        return clamp(s["output volume"]), bool(s.get("output muted"))

    def set_volume(self, percent):
        self._script(f"set volume output volume {clamp(percent)}")

    def set_output_mute(self, muted):
        self._script(f"set volume output muted {'true' if muted else 'false'}")

    def input_muted(self):
        level = self.settings().get("input volume")
        if level is None:
            raise Unsupported("macOS can't change this microphone's level")
        return level == 0

    def _saved_input(self):
        try:
            with open(self.state_file(), encoding="utf-8") as f:
                level = int(f.read().strip())
            return level if 0 < level <= 100 else MAC_DEFAULT_INPUT
        except (OSError, ValueError):
            return MAC_DEFAULT_INPUT

    def set_input_mute(self, muted):
        level = self.settings().get("input volume")
        if level is None:
            raise Unsupported("macOS can't change this microphone's level")
        if muted:
            if level > 0:  # remember it, even across restarts, for unmuting
                try:
                    with open(self.state_file(), "w", encoding="utf-8") as f:
                        f.write(str(level))
                except OSError as e:
                    plugin.log(f"couldn't remember the microphone level: {e}", "warn")
            self._script("set volume input volume 0")
        elif level == 0:
            self._script(f"set volume input volume {self._saved_input()}")

    def devices(self, kind):
        if not self.switch:
            raise Unsupported(SWITCH_AUDIO_HINT)
        names = parse_switch_audio_list(self.run([self.switch, "-a", "-t", kind]), kind)
        current = self.run([self.switch, "-c", "-t", kind]).strip() or None
        return [(n, n) for n in names], current

    def set_default(self, kind, device_id):
        if not self.switch:
            raise Unsupported(SWITCH_AUDIO_HINT)
        self.run([self.switch, "-t", kind, "-s", device_id])

    def snapshot(self):
        values = dict.fromkeys(FIXED_VALUES)
        try:
            s = self.settings()
            if s.get("output volume") is not None:
                values["volume"] = clamp(s["output volume"])
            if s.get("output muted") is not None:
                values["output_muted"] = bool(s["output muted"])
            if s.get("input volume") is not None:
                values["mic_muted"] = s["input volume"] == 0
        except Exception as e:
            _log_once(f"couldn't read the volume: {e}")
        if self.switch:
            for kind in ("output", "input"):
                try:
                    values[kind + "_device"] = self.run([self.switch, "-c", "-t", kind]).strip() or None
                except Exception as e:
                    _log_once(f"couldn't read the {kind} device: {e}")
        return values

    def app_volume(self, app):
        raise Unsupported("macOS has no volume for each app")

    def set_app(self, app, percent=None, muted=None):
        raise Unsupported("macOS has no volume for each app")


# --- Windows --------------------------------------------------------------------


class WindowsAudio:
    def __init__(self, module=None):
        if module is None:
            import win_audio as module
        self.w = module

    def output_volume(self):
        return self.w.get_volume("output"), self.w.get_mute("output")

    def set_volume(self, percent):
        self.w.set_volume(clamp(percent), "output")

    def set_output_mute(self, muted):
        self.w.set_mute(muted, "output")

    def input_muted(self):
        return self.w.get_mute("input")

    def set_input_mute(self, muted):
        self.w.set_mute(muted, "input")

    def devices(self, kind):
        found = self.w.devices(kind)
        try:
            current = self.w.default_device(kind)[1]
        except OSError:
            current = None  # no default device of this kind (e.g. no microphone)
        return found, current

    def set_default(self, kind, device_id):
        self.w.set_default_device(device_id)

    def snapshot(self):
        return _snapshot(self)

    def app_volume(self, app):
        return self.w.get_app_volume(app)

    def set_app(self, app, percent=None, muted=None):
        return self.w.set_app_volume(app, None if percent is None else clamp(percent), muted)


def _snapshot(backend):
    """The fixed values, each None when it can't be read."""
    values = dict.fromkeys(FIXED_VALUES)
    try:
        values["volume"], values["output_muted"] = backend.output_volume()
    except Exception as e:
        _log_once(f"couldn't read the volume: {e}")
    try:
        values["mic_muted"] = backend.input_muted()
    except Exception as e:
        _log_once(f"couldn't read the microphone: {e}")
    for kind in ("output", "input"):
        try:
            values[kind + "_device"] = backend.devices(kind)[1]
        except Exception as e:
            _log_once(f"couldn't read the {kind} device: {e}")
    return values


# --- Shared logic ---------------------------------------------------------------

_backend = None
_backend_lock = threading.Lock()


def make_backend(platform=None):
    platform = platform or sys.platform
    if platform == "win32":
        return WindowsAudio()
    if platform == "darwin":
        return MacAudio()
    return LinuxAudio()


def backend():
    global _backend
    with _backend_lock:
        if _backend is None:
            _backend = make_backend()
        return _backend


_logged = set()


def _log_once(message):
    if message not in _logged:
        _logged.add(message)
        plugin.log(message, "warn")


def find_device(devices, wanted):
    """The (id, name) whose name is `wanted` (any case), or else contains it."""
    w = wanted.strip().lower()
    for device in devices:
        if device[1].lower() == w:
            return device
    for device in devices:
        if w in device[1].lower():
            return device
    return None


def choose_device(devices, current, wanted_text, kind="output"):
    """The device to switch to: the named one, or the next of several after the current one."""
    wanted = [w.strip() for w in (wanted_text or "").split(",") if w.strip()]
    if not wanted:
        raise ValueError("enter a device name first")
    chosen, missing = [], []
    for w in wanted:
        device = find_device(devices, w)
        if device is None:
            missing.append(w)
        elif device not in chosen:
            chosen.append(device)
    if not chosen:
        names = ", ".join(name for _, name in devices) or "none"
        raise RuntimeError(f"there's no {kind} device called “{wanted[0]}” (found: {names})")
    for i, (_, name) in enumerate(chosen):
        if name == current and len(chosen) > 1:
            return chosen[(i + 1) % len(chosen)], missing
    return chosen[0], missing


def want_muted(mode, currently_muted):
    if mode in (None, "", "toggle"):
        return not currently_muted
    if mode == "mute":
        return True
    if mode == "unmute":
        return False
    raise ValueError(f"unknown mode {mode}")


def parse_step(value):
    if value is None or value == "":
        return DEFAULT_STEP
    try:
        return abs(float(value))
    except (TypeError, ValueError):
        raise ValueError(f"the step “{value}” isn't a number")


# --- Values ---------------------------------------------------------------------

_published = {}
_refresh_lock = threading.Lock()


def show(value):
    if value is None:
        return UNKNOWN
    if isinstance(value, bool):
        return "on" if value else "off"
    return str(value)


def _publish(value_id, value, arg=None):
    key = value_id if arg is None else f"{value_id}:{arg}"
    text = show(value)
    if _published.get(key) != text:
        _published[key] = text
        plugin.set_value(value_id, text, arg=arg)


def refresh():
    """Read and publish the fixed values and the app volumes keys show."""
    with _refresh_lock:
        b = backend()
        for value_id, value in b.snapshot().items():
            _publish(value_id, value)
        for app in plugin.watched("app_volume"):
            try:
                state = b.app_volume(app)
                volume = None if state is None else state[0]
            except Unsupported:
                volume = None
            except Exception as e:
                _log_once(f"couldn't read the volume of {app}: {e}")
                volume = None
            _publish("app_volume", volume, arg=app)


def refresh_soon():
    """Show what an action changed now rather than at the next poll."""
    try:
        refresh()
    except Exception as e:
        plugin.log(f"couldn't refresh: {e}", "warn")


@plugin.on_watch
def watch_changed(values):
    watched = {"app_volume:" + app for app in plugin.watched("app_volume")}
    with _refresh_lock:
        for key in [k for k in _published if k.startswith("app_volume:") and k not in watched]:
            del _published[key]


@plugin.every(POLL_SECONDS)
def poll():
    refresh()


# --- Actions --------------------------------------------------------------------


def _act(fn):
    """Run an action; Unsupported becomes a warning on the key."""
    try:
        warning = fn()
    except Unsupported as e:
        return str(e)
    refresh_soon()
    return warning


@plugin.action("mic-mute")
def mic_mute(settings):
    b = backend()
    return _act(lambda: b.set_input_mute(want_muted(settings.get("mode"), b.input_muted())))


@plugin.action("output-mute")
def output_mute(settings):
    b = backend()
    return _act(lambda: b.set_output_mute(want_muted(settings.get("mode"), b.output_volume()[1])))


def _step(settings, sign):
    b = backend()
    step = parse_step(settings.get("step"))
    return _act(lambda: b.set_volume(clamp(b.output_volume()[0] + sign * step)))


@plugin.action("volume-up")
def volume_up(settings):
    return _step(settings, +1)


@plugin.action("volume-down")
def volume_down(settings):
    return _step(settings, -1)


@plugin.action("volume")
def volume(settings):
    b = backend()
    return _act(lambda: b.set_volume(clamp(settings.get("value", 0))))


def _switch(settings, kind):
    b = backend()

    def go():
        devices, current = b.devices(kind)
        (device_id, name), missing = choose_device(devices, current, settings.get("devices"), kind)
        b.set_default(kind, device_id)
        plugin.log(f"switched the {kind} to {name}")
        if missing:
            return "not found: " + ", ".join(missing)
        return None

    return _act(go)


@plugin.action("output-device")
def output_device(settings):
    return _switch(settings, "output")


@plugin.action("input-device")
def input_device(settings):
    return _switch(settings, "input")


def _app_name(settings):
    app = str(settings.get("app") or "").strip()
    if not app:
        raise ValueError("enter the app's program name first")
    return app


@plugin.action("app-volume")
def app_volume(settings):
    b = backend()
    app = _app_name(settings)

    def go():
        if not b.set_app(app, percent=clamp(settings.get("value", 0))):
            return f"{app} isn't playing any sound right now"
        return None

    return _act(go)


@plugin.action("app-mute")
def app_mute(settings):
    b = backend()
    app = _app_name(settings)

    def go():
        state = b.app_volume(app)
        if state is None:
            return f"{app} isn't playing any sound right now"
        b.set_app(app, muted=want_muted(settings.get("mode"), state[1]))
        return None

    return _act(go)


@plugin.on_start
def started():
    if isinstance(backend(), LinuxAudio) and not (backend().wpctl or backend().pactl):
        plugin.log("neither wpctl nor pactl is installed, so nothing can be shown or changed", "warn")
    if isinstance(backend(), MacAudio) and not backend().switch:
        plugin.log(SWITCH_AUDIO_HINT + "; volume and mute work without it", "info")


if __name__ == "__main__":
    plugin.run()
