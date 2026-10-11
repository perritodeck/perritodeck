"""Countdown timers, stopwatches and a Pomodoro timer. Standard library only.

Timers and stopwatches have names; key titles pick one with the value's argument,
e.g. {plugin:dev.perritodeck.timer:remaining:tea}. Everything is kept in
timer-state.json in the plugin's data folder (plugin.data_dir, which the host gives
each plugin), with wall-clock end times, so a timer keeps counting while the plugin
(or the computer's host app) restarts. A timer-state.json in the old location (the
PerritoDeck config folder, see legacy_dir) is read once and moved over.
"""

import json
import math
import os
import subprocess
import sys
import tempfile
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()

STATE_FILE = None  # None means timer-state.json in plugin.data_dir; tests point it elsewhere
FILE_NAME = "timer-state.json"
clock = time.time  # tests replace this

PHASES = {"idle": "Ready", "work": "Work", "short": "Short break", "long": "Long break"}
POMODORO_DEFAULTS = {"work": 25, "short": 5, "long": 15, "rounds": 4}

_lock = threading.RLock()
_wake = threading.Event()
_published = {}
_ticker = None
state = {"timers": {}, "stopwatches": {}, "pomodoro": None}  # filled by load()


# ---------------------------------------------------------------- storage


def legacy_dir():
    """Where older versions kept timer-state.json (read once, as a migration)."""
    if sys.platform.startswith("win"):
        base = os.environ.get("APPDATA") or os.path.join(os.path.expanduser("~"), "AppData", "Roaming")
    elif sys.platform == "darwin":
        base = os.path.join(os.path.expanduser("~"), "Library", "Application Support")
    else:
        base = os.environ.get("XDG_CONFIG_HOME") or os.path.join(os.path.expanduser("~"), ".config")
    return os.path.join(base, "PerritoDeck")


def state_path():
    return STATE_FILE or os.path.join(plugin.data_dir, FILE_NAME)


def read_state():
    """The saved state, from the data folder or (once) from the old location."""
    path = state_path()
    sources = [path]
    if STATE_FILE is None and not os.path.exists(path):
        sources.append(os.path.join(legacy_dir(), FILE_NAME))
    for source in sources:
        try:
            with open(source, encoding="utf-8") as f:
                data = json.load(f)
        except (OSError, ValueError):
            continue
        if source != path:
            plugin.log(f"moved the timers from {source} to {path}")
        return data if isinstance(data, dict) else {}
    return {}


def new_pomodoro(config=None):
    """A Pomodoro that hasn't started, with the given lengths (minutes) and rounds."""
    p = {"phase": "idle", "round": 1, "end": None, "left": 0.0}
    p.update(config or POMODORO_DEFAULTS)
    p.setdefault("auto", True)
    return p


def load():
    global state
    data = read_state()
    pomodoro = data.get("pomodoro") if isinstance(data.get("pomodoro"), dict) else None
    with _lock:
        state = {
            "timers": dict(data.get("timers") or {}),
            "stopwatches": dict(data.get("stopwatches") or {}),
            "pomodoro": pomodoro or new_pomodoro(),
        }
        if not os.path.exists(state_path()) and (state["timers"] or state["stopwatches"] or pomodoro):
            save()  # finish a migration from the old location


def save():
    # Under the lock, so two saves can't interleave or write an older state last.
    with _lock:
        try:
            path = state_path()
            write_atomic(path, json.dumps(state, indent=1))
        except OSError as e:
            plugin.log(f"couldn't save the timers: {e}", "warn")


def write_atomic(path, text):
    """Write text to path all at once: a unique temp file in the same folder, then
    os.replace, so a crash or a second writer never leaves half a file."""
    folder = os.path.dirname(path) or "."
    os.makedirs(folder, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=folder, prefix="." + os.path.basename(path) + ".", suffix=".tmp")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(text)
        for attempt in range(5):
            try:
                os.replace(tmp, path)
                return
            except PermissionError:  # Windows: another program has the file open
                if attempt == 4:
                    raise
                time.sleep(0.02 * (attempt + 1))
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


# ---------------------------------------------------------------- helpers


