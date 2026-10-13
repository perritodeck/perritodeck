import base64
import datetime
import os
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as shot  # noqa: E402
import winapi  # noqa: E402  (must import on every platform)


def done(stdout="", returncode=0, stderr=""):
    return subprocess.CompletedProcess([], returncode, stdout=stdout, stderr=stderr)


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.data = os.path.join(self.tmp.name, "data")
        os.makedirs(self.data)
        self.out = os.path.join(self.tmp.name, "shots")
        patcher = mock.patch.object(shot.plugin, "_data_dir", self.data)
        patcher.start()
        self.addCleanup(patcher.stop)
        patcher = mock.patch.object(shot.plugin, "set_value")
        self.set_value = patcher.start()
        self.addCleanup(patcher.stop)
        patcher = mock.patch.object(shot.plugin, "log")
        self.log = patcher.start()
        self.addCleanup(patcher.stop)
        shot.plugin.settings = {"folder": self.out}
        self.addCleanup(setattr, shot.plugin, "settings", {})

    def last(self):
        calls = [c.args[1] for c in self.set_value.call_args_list if c.args[0] == "last"]
        return calls[-1] if calls else None


class FolderTest(Base):
    def test_xdg_pictures(self):
        config = os.path.join(self.tmp.name, "config")
        os.makedirs(config)
        with open(os.path.join(config, "user-dirs.dirs"), "w") as f:
            f.write('# written by xdg-user-dirs-update\nXDG_DESKTOP_DIR="$HOME/Schreibtisch"\nXDG_PICTURES_DIR="$HOME/Bilder"\n')
        self.assertEqual(shot.xdg_pictures("/home/ana", config), "/home/ana/Bilder")
        self.assertEqual(shot.xdg_pictures("/home/ana", "/nonexistent"), os.path.join("/home/ana", "Pictures"))

    def test_folder_setting_and_default(self):
        self.assertEqual(shot.folder(), self.out)
        self.assertTrue(os.path.isdir(self.out))
        # ~ is HOME on Linux and macOS but USERPROFILE on Windows, where "~/caps" also keeps its "/".
        home = {"HOME": self.tmp.name, "USERPROFILE": self.tmp.name, "XDG_CONFIG_HOME": os.path.join(self.tmp.name, "none")}
        with mock.patch.dict(os.environ, home):
            self.assertEqual(os.path.normpath(shot.folder({"folder": "~/caps"})), os.path.join(self.tmp.name, "caps"))
            self.assertEqual(shot.folder({}, platform="linux"), os.path.join(self.tmp.name, "Pictures", "Screenshots"))
            self.assertEqual(shot.folder({}, platform="darwin"), os.path.join(self.tmp.name, "Pictures", "Screenshots"))

    def test_new_name_never_overwrites(self):
        os.makedirs(self.out)
        when = datetime.datetime(2026, 10, 11, 14, 3, 22)
        first = shot.new_name(self.out, when)
        self.assertEqual(os.path.basename(first), "Screenshot 2026-10-11 14.03.22.png")
        open(first, "wb").close()
        self.assertEqual(os.path.basename(shot.new_name(self.out, when)), "Screenshot 2026-10-11 14.03.22 (2).png")

    def test_last_is_remembered(self):
        shot.started()
        self.assertEqual(self.last(), "–")
        shot.remember("/x/Screenshot 1.png")
        self.assertEqual(self.last(), "Screenshot 1.png")
        self.assertEqual(shot.remembered(), "Screenshot 1.png")

    def test_unknown_mode(self):
        with self.assertRaisesRegex(ValueError, "unknown"):
            shot.capture({"what": "everything"})

    def test_winapi_guid(self):
        self.assertEqual(winapi.FOLDERID_PICTURES.Data1, 0x33E28130)
        self.assertEqual(bytes(winapi.FOLDERID_PICTURES.Data4), bytes.fromhex("835A98395C3BC3BB"))


