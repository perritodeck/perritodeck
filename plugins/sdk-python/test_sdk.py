"""Run a tiny plugin built on the SDK and talk to it like the host does."""

import json
import os
import subprocess
import sys
import tempfile
import time
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


WATCHER = """
import sys
sys.path.insert(0, %r)
from perritodeck_plugin import Plugin
p = Plugin()
@p.on_settings
def configured(s):
    p.set_value("server", s.get("url", "none"))
@p.on_watch
def watching(values):
    for host in p.watched("up"):
        p.set_value("up", "yes", arg=host)
p.run()
""" % HERE


class SettingsAndWatchTest(unittest.TestCase):
    def test_settings_and_watch(self):
        proc = subprocess.Popen(
            [sys.executable, "-c", WATCHER], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True
        )

        def send(m):
            proc.stdin.write(json.dumps(m) + "\n")
            proc.stdin.flush()

        def receive():
            return json.loads(proc.stdout.readline())

        send({"event": "hello", "protocol": 1, "host": "test", "settings": {"url": "https://pve"}})
        self.assertEqual(receive(), {"event": "value", "id": "server", "value": "https://pve"})
        self.assertEqual(receive(), {"event": "ready"})
        send({"event": "settings", "settings": {}})
        self.assertEqual(receive(), {"event": "value", "id": "server", "value": "none"})
        send({"event": "watch", "values": ["up:nas", "other:x", "up:10.0.0.1"]})
        self.assertEqual(receive(), {"event": "value", "id": "up:10.0.0.1", "value": "yes"})
        self.assertEqual(receive(), {"event": "value", "id": "up:nas", "value": "yes"})
        send({"event": "shutdown"})
        self.assertEqual(proc.wait(timeout=5), 0)


def start(script, env=None):
    """Start a plugin the way a Windows host on a cp1252 locale would, bytes in and out."""
    full = dict(os.environ, **(env or {}))
    full.pop("PYTHONUTF8", None)
    return subprocess.Popen(
        [sys.executable, "-X", "utf8=0", "-c", script],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        env=full,
    )


class Pipe:
    def __init__(self, proc):
        self.proc = proc

    def send(self, m):
        self.proc.stdin.write((json.dumps(m, ensure_ascii=False) + "\n").encode("utf-8"))
        self.proc.stdin.flush()

    def receive(self):
        return json.loads(self.proc.stdout.readline().decode("utf-8"))


ECHO = """
import sys
sys.path.insert(0, %r)
from perritodeck_plugin import Plugin
p = Plugin()
@p.on_settings
def configured(s):
    p.set_value("name", s.get("name", ""))
@p.action("echo")
def echo(settings):
    p.log(settings["text"])
    return settings["text"] + " \U0001F44D"
p.run()
""" % HERE


class EncodingTest(unittest.TestCase):
    def test_non_ascii_on_a_cp1252_locale(self):
        proc = start(ECHO, {"PYTHONIOENCODING": "cp1252"})
        self.addCleanup(proc.stdout.close)
        self.addCleanup(proc.stdin.close)
        pipe = Pipe(proc)
        pipe.send({"event": "hello", "protocol": 1, "host": "t", "settings": {"name": "Café ☕ 日本"}})
        self.assertEqual(pipe.receive(), {"event": "value", "id": "name", "value": "Café ☕ 日本"})
        self.assertEqual(pipe.receive(), {"event": "ready"})
        pipe.send({"event": "run", "id": 1, "action": "echo", "settings": {"text": "🎙️ Живой"}})
        self.assertEqual(pipe.receive(), {"event": "log", "level": "info", "message": "🎙️ Живой"})
        self.assertEqual(pipe.receive(), {"event": "result", "id": 1, "ok": True, "message": "🎙️ Живой 👍"})
        pipe.send({"event": "shutdown"})
        self.assertEqual(proc.wait(timeout=5), 0)

    def test_bad_bytes_do_not_kill_the_plugin(self):
        proc = start(ECHO, {"PYTHONIOENCODING": "cp1252"})
        self.addCleanup(proc.stdout.close)
        self.addCleanup(proc.stdin.close)
        proc.stdin.write(b"\xff\xfe not json\n")
        pipe = Pipe(proc)
        pipe.send({"event": "hello", "protocol": 1, "host": "t", "settings": {}})
        self.assertEqual(pipe.receive()["id"], "name")
        self.assertEqual(pipe.receive(), {"event": "ready"})
        pipe.send({"event": "shutdown"})
        self.assertEqual(proc.wait(timeout=5), 0)


