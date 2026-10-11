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

The host talks to the plugin over stdin/stdout, one JSON object per line; see
PROTOCOL.md. Don't print to stdout yourself: use plugin.log (or print to stderr).

MIT licensed. Copy this file into your plugin's folder next to your script.
"""

import json
import sys
import threading
import traceback

PROTOCOL = 1
__version__ = "1.0.0"


class Plugin:
    def __init__(self):
        self._actions = {}
        self._on_start = []
        self._lock = threading.Lock()
        self.host_version = None

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

    def set_value(self, value_id, value):
        """Publish a live value; key titles show it as {plugin:<plugin id>:<value id>}."""
        self._send({"event": "value", "id": value_id, "value": str(value)})

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
                for fn in self._on_start:
                    try:
                        fn()
                    except Exception:
                        traceback.print_exc(file=sys.stderr)
                self._send({"event": "ready"})
            elif event == "run":
                threading.Thread(
                    target=self._run_action,
                    args=(message.get("id"), message.get("action"), message.get("settings")),
                    daemon=True,
                ).start()
            elif event == "shutdown":
                break
