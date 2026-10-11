"""Run a tiny plugin built on the SDK and talk to it like the host does."""

import json
import os
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))

PLUGIN = """
import sys
sys.path.insert(0, %r)
from perritodeck_plugin import Plugin
p = Plugin()
@p.on_start
def go():
    p.set_value("ready", "yes")
@p.action("ok")
def ok(settings):
    return settings.get("warn")
@p.action("boom")
def boom(settings):
    raise RuntimeError("it broke")
p.run()
""" % HERE


class SdkTest(unittest.TestCase):
    def test_round_trip(self):
        proc = subprocess.Popen(
            [sys.executable, "-c", PLUGIN], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True
        )

        def send(m):
            proc.stdin.write(json.dumps(m) + "\n")
            proc.stdin.flush()

        def receive():
            return json.loads(proc.stdout.readline())

        send({"event": "hello", "protocol": 1, "host": "test"})
        self.assertEqual(receive(), {"event": "value", "id": "ready", "value": "yes"})
        self.assertEqual(receive(), {"event": "ready"})
        send({"event": "run", "id": 1, "action": "ok", "settings": {}})
        self.assertEqual(receive(), {"event": "result", "id": 1, "ok": True})
        send({"event": "run", "id": 2, "action": "ok", "settings": {"warn": "careful"}})
        self.assertEqual(receive(), {"event": "result", "id": 2, "ok": True, "message": "careful"})
        send({"event": "run", "id": 3, "action": "boom", "settings": {}})
        self.assertEqual(receive(), {"event": "result", "id": 3, "ok": False, "message": "it broke"})
        send({"event": "run", "id": 4, "action": "nope", "settings": {}})
        self.assertFalse(receive()["ok"])
        send({"event": "shutdown"})
        self.assertEqual(proc.wait(timeout=5), 0)


if __name__ == "__main__":
    unittest.main()
