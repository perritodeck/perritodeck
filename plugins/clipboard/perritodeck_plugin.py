"""PerritoDeck plugin SDK for Python (standard library only).

    from perritodeck_plugin import Plugin

    plugin = Plugin()

    @plugin.action("say-hello")
    def say_hello(settings):
        name = settings.get("name", "world")
        plugin.log(f"hello, {name}")
        # Return a string to show it as a warning on the key; raise to show an error.

    @plugin.on_start
    def started():
        plugin.set_value("greeting", "hi")

    plugin.run()

Settings and watched values (protocol 1, host 0.1+):

    @plugin.on_settings              # the plugin's own settings changed (Plugins page)
    def configure(settings): ...     # also readable any time as plugin.settings

    @plugin.every(5)                 # poll in the background every 5 s
    def poll():
        for host in plugin.watched("up"):            # titles use {plugin:<id>:up:<host>}
            plugin.set_value("up", check(host), arg=host)

Callbacks and threads:

- Each action runs on its own thread, so a slow action never holds up another.
- on_settings and on_watch callbacks run one at a time, in the order the host sent
  the messages, on one worker thread (the same one that runs on_start after hello).
  A slow callback delays the callbacks queued after it, but never the actions or
  the reading of new messages. plugin.settings and plugin.watching always hold the
  newest values, even while an older callback is still running.
- every() timers run on their own threads.

Files: plugin.data_dir is a folder the plugin may keep its own files in (counters,
state, caches). The host gives each plugin one (PERRITODECK_PLUGIN_DATA); run outside
the host, it falls back to a per-user folder (see _default_data_dir). It is created
the first time you use it.

stdin and stdout are switched to UTF-8 whatever the system locale is, so titles and
settings with emoji or non-Latin text work on Windows too.

The host talks to the plugin over stdin/stdout, one JSON object per line; see
PROTOCOL.md. Don't print to stdout yourself: use plugin.log (or print to stderr).

MIT licensed. Copy this file into your plugin's folder next to your script.
"""

import io
import json
import os
import queue
import sys
import threading
import traceback

PROTOCOL = 1
__version__ = "1.2.0"
DATA_ENV = "PERRITODECK_PLUGIN_DATA"


def _utf8_streams():
    """Read and write UTF-8 on stdin/stdout whatever the locale (cp1252 on Windows)."""
    for name, errors in (("stdin", "replace"), ("stdout", "replace")):
        stream = getattr(sys, name, None)
        if stream is None:
            continue
        try:
            if hasattr(stream, "reconfigure"):
                stream.reconfigure(encoding="utf-8", errors=errors)
            elif hasattr(stream, "buffer"):  # Python < 3.7
                setattr(sys, name, io.TextIOWrapper(stream.buffer, encoding="utf-8", errors=errors))
        except (AttributeError, ValueError, io.UnsupportedOperation):
            pass  # a replaced stream (tests, IDEs) that can't be changed; leave it


def _plugin_name():
    """The plugin's id from the manifest next to the script, or the folder's name."""
    folder = os.path.dirname(os.path.abspath(sys.argv[0] if sys.argv and sys.argv[0] else "."))
    try:
        with open(os.path.join(folder, "perritodeck-plugin.json"), encoding="utf-8") as f:
            plugin_id = json.load(f).get("id")
        if isinstance(plugin_id, str) and plugin_id:
            return plugin_id
    except (OSError, ValueError, AttributeError):
        pass
    return os.path.basename(folder) or "plugin"


def _default_data_dir(name):
    """Where plugin files go when the host didn't say (a plugin run by hand)."""
    home = os.path.expanduser("~")
    if sys.platform == "win32":
        base = os.environ.get("LOCALAPPDATA") or os.environ.get("APPDATA") or os.path.join(home, "AppData", "Local")
        return os.path.join(base, "PerritoDeck", "plugin-data", name)
    if sys.platform == "darwin":
        return os.path.join(home, "Library", "Application Support", "PerritoDeck", "plugin-data", name)
    base = os.environ.get("XDG_DATA_HOME") or os.path.join(home, ".local", "share")
    return os.path.join(base, "perritodeck", "plugin-data", name)


