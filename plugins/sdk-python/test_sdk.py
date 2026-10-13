"""Run a tiny plugin built on the SDK and talk to it like the host does."""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

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


CHATTY = """
import sys, threading
sys.path.insert(0, %r)
from perritodeck_plugin import Plugin
p = Plugin()
def chatter():
    while True:
        p.set_value("n", "x" * 200)
@p.on_start
def go():
    threading.Thread(target=chatter, daemon=True).start()
p.run()
""" % HERE


class ShutdownTest(unittest.TestCase):
    def test_exits_cleanly_while_another_thread_is_writing(self):
        # Python exiting while a daemon thread is inside a write to stdout is a fatal
        # error at shutdown (exit code 0xC0000409 on Windows, SIGABRT elsewhere).
        for _ in range(5):
            proc = start(CHATTY)
            pipe = Pipe(proc)
            pipe.send({"event": "hello", "protocol": 1, "host": "t"})
            for _ in range(50):
                pipe.receive()
            pipe.send({"event": "shutdown"})
            proc.communicate(timeout=10)  # keeps reading, as the host does
            self.assertEqual(proc.returncode, 0)


def remove_soon(folder):
    """Delete a folder a program started in it may still be leaving (Windows keeps a
    process's working folder busy until it has exited)."""
    deadline = time.monotonic() + 5
    while True:
        try:
            shutil.rmtree(folder)
            return
        except FileNotFoundError:
            return
        except OSError:
            if time.monotonic() > deadline:
                raise
            time.sleep(0.1)