class WindowsTest(Base):
    def test_script_quotes_the_path(self):
        script = shot.windows_script("window", "C:\\Users\\o'neil\\Pictures\\a.png", True)
        self.assertIn("$path = 'C:\\Users\\o''neil\\Pictures\\a.png'", script)
        self.assertIn("$copy = $true", script)
        self.assertIn("$mode = 'window'", script)
        with self.assertRaises(ValueError):
            shot.windows_script("region", "x", False)

    def test_parse_ps(self):
        self.assertEqual(shot.parse_ps('noise\n{"ok":true,"width":10}\r\n'), {"ok": True, "width": 10})
        with self.assertRaisesRegex(RuntimeError, "Access denied"):
            shot.parse_ps("", "Line 1\nAccess denied")

    def test_capture_runs_powershell(self):
        with mock.patch.object(shot.sys, "platform", "win32"), mock.patch.object(
            shot.subprocess, "run", return_value=done('{"ok":true,"width":1920,"height":1080}\n')
        ) as run:
            self.assertIsNone(shot.capture({"what": "screen", "copy": True}))
        args = run.call_args.args[0]
        self.assertEqual(args[:4], ["powershell.exe", "-NoProfile", "-NonInteractive", "-Sta"])
        script = base64.b64decode(args[-1]).decode("utf-16-le")
        self.assertIn("$mode = 'screen'", script)
        self.assertIn("CopyFromScreen", script)
        self.assertTrue(self.last().startswith("Screenshot "))

    def test_capture_error(self):
        with mock.patch.object(shot.sys, "platform", "win32"), mock.patch.object(
            shot.subprocess, "run", return_value=done('{"ok":false,"error":"no window is in front"}')
        ):
            with self.assertRaisesRegex(RuntimeError, "no window is in front"):
                shot.capture({"what": "window"})
        self.assertIsNone(self.last())

    def test_script_asks_for_per_monitor_dpi_first(self):
        script = shot.WINDOWS_SCRIPT
        v2 = script.index("SetProcessDpiAwarenessContext([IntPtr](-4))")
        self.assertLess(v2, script.index("[void][PerritoShot]::SetProcessDPIAware()"))
        self.assertLess(v2, script.index("System.Windows.Forms, System.Drawing"))

    def test_timeout_does_not_put_the_command_line_on_the_key(self):
        slow = subprocess.TimeoutExpired(["powershell.exe", "-EncodedCommand", "A" * 4000], shot.TIMEOUT)
        with mock.patch.object(shot.sys, "platform", "win32"), mock.patch.object(shot.subprocess, "run", side_effect=slow):
            with self.assertRaises(RuntimeError) as caught:
                shot.capture({"what": "screen"})
        self.assertEqual(str(caught.exception), f"Windows didn't take the screenshot in {shot.TIMEOUT} s")

    def test_long_errors_are_shortened(self):
        with mock.patch.object(shot.sys, "platform", "win32"), mock.patch.object(
            shot.subprocess, "run", return_value=done('{"ok":false,"error":"' + "x" * 300 + '"}')
        ):
            with self.assertRaises(RuntimeError) as caught:
                shot.capture({"what": "screen"})
        self.assertLessEqual(len(str(caught.exception)), shot.MAX_ERROR)

    def test_region_uses_the_snipping_overlay(self):
        with mock.patch.object(shot.sys, "platform", "win32"), mock.patch.object(shot.subprocess, "Popen") as popen:
            warning = shot.capture({"what": "region"})
        self.assertEqual(popen.call_args.args[0], ["explorer.exe", "ms-screenclip:"])
        self.assertIn("clipboard", warning)


class FakeProc:
    """A Popen that 'saves' a picture when the user would have picked one."""

    def __init__(self, args, picked=True, **kwargs):
        self.args = args
        self.picked = picked
        self.returncode = 0

    def communicate(self, timeout=None):
        if self.picked:
            with open(self.args[-1], "wb") as f:
                f.write(b"\x89PNG")
        else:
            self.returncode = 1
        return "", "" if self.picked else "selection cancelled"

    def wait(self, timeout=None):
        return 0


def wait_for_pick(test):
    """Wait for the background pick to finish (it releases _picking)."""
    test.assertTrue(shot._picking.acquire(timeout=2))
    shot._picking.release()