class Plugin:
    def __init__(self):
        _utf8_streams()
        self._actions = {}
        self._on_start = []
        self._on_settings = []
        self._on_watch = []
        self._timers = []
        self._lock = threading.Lock()
        self._started = threading.Event()
        self._events = queue.Queue()  # hello/settings/watch callbacks, in order
        self._worker = None
        self._data_dir = None
        self.host_version = None
        # The plugin's own settings, as saved on the Plugins page.
        self.settings = {}
        # Values with an argument that keys show, e.g. {"up:192.168.1.10", "up:nas"}.
        self.watching = set()

    def action(self, action_id):
        """Register a function(settings) for an action declared in the manifest."""

        def register(fn):
            self._actions[action_id] = fn
            return fn

        return register

    def on_start(self, fn):
        """Run fn() once the host has said hello (before any action)."""
        self._on_start.append(fn)
        return fn

    def on_settings(self, fn):
        """Run fn(settings) whenever the plugin's settings arrive or change."""
        self._on_settings.append(fn)
        return fn

    def on_watch(self, fn):
        """Run fn(values) when the set of watched values changes (see watched())."""
        self._on_watch.append(fn)
        return fn

    def every(self, seconds, now=True):
        """Run fn() every `seconds` on a background thread, once the host said hello.

        Errors are logged and the timer keeps going."""

        def register(fn):
            self._timers.append((seconds, fn, now))
            return fn

        return register

    @property
    def data_dir(self):
        """A folder for the plugin's own files, created if missing.

        The host sets PERRITODECK_PLUGIN_DATA to a per-plugin folder in its settings
        folder; outside the host this is a per-user folder named after the plugin id
        (Windows %LOCALAPPDATA%\\PerritoDeck\\plugin-data\\<id>, macOS
        ~/Library/Application Support/PerritoDeck/plugin-data/<id>, elsewhere
        $XDG_DATA_HOME or ~/.local/share/perritodeck/plugin-data/<id>)."""
        if self._data_dir is None:
            folder = os.environ.get(DATA_ENV) or _default_data_dir(_plugin_name())
            os.makedirs(folder, exist_ok=True)
            self._data_dir = folder
        return self._data_dir

    def watched(self, value_id):
        """The arguments keys use with a value: ["192.168.1.10", …] for up:<host>."""
        prefix = value_id + ":"
        return sorted(v[len(prefix):] for v in self.watching if v.startswith(prefix))

    def set_value(self, value_id, value, arg=None):
        """Publish a live value; key titles show it as {plugin:<plugin id>:<value id>}
        (or {plugin:<plugin id>:<value id>:<arg>} for a value with an argument)."""
        full = value_id if arg is None else f"{value_id}:{arg}"
        self._send({"event": "value", "id": full, "value": str(value)})

    def log(self, message, level="info"):
        self._send({"event": "log", "level": level, "message": str(message)})

    def _send(self, message):
        line = json.dumps(message, ensure_ascii=False)
        with self._lock:
            sys.stdout.write(line + "\n")
            sys.stdout.flush()

    def _run_action(self, request_id, action_id, settings):
        fn = self._actions.get(action_id)
        if fn is None:
            self._send({"event": "result", "id": request_id, "ok": False, "message": f"unknown action {action_id}"})
            return
        try:
            warning = fn(settings or {})
        except Exception as e:  # report every failure to the key
            traceback.print_exc(file=sys.stderr)
            self._send({"event": "result", "id": request_id, "ok": False, "message": str(e) or type(e).__name__})
            return
        result = {"event": "result", "id": request_id, "ok": True}
        if isinstance(warning, str) and warning:
            result["message"] = warning
        self._send(result)

    def _call(self, fns, *args):
        for fn in fns:
            try:
                fn(*args)
            except Exception:
                traceback.print_exc(file=sys.stderr)

    def _timer(self, seconds, fn, now):
        stop = threading.Event()
        if not now:
            stop.wait(seconds)
        while True:
            try:
                fn()
            except Exception:
                traceback.print_exc(file=sys.stderr)
            stop.wait(seconds)

    def _hello(self, settings):
        self._call(self._on_settings, settings)
        self._call(self._on_start)
        self._send({"event": "ready"})
        if not self._started.is_set():
            self._started.set()
            for seconds, fn, now in self._timers:
                threading.Thread(target=self._timer, args=(seconds, fn, now), daemon=True).start()

    def _work(self):
        # One thread runs hello, settings and watch callbacks in arrival order, so a
        # slow callback (a reconnect, a network check) never stops run() reading.
        while True:
            job = self._events.get()
            if job is None:
                return
            fn, args = job
            try:
                fn(*args)
            except Exception:
                traceback.print_exc(file=sys.stderr)

    def _queue(self, fn, *args):
        if self._worker is None:
            self._worker = threading.Thread(target=self._work, name="perritodeck-events", daemon=True)
            self._worker.start()
        self._events.put((fn, args))

    def run(self):
        """Talk to the host until it says goodbye. Each action runs on its own thread;
        on_start, on_settings and on_watch run in order on one worker thread."""
        for line in sys.stdin:
            try:
                message = json.loads(line)
            except ValueError:
                continue
            if not isinstance(message, dict):
                continue
            event = message.get("event")
            if event == "hello":
                self.host_version = message.get("host")
                self.settings = message.get("settings") or {}
                self._queue(self._hello, self.settings)
            elif event == "settings":
                self.settings = message.get("settings") or {}
                self._queue(self._call, self._on_settings, self.settings)
            elif event == "watch":
                self.watching = set(message.get("values") or [])
                self._queue(self._call, self._on_watch, sorted(self.watching))
            elif event == "run":
                threading.Thread(
                    target=self._run_action,
                    args=(message.get("id"), message.get("action"), message.get("settings")),
                    daemon=True,
                ).start()
            elif event == "shutdown":
                break
        if self._worker is not None:
            self._events.put(None)