def clock_text(seconds):
    """M:SS, or H:MM:SS from an hour up."""
    s = max(0, int(seconds))
    h, rest = divmod(s, 3600)
    m, s = divmod(rest, 60)
    return f"{h}:{m:02}:{s:02}" if h else f"{m}:{s:02}"


def countdown_text(seconds):
    # Round up, so a timer shows 0:00 only once it has really finished.
    return clock_text(math.ceil(round(max(0.0, seconds), 3)))


def number(settings, key, default=0.0):
    raw = settings.get(key)
    if raw is None or (isinstance(raw, str) and not raw.strip()):
        return default
    try:
        return float(raw)
    except (TypeError, ValueError):
        raise ValueError(f"“{raw}” isn't a number") from None


def name_of(settings):
    name = str(settings.get("name") or "").strip()
    if not name:
        raise ValueError("give the timer a name")
    return name


def length_of(settings):
    return number(settings, "minutes") * 60 + number(settings, "seconds")


def left(entry, now):
    """Seconds left on a countdown (or Pomodoro phase)."""
    if entry.get("end") is not None:
        return max(0.0, entry["end"] - now)
    return max(0.0, float(entry.get("left") or 0))


def elapsed(watch, now):
    total = float(watch.get("elapsed") or 0)
    if watch.get("start") is not None:
        total += max(0.0, now - watch["start"])
    return total


def on_off(flag):
    return "on" if flag else "off"


# ---------------------------------------------------------------- sound