class ProgramsForTheUserTest(unittest.TestCase):
    """open_for_user and start_for_user: programs that outlive the plugin."""

    def setUp(self):
        import perritodeck_plugin as sdk

        self.sdk = sdk
        self.home = os.path.expanduser("~")
        self.folder = os.path.abspath(tempfile.mkdtemp())
        self.addCleanup(remove_soon, self.folder)
        self.exe = os.path.join(self.folder, "app.exe")
        open(self.exe, "w").close()

    def on(self, platform):
        """Pretend to be on `platform`, with subprocess.Popen faked."""
        patchers = [mock.patch.object(self.sdk.sys, "platform", platform), mock.patch.object(self.sdk.subprocess, "Popen")]
        for p in patchers:
            p.start()
            self.addCleanup(p.stop)
        return self.sdk.subprocess.Popen

    def test_the_program_a_command_starts(self):
        program = self.sdk._program
        self.assertEqual(program(["C:\\Tools\\app.exe", "-x"]), "C:\\Tools\\app.exe")
        self.assertEqual(program('"C:\\Program Files\\OBS\\obs64.exe" --startrecording'), "C:\\Program Files\\OBS\\obs64.exe")
        self.assertEqual(program("notepad notes.txt"), "notepad")
        self.assertEqual(program("  "), "")
        self.assertEqual(program([]), "")

    def test_it_starts_in_its_own_folder_or_home_never_the_plugins(self):
        self.assertEqual(self.sdk._start_folder(self.exe), self.folder)
        self.assertEqual(self.sdk._start_folder("spotify:"), self.home)
        self.assertEqual(self.sdk._start_folder(self.folder), self.home)  # a folder isn't a program
        self.assertEqual(self.sdk._start_folder(""), self.home)

    def test_start_on_windows_leaves_the_job_with_a_console_of_its_own(self):
        popen = self.on("win32")
        self.sdk.start_for_user([self.exe, "--profile", "work"])
        args, kwargs = popen.call_args
        self.assertEqual(args[0], [self.exe, "--profile", "work"])
        self.assertEqual(kwargs["cwd"], self.folder)
        self.assertEqual(kwargs["creationflags"], 0x01000000 | 0x10)  # breakaway, new console
        self.assertTrue(kwargs["close_fds"])
        self.assertNotIn("stdout", kwargs)  # never the plugin's pipes; its new console's
        self.sdk.start_for_user('"%s" --profile work' % self.exe)
        self.assertEqual(popen.call_args.kwargs["cwd"], self.folder)

    def test_start_inside_a_job_that_cant_be_left(self):
        popen = self.on("win32")
        popen.side_effect = [PermissionError(13, "Access is denied"), mock.DEFAULT]
        self.sdk.start_for_user(["notepad"])
        self.assertEqual([c.kwargs["creationflags"] for c in popen.call_args_list], [0x01000010, 0x10])
        self.assertEqual(popen.call_args.kwargs["cwd"], self.home)

    def test_start_elsewhere_in_a_session_of_its_own(self):
        popen = self.on("linux")
        self.sdk.start_for_user(["flatpak", "run", "com.spotify.Client"])
        kwargs = popen.call_args.kwargs
        self.assertTrue(kwargs["start_new_session"])
        self.assertEqual(kwargs["stdout"], subprocess.DEVNULL)  # never the protocol pipe
        self.assertEqual(kwargs["cwd"], self.home)
        self.assertNotIn("creationflags", kwargs)

    def test_open_hands_over_to_the_desktop(self):
        for platform, opener in (("darwin", "open"), ("linux", "xdg-open")):
            with mock.patch.object(self.sdk.sys, "platform", platform), mock.patch.object(self.sdk.subprocess, "Popen") as popen:
                popen.return_value.communicate.return_value = (None, b"")
                popen.return_value.returncode = 0
                self.sdk.open_for_user(self.exe)
            self.assertEqual(popen.call_args.args[0], [opener, self.exe])
            self.assertEqual(popen.call_args.kwargs["cwd"], self.folder)
        popen = self.on("win32")
        popen.return_value.communicate.return_value = (None, b"")
        popen.return_value.returncode = 0
        self.sdk.open_for_user("https://example.com/?a=1&b=2")
        args, kwargs = popen.call_args
        # Opened by a helper that left the job, so whatever it starts is outside too.
        self.assertEqual(args[0][0], sys.executable)
        self.assertEqual(args[0][-1], "https://example.com/?a=1&b=2")
        self.assertEqual(kwargs["creationflags"], 0x01000000 | 0x08000000)  # breakaway, no window
        self.assertEqual(kwargs["cwd"], self.home)
        self.assertEqual(kwargs["stdout"], subprocess.DEVNULL)

    def test_open_failures_raise_and_slow_opens_dont(self):
        popen = self.on("win32")
        popen.return_value.communicate.return_value = (None, b"[WinError 2] The system cannot find the file specified: 'x'")
        popen.return_value.returncode = 1
        with self.assertRaisesRegex(OSError, "WinError 2"):
            self.sdk.open_for_user("x")
        popen.return_value.communicate.side_effect = subprocess.TimeoutExpired("python", 5)
        self.sdk.open_for_user("x")  # still opening: not an error
        popen.return_value.kill.assert_not_called()

    def test_open_without_xdg_open_asks_the_standard_library(self):
        import webbrowser

        popen = self.on("linux")
        popen.side_effect = FileNotFoundError("xdg-open")
        with mock.patch.object(webbrowser, "open", return_value=True) as browser:
            self.sdk.open_for_user("https://example.com")
        browser.assert_called_once_with("https://example.com")
        with mock.patch.object(webbrowser, "open", return_value=False):
            with self.assertRaises(OSError):
                self.sdk.open_for_user("https://example.com")

    @unittest.skipUnless(sys.platform == "win32", "Windows only")
    def test_open_on_windows(self):
        with self.assertRaisesRegex(OSError, "WinError 2"):
            self.sdk.open_for_user(os.path.join(self.folder, "missing.exe"))
        import winreg

        try:
            winreg.QueryValue(winreg.HKEY_CLASSES_ROOT, ".pyw")
        except OSError:
            self.skipTest("no program opens .pyw files")
        # A windowless script that says which folder it was started in.
        said = os.path.join(self.folder, "cwd.txt")
        script = os.path.join(self.folder, "where.pyw")
        with open(script, "w") as f:
            f.write("import os\nopen(%r, 'w').write(os.getcwd())\nos.replace(%r, %r)\n" % (said + ".tmp", said + ".tmp", said))
        self.sdk.open_for_user(script)
        deadline = time.monotonic() + 15
        while not os.path.exists(said) and time.monotonic() < deadline:
            time.sleep(0.1)
        with open(said) as f:
            self.assertEqual(os.path.normcase(f.read()), os.path.normcase(self.folder))


if __name__ == "__main__":
    unittest.main()