class MacTest(Base):
    def setUp(self):
        super().setUp()
        patcher = mock.patch.object(shot.sys, "platform", "darwin")
        patcher.start()
        self.addCleanup(patcher.stop)
        self.calls = []

        def fake_run(args, **kwargs):
            self.calls.append(list(args))
            if args[0] == "screencapture":
                with open(args[-1], "wb") as f:
                    f.write(b"\x89PNG")
            if args[0] == "osascript" and args[2] == shot.MAC_BOUNDS_SCRIPT:
                return self.bounds
            return done("")

        self.bounds = done("10,38,1280,800\n")
        patcher = mock.patch.object(shot.subprocess, "run", side_effect=fake_run)
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_screen_and_copy(self):
        self.assertIsNone(shot.capture({"what": "screen", "copy": True}))
        self.assertEqual(self.calls[0][:2], ["screencapture", "-x"])
        self.assertEqual(self.calls[1][:3], ["osascript", "-e", shot.MAC_COPY_SCRIPT])
        self.assertEqual(self.calls[1][3], self.calls[0][-1])

    def test_window_uses_its_bounds(self):
        shot.capture({"what": "window"})
        self.assertEqual(self.calls[1][:4], ["screencapture", "-x", "-R", "10,38,1280,800"])

    def test_window_falls_back_to_clicking_one(self):
        self.bounds = done("", 1, "System Events got an error: osascript is not allowed assistive access. (-1719)")
        with mock.patch.object(shot.subprocess, "Popen", side_effect=FakeProc) as popen:
            warning = shot.capture({"what": "window"})
            wait_for_pick(self)
        self.assertIn("Click the window", warning)
        self.assertEqual(popen.call_args.args[0][:4], ["screencapture", "-x", "-i", "-w"])
        self.assertTrue(self.last().startswith("Screenshot "))

    def test_parse_bounds(self):
        self.assertEqual(shot.parse_bounds("0, 25, 1440, 875"), "0,25,1440,875")
        self.assertIsNone(shot.parse_bounds("0,25,0,875"))
        self.assertIsNone(shot.parse_bounds("missing value"))


