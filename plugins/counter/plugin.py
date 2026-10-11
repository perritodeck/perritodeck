"""Named counters (death counters, tallies). Standard library only.

Counts are kept in counters-state.json in the plugin's data folder (plugin.data_dir,
which the host gives each plugin); a counters-state.json in the old location (the
PerritoDeck config folder, see legacy_dir) is read once and moved over. With the
"Text file folder" setting, each count is also written to <folder>/<name>.txt so OBS
(or anything else) can show it with a text source that reads from a file. Every file
is written atomically (a temp file in the same folder, then os.replace).
"""

import json
import os
import re
import sys
import tempfile
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()

STATE_FILE = None  # None means counters-state.json in plugin.data_dir; tests point it elsewhere
FILE_NAME = "counters-state.json"

_lock = threading.RLock()
counts = {}


def legacy_dir():
    """Where older versions kept counters-state.json (read once, as a migration)."""
    if sys.platform.startswith("win"):
        base = os.environ.get("APPDATA") or os.path.join(os.path.expanduser("~"), "AppData", "Roaming")
    elif sys.platform == "darwin":
        base = os.path.join(os.path.expanduser("~"), "Library", "Application Support")
    else:
        base = os.environ.get("XDG_CONFIG_HOME") or os.path.join(os.path.expanduser("~"), ".config")
    return os.path.join(base, "PerritoDeck")


def state_path():
    return STATE_FILE or os.path.join(plugin.data_dir, FILE_NAME)


def load():
    global counts
    path = state_path()
    sources = [path]
    if STATE_FILE is None and not os.path.exists(path):
        sources.append(os.path.join(legacy_dir(), FILE_NAME))
    data, migrated = None, False
    for source in sources:
        try:
            with open(source, encoding="utf-8") as f:
                data = json.load(f)
        except (OSError, ValueError):
            continue
        migrated = source != path
        break
    data = data.get("counters") if isinstance(data, dict) else None
    with _lock:
        counts = {str(k): int(v) for k, v in (data or {}).items() if isinstance(v, (int, float))}
        if migrated:
            plugin.log(f"moved the counters from {sources[-1]} to {path}")
            save()


def save():
    # Under the lock, so two saves can't interleave or write an older count last.
    with _lock:
        try:
            write_atomic(state_path(), json.dumps({"counters": counts}, indent=1))
        except OSError as e:
            plugin.log(f"couldn't save the counters: {e}", "warn")


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
        write_atomic(path, str(value))  # OBS never reads a half-written file
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
        # Still under the lock, so a slower press can't leave an older count in the file.
        problem = write_text_file(name, value)
    if name in plugin.watched("count"):
        plugin.set_value("count", value, arg=name)
    return problem


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
            for name, value in counts.items():
                write_text_file(name, value)


if __name__ == "__main__":
    plugin.run()
