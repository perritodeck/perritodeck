"""Named counters (death counters, tallies). Standard library only.

Counts are kept in counters-state.json in the PerritoDeck config folder. With the
"Text file folder" setting, each count is also written to <folder>/<name>.txt so OBS
(or anything else) can show it with a text source that reads from a file.
"""

import json
import os
import re
import sys
import threading

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()

STATE_FILE = None  # None means counters-state.json in config_dir(); tests point it elsewhere

_lock = threading.RLock()
counts = {}


def config_dir():
    """PerritoDeck's per-user config folder for plugin data."""
    if sys.platform.startswith("win"):
        base = os.environ.get("APPDATA") or os.path.join(os.path.expanduser("~"), "AppData", "Roaming")
    elif sys.platform == "darwin":
        base = os.path.join(os.path.expanduser("~"), "Library", "Application Support")
    else:
        base = os.environ.get("XDG_CONFIG_HOME") or os.path.join(os.path.expanduser("~"), ".config")
    return os.path.join(base, "PerritoDeck")


def state_path():
    return STATE_FILE or os.path.join(config_dir(), "counters-state.json")


def load():
    global counts
    try:
        with open(state_path(), encoding="utf-8") as f:
            data = json.load(f)
        data = data.get("counters") if isinstance(data, dict) else None
    except (OSError, ValueError):
        data = None
    with _lock:
        counts = {str(k): int(v) for k, v in (data or {}).items() if isinstance(v, (int, float))}


def save():
    path = state_path()
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        tmp = path + ".tmp"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump({"counters": counts}, f, indent=1)
        os.replace(tmp, path)
    except OSError as e:
        plugin.log(f"couldn't save the counters to {path}: {e}", "warn")


def file_name(name):
    """A safe file name for a counter: 'Deaths: boss 2' -> 'Deaths_ boss 2.txt'."""
    safe = re.sub(r'[<>:"/\\|?*\x00-\x1f]', "_", name).strip(" .") or "counter"
    return safe + ".txt"


def write_text_file(name, value):
    folder = str(plugin.settings.get("folder") or "").strip()
    if not folder:
        return None
    path = os.path.join(os.path.expanduser(folder), file_name(name))
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(str(value))
    except OSError as e:
        plugin.log(f"couldn't write {path}: {e}", "warn")
        return f"couldn't write the text file: {e.strerror or e}"
    return None


def name_of(settings):
    name = str(settings.get("name") or "").strip()
    if not name:
        raise ValueError("give the counter a name")
    return name


def whole_number(settings, key, default):
    raw = settings.get(key)
    if raw is None or (isinstance(raw, str) and not raw.strip()):
        return default
    try:
        value = float(raw)
    except (TypeError, ValueError):
        raise ValueError(f"“{raw}” isn't a number") from None
    if value != int(value):
        raise ValueError(f"“{raw}” isn't a whole number")
    return int(value)


def change(name, fn):
    """Set counter `name` to fn(old count), save, publish and write its text file."""
    with _lock:
        counts[name] = fn(counts.get(name, 0))
        value = counts[name]
        save()
    if name in plugin.watched("count"):
        plugin.set_value("count", value, arg=name)
    return write_text_file(name, value)


@plugin.action("increment")
def increment(settings):
    step = whole_number(settings, "step", 1)
    return change(name_of(settings), lambda n: n + step)


@plugin.action("decrement")
def decrement(settings):
    step = whole_number(settings, "step", 1)
    floor = 0 if settings.get("stop_at_zero") else None

    def down(n):
        n -= step
        return n if floor is None else max(floor, n)

    return change(name_of(settings), down)


@plugin.action("reset")
def reset(settings):
    return change(name_of(settings), lambda n: 0)


@plugin.action("set")
def set_to(settings):
    value = whole_number(settings, "value", None)
    if value is None:
        raise ValueError("set the number to count from")
    return change(name_of(settings), lambda n: value)


def publish_watched():
    with _lock:
        snapshot = {name: counts.get(name, 0) for name in plugin.watched("count")}
    for name, value in snapshot.items():
        plugin.set_value("count", value, arg=name)


@plugin.on_start
def started():
    load()
    publish_watched()
    configure(plugin.settings)


@plugin.on_watch
def watching(_values):
    publish_watched()


@plugin.on_settings
def configure(settings):
    # A new text file folder gets every count right away.
    if str(settings.get("folder") or "").strip():
        with _lock:
            snapshot = dict(counts)
        for name, value in snapshot.items():
            write_text_file(name, value)


if __name__ == "__main__":
    plugin.run()