class LinuxTest(Base):
    def setUp(self):
        super().setUp()
        patcher = mock.patch.dict(os.environ, {"DISPLAY": ":0"})
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_needs_a_display(self):
        with mock.patch.object(shot.sys, "platform", "linux"), mock.patch.dict(os.environ, {"DISPLAY": "", "WAYLAND_DISPLAY": ""}):
            with self.assertRaisesRegex(RuntimeError, "no desktop session"):
                shot.capture({})

    def which(self, *present):
        return lambda name: f"/usr/bin/{name}" if name in present else None

    def test_tool_order(self):
        kde_wayland = {"XDG_CURRENT_DESKTOP": "KDE", "WAYLAND_DISPLAY": "wayland-0"}
        self.assertEqual(shot.linux_tool(kde_wayland, self.which("grim", "spectacle")), "spectacle")
        gnome_x11 = {"XDG_CURRENT_DESKTOP": "ubuntu:GNOME", "DISPLAY": ":0"}
        self.assertEqual(shot.linux_tool(gnome_x11, self.which("scrot", "gnome-screenshot")), "gnome-screenshot")
        sway = {"XDG_CURRENT_DESKTOP": "sway", "WAYLAND_DISPLAY": "wayland-1"}
        self.assertEqual(shot.linux_tool(sway, self.which("grim", "scrot")), "grim")
        self.assertIsNone(shot.linux_tool(sway, self.which("scrot", "import")))  # X11-only tools
        self.assertEqual(shot.linux_tool({"DISPLAY": ":0"}, self.which("import", "scrot")), "scrot")

    def test_commands(self):
        p = "/tmp/a.png"
        self.assertEqual(shot.linux_command("spectacle", "region", p), (["spectacle", "-b", "-n", "-r", "-o", p], True))
        self.assertEqual(shot.linux_command("gnome-screenshot", "window", p), (["gnome-screenshot", "-w", "-f", p], False))
        self.assertEqual(shot.linux_command("scrot", "screen", p), (["scrot", p], False))
        self.assertEqual(shot.linux_command("import", "screen", p), (["import", "-window", "root", p], False))
        self.assertEqual(shot.linux_command("grim", "screen", p), (["grim", p], False))
        with mock.patch.object(shot.shutil, "which", return_value="/usr/bin/slurp"):
            args, waits = shot.linux_command("grim", "region", p)
        self.assertTrue(waits)
        self.assertEqual(args[-1], p)
        with mock.patch.object(shot.shutil, "which", return_value=None):
            with self.assertRaisesRegex(RuntimeError, "slurp"):
                shot.linux_command("grim", "region", p)

    def test_grim_window_on_sway(self):
        tree = '{"nodes":[{"nodes":[{"focused":true,"rect":{"x":10,"y":20,"width":800,"height":600},"nodes":[]}]}]}'
        with mock.patch.dict(os.environ, {"SWAYSOCK": "/run/sway"}), mock.patch.object(
            shot.shutil, "which", return_value="/usr/bin/swaymsg"
        ), mock.patch.object(shot.subprocess, "run", return_value=done(tree)):
            args, _ = shot.linux_command("grim", "window", "/tmp/a.png")
        self.assertEqual(args, ["grim", "-g", "10,20 800x600", "/tmp/a.png"])

    def test_capture_and_copy(self):
        def fake_run(args, **kwargs):
            with open(args[-1], "wb") as f:
                f.write(b"\x89PNG")
            return done("")

        with mock.patch.object(shot.sys, "platform", "linux"), mock.patch.object(
            shot, "linux_tool", return_value="scrot"
        ), mock.patch.object(shot.subprocess, "run", side_effect=fake_run), mock.patch.object(
            shot.shutil, "which", side_effect=self.which("xclip")
        ), mock.patch.dict(os.environ, {"WAYLAND_DISPLAY": ""}), mock.patch.object(shot.subprocess, "Popen") as popen:
            popen.return_value.wait.return_value = 0
            self.assertIsNone(shot.capture({"what": "window", "copy": True}))
        self.assertEqual(popen.call_args.args[0], ["xclip", "-selection", "clipboard", "-t", "image/png", "-i"])
        self.assertEqual(popen.call_args.kwargs["stdout"], subprocess.DEVNULL)  # xclip lingers; never wait on its pipe
        self.assertTrue(self.last().startswith("Screenshot "))

    def test_copy_without_a_clipboard_tool_warns(self):
        def fake_run(args, **kwargs):
            open(args[-1], "wb").close()
            return done("")

        with mock.patch.object(shot.sys, "platform", "linux"), mock.patch.object(
            shot, "linux_tool", return_value="scrot"
        ), mock.patch.object(shot.subprocess, "run", side_effect=fake_run), mock.patch.object(shot.shutil, "which", return_value=None):
            self.assertIn("xclip", shot.capture({"what": "screen", "copy": True}))

    def test_no_tool(self):
        with mock.patch.object(shot.sys, "platform", "linux"), mock.patch.object(shot, "linux_tool", return_value=None):
            with self.assertRaisesRegex(RuntimeError, "install a screenshot tool"):
                shot.capture({})

    def test_region_runs_in_the_background(self):
        with mock.patch.object(shot.sys, "platform", "linux"), mock.patch.object(
            shot, "linux_tool", return_value="scrot"
        ), mock.patch.object(shot.subprocess, "Popen", side_effect=FakeProc):
            self.assertIsNone(shot.capture({"what": "region"}))
            wait_for_pick(self)
        self.assertTrue(self.last().startswith("Screenshot "))

    def test_cancelled_region(self):
        with mock.patch.object(shot.sys, "platform", "linux"), mock.patch.object(
            shot, "linux_tool", return_value="scrot"
        ), mock.patch.object(shot.subprocess, "Popen", side_effect=lambda a, **k: FakeProc(a, picked=False)):
            shot.capture({"what": "region"})
            wait_for_pick(self)
        self.assertIsNone(self.last())
        self.assertIn("cancelled", self.log.call_args.args[0])


if __name__ == "__main__":
    unittest.main()
