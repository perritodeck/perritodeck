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

The host talks to the plugin over stdin/stdout, one JSON object per line; see
PROTOCOL.md. Don't print to stdout yourself: use plugin.log (or print to stderr).

MIT licensed. Copy this file into your plugin's folder next to your script.
"""

import json
import sys
import threading
import traceback

PROTOCOL = 1
__version__ = "1.1.0"


class Plugin:
    def __init__(self):
        self._actions = {}
        self._on_start = []
        self._on_settings = []
        self._on_watch = []
        self._timers = []
        self._lock = threading.Lock()
        self._started = threading.Event()
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

    def run(self):
        """Talk to the host until it says goodbye. Each action runs on its own thread."""
        for line in sys.stdin:
            try:
                message = json.loads(line)
            except ValueError:
                continue
            event = message.get("event")
            if event == "hello":
                self.host_version = message.get("host")
                self.settings = message.get("settings") or {}
                self._call(self._on_settings, self.settings)
                self._call(self._on_start)
                self._send({"event": "ready"})
                if not self._started.is_set():
                    self._started.set()
                    for seconds, fn, now in self._timers:
                        threading.Thread(target=self._timer, args=(seconds, fn, now), daemon=True).start()
            elif event == "settings":
                self.settings = message.get("settings") or {}
                self._call(self._on_settings, self.settings)
            elif event == "watch":
                self.watching = set(message.get("values") or [])
                self._call(self._on_watch, sorted(self.watching))
            elif event == "run":
                threading.Thread(
                    target=self._run_action,
                    args=(message.get("id"), message.get("action"), message.get("settings")),
                    daemon=True,
                ).start()
            elif event == "shutdown":
                break