DATA = """
import os, sys
sys.path.insert(0, %r)
from perritodeck_plugin import Plugin
p = Plugin()
@p.on_start
def go():
    p.set_value("dir", p.data_dir)
    p.set_value("exists", os.path.isdir(p.data_dir))
p.run()
""" % HERE


class DataDirTest(unittest.TestCase):
    def test_host_folder_is_used_and_created(self):
        with tempfile.TemporaryDirectory() as tmp:
            folder = os.path.join(tmp, "plugin-data", "dev.example")
            proc = start(DATA, {"PERRITODECK_PLUGIN_DATA": folder})
            self.addCleanup(proc.stdout.close)
            self.addCleanup(proc.stdin.close)
            pipe = Pipe(proc)
            pipe.send({"event": "hello", "protocol": 1, "host": "t"})
            self.assertEqual(pipe.receive(), {"event": "value", "id": "dir", "value": folder})
            self.assertEqual(pipe.receive(), {"event": "value", "id": "exists", "value": "True"})
            self.assertEqual(pipe.receive(), {"event": "ready"})
            pipe.send({"event": "shutdown"})
            self.assertEqual(proc.wait(timeout=5), 0)

    def test_fallback_outside_the_host(self):
        import perritodeck_plugin as sdk

        with tempfile.TemporaryDirectory() as tmp:
            old = dict(os.environ)
            try:
                os.environ.pop(sdk.DATA_ENV, None)
                os.environ.update({"XDG_DATA_HOME": tmp, "LOCALAPPDATA": tmp, "HOME": tmp, "USERPROFILE": tmp})
                folder = sdk.Plugin().data_dir
            finally:
                os.environ.clear()
                os.environ.update(old)
            self.assertTrue(os.path.isdir(folder))
            self.assertTrue(folder.startswith(tmp), folder)
            self.assertIn("plugin-data", folder)


SLOW = """
import sys, time
sys.path.insert(0, %r)
from perritodeck_plugin import Plugin
p = Plugin()
@p.on_settings
def configured(s):
    if s.get("slow"):
        time.sleep(1.5)
    p.set_value("settings", s.get("n", 0))
@p.on_watch
def watching(values):
    p.set_value("watch", ",".join(values))
@p.action("now")
def now(settings):
    return "seen %%s" %% p.settings.get("n")
p.run()
""" % HERE


class OrderingTest(unittest.TestCase):
    def test_slow_callback_does_not_block_actions_and_keeps_order(self):
        proc = start(SLOW)
        self.addCleanup(proc.stdout.close)
        self.addCleanup(proc.stdin.close)
        pipe = Pipe(proc)
        pipe.send({"event": "hello", "protocol": 1, "host": "t", "settings": {"n": 0}})
        self.assertEqual(pipe.receive(), {"event": "value", "id": "settings", "value": "0"})
        self.assertEqual(pipe.receive(), {"event": "ready"})
        pipe.send({"event": "settings", "settings": {"n": 1, "slow": True}})
        pipe.send({"event": "watch", "values": ["a:1"]})
        pipe.send({"event": "settings", "settings": {"n": 2}})
        began = time.monotonic()
        pipe.send({"event": "run", "id": 7, "action": "now", "settings": {}})
        # The action answers at once (with the newest settings) while the slow callback runs.
        self.assertEqual(pipe.receive(), {"event": "result", "id": 7, "ok": True, "message": "seen 2"})
        self.assertLess(time.monotonic() - began, 1.0)
        # Callbacks still run in the order the host sent them.
        self.assertEqual(pipe.receive(), {"event": "value", "id": "settings", "value": "1"})
        self.assertEqual(pipe.receive(), {"event": "value", "id": "watch", "value": "a:1"})
        self.assertEqual(pipe.receive(), {"event": "value", "id": "settings", "value": "2"})
        pipe.send({"event": "shutdown"})
        self.assertEqual(proc.wait(timeout=5), 0)


if __name__ == "__main__":
    unittest.main()