def _popen(args):
    return subprocess.Popen(args, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def play_sound():
    """A short system sound, best effort, when the 'sound' setting is on."""
    if not plugin.settings.get("sound"):
        return
    try:
        if sys.platform.startswith("win"):
            import winsound

            winsound.MessageBeep()
        elif sys.platform == "darwin":
            _popen(["afplay", "/System/Library/Sounds/Glass.aiff"])
        else:
            sound = "/usr/share/sounds/freedesktop/stereo/complete.oga"
            for args in (["paplay", sound], ["pw-play", sound], ["canberra-gtk-play", "-i", "complete"]):
                try:
                    _popen(args)
                    return
                except OSError:
                    continue
    except Exception as e:  # a missing sound must never break a timer
        plugin.log(f"couldn't play a sound: {e}", "warn")


# ---------------------------------------------------------------- state changes (hold _lock)


def _finish(name, timer):
    timer.update(end=None, left=0.0, done=True)
    plugin.log(f"timer “{name}” finished")
    play_sound()


def _pomodoro_next(p, now, run):
    if p["phase"] == "work":
        p["phase"] = "long" if p["round"] % max(1, int(p["rounds"])) == 0 else "short"
    else:
        p["round"] = 1 if p["phase"] == "long" else p["round"] + 1
        p["phase"] = "work"
    length = float(p[p["phase"]]) * 60
    p["left"] = length
    p["end"] = now + length if run else None


def _advance(now):
    """Finish countdowns and Pomodoro phases whose time is up. True if something changed."""
    changed = False
    for name, t in state["timers"].items():
        if t.get("end") is not None and left(t, now) <= 0:
            _finish(name, t)
            changed = True
    p = state["pomodoro"]
    if p["phase"] != "idle" and p.get("end") is not None and left(p, now) <= 0:
        _pomodoro_next(p, now, bool(p.get("auto", True)))
        plugin.log(f"Pomodoro: {PHASES[p['phase']].lower()} now")
        play_sound()
        changed = True
    return changed


def _timer(name):
    t = state["timers"].get(name)
    if t is None:
        raise ValueError(f"there's no timer called “{name}” yet; start it first")
    return t


# ---------------------------------------------------------------- values


def values(now):
    """Every value keys use right now, as {full value id: text}."""
    out = {}
    timers, watches = state["timers"], state["stopwatches"]
    for name in plugin.watched("remaining"):
        t = timers.get(name)
        out[f"remaining:{name}"] = countdown_text(left(t, now)) if t else "–"
    for name in plugin.watched("running"):
        t, w = timers.get(name), watches.get(name)
        out[f"running:{name}"] = on_off((t and t.get("end") is not None) or (w and w.get("start") is not None))
    for name in plugin.watched("done"):
        out[f"done:{name}"] = on_off(timers.get(name, {}).get("done"))
    for name in plugin.watched("progress"):
        t = timers.get(name)
        duration = float(t.get("duration") or 0) if t else 0
        out[f"progress:{name}"] = str(round(100 * (duration - left(t, now)) / duration) if duration else 0)
    for name in plugin.watched("stopwatch"):
        w = watches.get(name)
        out[f"stopwatch:{name}"] = clock_text(elapsed(w, now)) if w else "0:00"
    for name in plugin.watched("lap"):
        w = watches.get(name)
        out[f"lap:{name}"] = clock_text(w["lap"]) if w and w.get("lap") is not None else "–"
    p = state["pomodoro"]
    length = float(p["work"]) * 60 if p["phase"] == "idle" else float(p[p["phase"]]) * 60
    remaining = length if p["phase"] == "idle" else left(p, now)
    out["pomodoro_phase"] = PHASES.get(p["phase"], "Ready")
    out["pomodoro_remaining"] = countdown_text(remaining)
    out["pomodoro_round"] = f"{p['round']}/{int(p['rounds'])}"
    out["pomodoro_running"] = on_off(p["phase"] != "idle" and p.get("end") is not None)
    out["pomodoro_progress"] = str(round(100 * (length - remaining) / length) if length else 0)
    return out


def publish(now=None):
    """Send the values that changed since last time."""
    now = clock() if now is None else now
    with _lock:
        current = values(now)
        for key in [k for k in _published if k not in current]:
            del _published[key]
        changes = {k: v for k, v in current.items() if _published.get(k) != v}
        _published.update(changes)
    for key, value in changes.items():
        plugin.set_value(key, value)


def anything_running():
    with _lock:
        p = state["pomodoro"]
        return (
            any(t.get("end") is not None for t in state["timers"].values())
            or any(w.get("start") is not None for w in state["stopwatches"].values())
            or (p["phase"] != "idle" and p.get("end") is not None)
        )


def tick(now=None):
    """Finish what's due, save if anything finished, and publish. Returns the delay to the
    next whole second of whatever is running, or None when nothing runs."""
    now = clock() if now is None else now
    with _lock:
        if _advance(now):
            save()
    publish(now)
    if not anything_running():
        return None
    delays = [1.0]
    with _lock:
        for t in state["timers"].values():
            if t.get("end") is not None:
                delays.append(left(t, now) % 1.0 or 1.0)
        for w in state["stopwatches"].values():
            if w.get("start") is not None:
                delays.append(1.0 - elapsed(w, now) % 1.0)
        p = state["pomodoro"]
        if p["phase"] != "idle" and p.get("end") is not None:
            delays.append(left(p, now) % 1.0 or 1.0)
    return max(0.05, min(delays) + 0.01)


def _tick_loop():
    while True:
        try:
            delay = tick()
        except Exception as e:
            plugin.log(f"timer tick failed: {e}", "error")
            delay = 1.0
        _wake.wait(delay)  # None: sleep until an action starts something
        _wake.clear()


def changed():
    """After an action: save, publish now and make sure the ticker is awake."""
    save()
    publish()
    _wake.set()


# ---------------------------------------------------------------- countdown actions


@plugin.action("start")
def start(settings):
    name = name_of(settings)
    now = clock()
    with _lock:
        length = length_of(settings)
        if length <= 0:
            length = float(state["timers"].get(name, {}).get("duration") or 0)
        if length <= 0:
            raise ValueError("set how long the timer runs")
        state["timers"][name] = {"duration": length, "left": length, "end": now + length, "done": False}
    changed()


@plugin.action("pause")
def pause(settings):
    name = name_of(settings)
    now = clock()
    with _lock:
        t = _timer(name)
        if t.get("end") is not None:
            t["left"], t["end"] = left(t, now), None
    changed()


@plugin.action("toggle")
def toggle(settings):
    name = name_of(settings)
    now = clock()
    with _lock:
        t = state["timers"].get(name)
        if t is None or t.get("done") or (t.get("end") is None and left(t, now) <= 0):
            return start(settings)
        if t.get("end") is not None:
            t["left"], t["end"] = left(t, now), None
        else:
            t["end"] = now + left(t, now)
    changed()


@plugin.action("reset")
def reset(settings):
    name = name_of(settings)
    with _lock:
        t = _timer(name)
        t.update(left=float(t.get("duration") or 0), end=None, done=False)
    changed()


@plugin.action("add")
def add(settings):
    name = name_of(settings)
    amount = length_of(settings)
    if not amount:
        raise ValueError("set how much time to add (use a minus sign to take time away)")
    now = clock()
    with _lock:
        t = _timer(name)
        if t.get("done") or left(t, now) <= 0:
            if amount < 0:
                return "the timer has already finished"
            t.update(duration=amount, left=amount, end=now + amount, done=False)
        elif t.get("end") is not None:
            t["end"] = max(now, t["end"] + amount)
        else:
            t["left"] = max(0.0, left(t, now) + amount)
            if t["left"] <= 0:
                _finish(name, t)
        t["duration"] = max(float(t.get("duration") or 0), left(t, now))
    changed()


# ---------------------------------------------------------------- stopwatch actions


@plugin.action("stopwatch-toggle")
def stopwatch_toggle(settings):
    name = name_of(settings)
    now = clock()
    with _lock:
        w = state["stopwatches"].setdefault(name, {"start": None, "elapsed": 0.0, "lap": None, "mark": 0.0})
        if w.get("start") is None:
            w["start"] = now
        else:
            w["elapsed"], w["start"] = elapsed(w, now), None
    changed()


@plugin.action("stopwatch-lap")
def stopwatch_lap(settings):
    name = name_of(settings)
    now = clock()
    with _lock:
        w = state["stopwatches"].get(name)
        if w is None:
            raise ValueError(f"there's no stopwatch called “{name}” yet; start it first")
        total = elapsed(w, now)
        w["lap"], w["mark"] = total - float(w.get("mark") or 0), total
    changed()


@plugin.action("stopwatch-reset")
def stopwatch_reset(settings):
    name = name_of(settings)
    now = clock()
    with _lock:
        w = state["stopwatches"].get(name)
        running = w is not None and w.get("start") is not None
        state["stopwatches"][name] = {"start": now if running else None, "elapsed": 0.0, "lap": None, "mark": 0.0}
    changed()


# ---------------------------------------------------------------- Pomodoro actions


def pomodoro_config(settings):
    config = {}
    for key, default in POMODORO_DEFAULTS.items():
        value = number(settings, key, default)
        if value <= 0:
            raise ValueError("Pomodoro lengths and rounds must be more than 0")
        config[key] = int(value) if key == "rounds" else value
    config["auto"] = bool(settings.get("auto", True))
    return config


@plugin.action("pomodoro-start")
def pomodoro_start(settings):
    now = clock()
    with _lock:
        p = state["pomodoro"]
        if p["phase"] == "idle":
            p = state["pomodoro"] = new_pomodoro(pomodoro_config(settings))
            p["phase"], p["left"] = "work", float(p["work"]) * 60
            p["end"] = now + p["left"]
        elif p.get("end") is not None:
            p["left"], p["end"] = left(p, now), None
        else:
            p["end"] = now + left(p, now)
    changed()


@plugin.action("pomodoro-skip")
def pomodoro_skip(settings):
    now = clock()
    with _lock:
        p = state["pomodoro"]
        if p["phase"] == "idle":
            return "the Pomodoro hasn't started"
        _pomodoro_next(p, now, True)
    changed()


@plugin.action("pomodoro-reset")
def pomodoro_reset(settings):
    with _lock:
        p = state["pomodoro"]
        state["pomodoro"] = new_pomodoro({k: p.get(k, v) for k, v in POMODORO_DEFAULTS.items()})
        state["pomodoro"]["auto"] = p.get("auto", True)
    changed()


# ---------------------------------------------------------------- host events


@plugin.on_start
def started():
    global _ticker
    load()
    _published.clear()  # a (re)started host wants every value again
    if _ticker is None:
        _ticker = threading.Thread(target=_tick_loop, daemon=True)
        _ticker.start()
    _wake.set()


@plugin.on_watch
def watching(_values):
    _wake.set()  # the ticker publishes the new values right away


state["pomodoro"] = new_pomodoro()

if __name__ == "__main__":
    plugin.run()
