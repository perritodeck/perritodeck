import ctypes
import os
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as win  # noqa: E402
import winapi  # noqa: E402  (must import on every platform)

# Real `wmctrl -lpx` output (columns: id, desktop, pid, WM_CLASS, host, title).
WMCTRL = """\
0x01e00003 -1 1823   xfdesktop.Xfdesktop   laptop Desktop
0x02a00004 -1 1810   xfce4-panel.Xfce4-panel  laptop xfce4-panel
0x03a00003  0 4321   Navigator.firefox     laptop Mozilla Firefox
0x04400006  0 5550   gnome-terminal-server.Gnome-terminal  laptop user@laptop: ~/src
0x03a0012b  1 4321   Navigator.firefox     laptop GitHub — Mozilla Firefox
0x05000001  0 0      N/A                   N/A
"""
COMMS = {4321: "firefox", 5550: "gnome-terminal-", 1823: "xfdesktop", 1810: "xfce4-panel"}

SWAY_TREE = {
    "id": 1, "type": "root", "nodes": [
        {"id": 3, "type": "output", "nodes": [
            {"id": 4, "type": "workspace", "nodes": [
                {"id": 7, "type": "con", "pid": 900, "app_id": "foot", "name": "~", "focused": False, "nodes": []},
                {"id": 9, "type": "con", "pid": 901, "app_id": None, "name": "Steam",
                 "window_properties": {"class": "steam"}, "focused": True, "nodes": []},
            ], "floating_nodes": [
                {"id": 12, "type": "floating_con", "pid": 902, "app_id": "pavucontrol", "name": "Volume Control", "nodes": []},
            ]},
        ]},
    ],
}

HYPR_CLIENTS = [
    {"address": "0x55aa10", "mapped": True, "hidden": False, "class": "kitty", "title": "zsh", "pid": 1200, "focusHistoryID": 1},
    {"address": "0x55aa20", "mapped": True, "hidden": False, "class": "firefox", "title": "News — Mozilla Firefox", "pid": 1300, "focusHistoryID": 0},
    {"address": "0x55aa30", "mapped": False, "class": "", "title": "", "pid": -1, "focusHistoryID": 5},
]

US = "\x1f"


def done(stdout="", returncode=0, stderr=""):
    return subprocess.CompletedProcess([], returncode, stdout=stdout, stderr=stderr)


class FakeRun:
    """Answers subprocess.run by program, and records every call."""

    def __init__(self, answers):
        self.answers = answers
        self.calls = []

    def __call__(self, args, **kwargs):
        self.calls.append(list(args))
        key = os.path.basename(args[0])
        answer = self.answers.get(key, "")
        if callable(answer):
            answer = answer(args)
        return answer if isinstance(answer, subprocess.CompletedProcess) else done(answer)


def comm_of(pid, root="/proc"):
    return COMMS.get(int(pid), "")


class MatchingTest(unittest.TestCase):
    def test_norm(self):
        self.assertEqual(win.norm("Firefox.EXE"), "firefox")
        self.assertEqual(win.norm(" Visual Studio Code.app "), "visual studio code")

    def test_rank_prefers_process_then_class_then_title(self):
        exe = win.Window(1, 1, "code.exe", "main.py - Visual Studio Code")
        cls = win.Window(2, 2, "electron", "x", "code.Code")
        title = win.Window(3, 3, "chrome.exe", "Code review - Chrome")
        self.assertEqual(win.rank(exe, "code"), 0)
        self.assertEqual(win.rank(cls, "Code"), 1)
        self.assertEqual(win.rank(title, "code"), 2)
        self.assertIsNone(win.rank(title, "spotify"))
        self.assertIsNone(win.rank(title, "  "))
        self.assertEqual(win.best_matches([title, cls, exe], "code"), [exe, cls, title])


class ParseTest(unittest.TestCase):
    def test_wmctrl(self):
        windows = win.parse_wmctrl(WMCTRL, comm_of=comm_of)
        self.assertEqual(len(windows), 4)  # sticky desktop/panel windows are left out
        ff = windows[0]
        self.assertEqual(ff, win.Window(0x03A00003, 4321, "firefox", "Mozilla Firefox", "Navigator.firefox"))
        self.assertEqual(windows[1].title, "user@laptop: ~/src")
        self.assertEqual(windows[3], win.Window(0x05000001, 0, "", "", "N/A"))

    def test_xprop_and_showing_desktop(self):
        self.assertEqual(win.parse_active_xprop("_NET_ACTIVE_WINDOW(WINDOW): window id # 0x3a00003\n"), 0x3A00003)
        self.assertIsNone(win.parse_active_xprop("_NET_ACTIVE_WINDOW(WINDOW): window id # 0x0"))
        self.assertIsNone(win.parse_active_xprop(""))
        wm = 'Name: Xfwm4\nClass: xfwm4\nPID: 1802\nWindow manager\'s "showing the desktop" mode: ON\n'
        self.assertTrue(win.parse_showing_desktop(wm))
        self.assertFalse(win.parse_showing_desktop(wm.replace(": ON", ": OFF")))
        self.assertIsNone(win.parse_showing_desktop("Name: dwm\n"))

    def test_sway(self):
        with mock.patch.object(win, "comm", side_effect=lambda pid: {900: "foot"}.get(pid, "")):
            windows, focused = win.sway_windows(SWAY_TREE)
        self.assertEqual([w.id for w in windows], [7, 9, 12])
        self.assertEqual(windows[1].cls, "steam")
        self.assertEqual(windows[1].app, "steam")  # no comm: falls back to the class
        self.assertEqual(focused.id, 9)

    def test_hyprland(self):
        with mock.patch.object(win, "comm", return_value=""):
            windows = win.hypr_windows(HYPR_CLIENTS)
        self.assertEqual([w.id for w in windows], ["0x55aa20", "0x55aa10"])  # most recent first, unmapped dropped

    def test_mac_list_and_poll(self):
        out = "!access\nSafari" + US + "501" + US + "1" + US + "Start Page\nSafari" + US + "501" + US + "2" + US + "Docs\nFinder" + US + "300" + US + "0" + US + "\n"
        windows, denied = win.parse_mac_list(out)
        self.assertTrue(denied)
        self.assertEqual(windows[1], win.Window(("Safari", 2), 501, "Safari", "Docs"))
        self.assertEqual(windows[2].id, ("Finder", 0))
        front, names = win.parse_mac_poll("Safari" + US + "loginwindow" + US + "Safari" + US + "Code\n")
        self.assertEqual(front, "Safari")
        self.assertEqual(names, {"loginwindow", "safari", "code"})

    def test_mac_errors_explain_permissions(self):
        self.assertIn("Accessibility", win.mac_error("execution error: System Events got an error: osascript is not allowed assistive access. (-1719)"))
        self.assertIn("Automation", win.mac_error("Not authorized to send Apple events to System Events. (-1743)"))
        self.assertEqual(win.mac_error("something else"), "something else")

    def test_proc_names(self):
        with tempfile.TemporaryDirectory() as root:
            for pid, comm, cmdline in (
                ("1", "systemd", b"/sbin/init\0splash\0"),
                ("42", "gnome-terminal-", b"/usr/libexec/gnome-terminal-server\0"),
                ("77", "Discord", b""),
            ):
                os.makedirs(os.path.join(root, pid))
                with open(os.path.join(root, pid, "comm"), "w") as f:
                    f.write(comm + "\n")
                with open(os.path.join(root, pid, "cmdline"), "wb") as f:
                    f.write(cmdline)
            os.makedirs(os.path.join(root, "self"))
            names = win.proc_names(root)
            self.assertEqual(win.comm(42, root), "gnome-terminal-")
        self.assertEqual(names, {"systemd", "init", "gnome-terminal-", "gnome-terminal-server", "discord"})


class PickBackendTest(unittest.TestCase):
    def which(self, *present):
        return lambda name: f"/usr/bin/{name}" if name in present else None

    def test_linux_choices(self):
        x11 = {"DISPLAY": ":0"}
        self.assertIsInstance(win.pick_backend("linux", x11, self.which("wmctrl", "xdotool")), win.WmctrlBackend)
        self.assertIsInstance(win.pick_backend("linux", x11, self.which("xdotool")), win.XdotoolBackend)
        sway = {"WAYLAND_DISPLAY": "wayland-1", "SWAYSOCK": "/run/sway.sock", "DISPLAY": ":0"}
        self.assertIsInstance(win.pick_backend("linux", sway, self.which("swaymsg", "wmctrl")), win.SwayBackend)
        hypr = {"WAYLAND_DISPLAY": "wayland-1", "HYPRLAND_INSTANCE_SIGNATURE": "abc"}
        self.assertIsInstance(win.pick_backend("linux", hypr, self.which("hyprctl")), win.HyprlandBackend)
        gnome = {"WAYLAND_DISPLAY": "wayland-0", "DISPLAY": ":0"}
        b = win.pick_backend("linux", gnome, self.which("wmctrl"))
        self.assertIsInstance(b, win.WmctrlBackend)
        self.assertTrue(b.wayland)
        b = win.pick_backend("linux", {"XDG_SESSION_TYPE": "wayland"}, self.which())
        self.assertIsInstance(b, win.NoBackend)
        self.assertIn("Wayland", b.why())
        self.assertIn("wmctrl", win.pick_backend("linux", x11, self.which()).why())

    def test_mac(self):
        self.assertIsInstance(win.pick_backend("darwin", {}), win.MacBackend)


class WinapiTest(unittest.TestCase):
    def test_imports_and_struct_layout(self):
        # Process32FirstW checks dwSize: 568 bytes on 64-bit Windows, 556 on 32-bit.
        # wchar_t is 4 bytes on Linux, so only the offsets before the name compare here.
        wide = ctypes.sizeof(ctypes.c_size_t) == 8
        self.assertEqual(winapi.PROCESSENTRY32W.szExeFile.offset, 44 if wide else 36)
        if sys.platform == "win32":
            self.assertEqual(ctypes.sizeof(winapi.PROCESSENTRY32W), 568 if wide else 556)
        self.assertEqual(winapi.SW_MINIMIZE, 6)
        self.assertEqual(winapi.WM_CLOSE, 0x10)

    def test_windows_backend_wraps_winapi(self):
        api = mock.Mock(SW_MINIMIZE=6, SW_MAXIMIZE=3, SW_RESTORE=9)
        api.list_windows.return_value = [(100, 7, "Spotify.exe", "Spotify Premium"), (200, 8, "notepad.exe", "notes.txt - Notepad")]
        api.foreground.return_value = 200
        api.info.return_value = (200, 8, "notepad.exe", "notes.txt - Notepad")
        api.process_names.return_value = {"spotify.exe", "system"}
        api.focus.return_value = False
        b = win.WindowsBackend.__new__(win.WindowsBackend)
        b.api = api
        self.assertEqual(b.find("spotify")[0].id, 100)
        self.assertEqual(win.friendly(b.active()), "notepad")
        self.assertTrue(b.running("Spotify.exe"))
        self.assertFalse(b.running("discord"))
        self.assertIn("front", b.activate(b.find("spotify")[0]))  # refused: a warning
        b.minimize(b.find("notepad")[0])
        api.show.assert_called_with(200, 6)
        b.quit("spotify", b.find("spotify"))
        api.close.assert_called_once_with(100)

    def test_store_apps_are_named_after_the_app_not_the_frame_host(self):
        names = {900: "CalculatorApp.exe"}.get
        hosted = mock.Mock(return_value=900)
        self.assertEqual(winapi.app_of(1, 40, "ApplicationFrameHost.exe", hosted, names), (900, "CalculatorApp.exe"))
        hosted.assert_called_once_with(1, 40)
        # A suspended app has no window of its own inside the frame: keep the host.
        self.assertEqual(winapi.app_of(1, 40, "ApplicationFrameHost.exe", lambda h, p: None, names), (40, "ApplicationFrameHost.exe"))
        self.assertEqual(winapi.app_of(2, 7, "notepad.exe", hosted, names), (7, "notepad.exe"))
        hosted.assert_called_once()

    @unittest.skipUnless(sys.platform == "win32", "Windows only")
    def test_lists_this_desktops_windows(self):
        for hwnd, pid, exe, title in winapi.list_windows():  # read-only
            self.assertTrue(hwnd and title)
            self.assertIsInstance(pid, int)
            self.assertIsInstance(exe, str)


class LinuxActionsTest(unittest.TestCase):
    def setUp(self):
        self.backend = win.WmctrlBackend()
        self.backend.xprop = "/usr/bin/xprop"
        self.backend.xdotool = None
        # launch() and the SDK's start_for_user pick their way of starting a program
        # from sys.platform.
        patcher = mock.patch.object(win.sys, "platform", "linux")
        patcher.start()
        self.addCleanup(patcher.stop)
        patcher = mock.patch.object(win, "backend", return_value=self.backend)
        patcher.start()
        self.addCleanup(patcher.stop)
        patcher = mock.patch.object(win, "comm", side_effect=comm_of)
        patcher.start()
        self.addCleanup(patcher.stop)
        self.active = "0x4400006"
        self.fake = FakeRun({
            "wmctrl": lambda args: WMCTRL if args[1] == "-lpx" else ('"showing the desktop" mode: OFF' if args[1] == "-m" else ""),
            "xprop": lambda args: f"_NET_ACTIVE_WINDOW(WINDOW): window id # {self.active}",
        })
        patcher = mock.patch.object(win.subprocess, "run", side_effect=self.fake)
        patcher.start()
        self.addCleanup(patcher.stop)

    def commands(self, program="wmctrl"):
        return [c[1:] for c in self.fake.calls if os.path.basename(c[0]) == program and c[1] not in ("-lpx", "-m")]

    def test_focus_activates_the_best_match(self):
        self.assertIsNone(win.focus({"app": "firefox"}))
        self.assertEqual(self.commands(), [["-i", "-a", "0x3a00003"]])

    def test_focus_cycles_when_already_in_front(self):
        self.active = "0x3a00003"
        win.focus({"app": "firefox"})
        self.assertEqual(self.commands()[-1], ["-i", "-a", "0x3a0012b"])

    def test_focus_by_title(self):
        win.focus({"app": "GitHub"})
        self.assertEqual(self.commands(), [["-i", "-a", "0x3a0012b"]])

    def test_focus_launches_when_nothing_matches(self):
        with mock.patch.object(win.subprocess, "Popen") as popen:
            self.assertIsNone(win.focus({"app": "spotify", "launch": "flatpak run com.spotify.Client"}))
        self.assertEqual(popen.call_args.args[0], ["flatpak", "run", "com.spotify.Client"])
        self.assertEqual(popen.call_args.kwargs["stdout"], subprocess.DEVNULL)  # never our protocol pipe
        self.assertTrue(popen.call_args.kwargs["start_new_session"])

    def test_focus_without_match_or_command(self):
        with mock.patch.object(win.shutil, "which", return_value=None):
            with self.assertRaisesRegex(RuntimeError, "no window matches “spotify”"):
                win.focus({"app": "spotify"})
        with self.assertRaisesRegex(ValueError, "set the app"):
            win.focus({"app": " "})

    def test_window_ops_on_the_front_window(self):
        win.window({"do": "maximize"})
        win.window({"do": "close"})
        win.window({"do": "minimize", "app": "firefox"})
        self.assertEqual(
            self.commands(),
            [
                ["-i", "-r", "0x4400006", "-b", "add,maximized_vert,maximized_horz"],
                ["-i", "-c", "0x4400006"],
                ["-i", "-r", "0x3a00003", "-b", "add,hidden"],
            ],
        )
        with self.assertRaisesRegex(ValueError, "unknown"):
            win.window({"do": "explode"})
        with self.assertRaisesRegex(RuntimeError, "no window matches"):
            win.window({"do": "close", "app": "zoom"})

    def test_minimize_prefers_xdotool(self):
        self.backend.xdotool = "/usr/bin/xdotool"
        win.window({"do": "minimize"})
        self.assertEqual(self.commands("xdotool"), [["windowminimize", str(0x4400006)]])

    def test_quit_closes_every_window_of_the_app(self):
        self.assertIsNone(win.quit_app({"app": "firefox"}))
        self.assertEqual(self.commands(), [["-i", "-c", "0x3a00003"], ["-i", "-c", "0x3a0012b"]])
        self.assertIn("isn't running", win.quit_app({"app": "zoom"}))

    def test_show_desktop_toggles(self):
        win.desktop({})
        self.assertEqual(self.commands(), [["-k", "on"]])

    def test_helper_errors_are_reported(self):
        self.fake.answers["wmctrl"] = lambda args: WMCTRL if args[1] == "-lpx" else done("", 1, "Cannot open display.")
        with self.assertRaisesRegex(RuntimeError, "Cannot open display"):
            win.focus({"app": "firefox"})


class ValuesTest(unittest.TestCase):
    def setUp(self):
        win._published.clear()
        self.backend = mock.Mock(spec=win.Backend)
        self.backend.active.return_value = win.Window(1, 2, "Code.exe", "x")
        self.backend.process_names.return_value = {"code", "spotify"}
        self.backend.windows.return_value = [win.Window(3, 4, "chrome", "YouTube Music - Chrome")]
        for target, value in ((win, "backend"),):
            patcher = mock.patch.object(target, value, return_value=self.backend)
            patcher.start()
            self.addCleanup(patcher.stop)
        patcher = mock.patch.object(win.plugin, "set_value")
        self.set_value = patcher.start()
        self.addCleanup(patcher.stop)
        win.plugin.watching = set()
        self.addCleanup(setattr, win.plugin, "watching", set())

    def published(self):
        return {(c.args[0] if c.kwargs.get("arg") is None else f"{c.args[0]}:{c.kwargs['arg']}"): c.args[1] for c in self.set_value.call_args_list}

    def test_focused_app_always_published_running_only_when_watched(self):
        win.poll()
        self.assertEqual(self.published(), {"focused_app": "Code"})
        self.backend.process_names.assert_not_called()
        win.plugin.watching = {"running:spotify", "running:discord", "running:YouTube Music"}
        win.poll()
        self.assertEqual(
            self.published(),
            {"focused_app": "Code", "running:spotify": "on", "running:discord": "off", "running:YouTube Music": "on"},
        )

    def test_only_changes_are_sent(self):
        win.plugin.watching = {"running:spotify"}
        win.poll()
        win.poll()
        self.assertEqual(self.set_value.call_count, 2)
        win.watching(["running:spotify"])
        win.poll()
        self.assertEqual(self.set_value.call_count, 2)

    def test_failures_show_a_dash(self):
        self.backend.active.side_effect = RuntimeError("boom")
        self.backend.process_names.side_effect = RuntimeError("boom")
        win.plugin.watching = {"running:spotify"}
        with mock.patch.object(win.plugin, "log"):
            win.poll()
        self.assertEqual(self.published(), {"focused_app": "–", "running:spotify": "–"})

    def test_nothing_in_front(self):
        self.backend.active.return_value = None
        win.poll()
        self.assertEqual(self.published(), {"focused_app": "–"})


class MacTest(unittest.TestCase):
    def setUp(self):
        self.backend = win.MacBackend()
        patcher = mock.patch.object(win, "backend", return_value=self.backend)
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_window_op_on_the_front_app(self):
        fake = FakeRun({"osascript": "ok\n"})
        with mock.patch.object(win.subprocess, "run", side_effect=fake):
            self.assertIsNone(win.window({"do": "minimize"}))
        self.assertEqual(fake.calls[0][0], "osascript")
        self.assertEqual(fake.calls[0][-3:], ["", "0", "minimize"])

    def test_focus_named_window_and_quit(self):
        listing = "Safari" + US + "501" + US + "1" + US + "Start Page\nMail" + US + "502" + US + "1" + US + "Inbox\n"

        def osa(args):
            script = args[2]
            if script == win.MAC_LIST_SCRIPT:
                return listing
            if script == win.MAC_WINDOW_SCRIPT and args[-1] == "quit":
                return "com.apple.mail\n"
            if script == win.MAC_POLL_SCRIPT:
                return "Finder" + US + "Finder\n"
            return "ok\n"

        fake = FakeRun({"osascript": osa})
        with mock.patch.object(win.subprocess, "run", side_effect=fake):
            win.focus({"app": "inbox"})
            win.quit_app({"app": "mail"})
        self.assertEqual(fake.calls[1][-3:], ["Mail", "1", "focus"])
        self.assertEqual(fake.calls[-1][-1], "com.apple.mail")

    def test_permission_error(self):
        fake = FakeRun({"osascript": done("", 1, "execution error: Not authorised (-1719)")})
        with mock.patch.object(win.subprocess, "run", side_effect=fake):
            with self.assertRaisesRegex(RuntimeError, "Accessibility"):
                win.window({"do": "close"})

    def test_launch_uses_open(self):
        fake = FakeRun({"open": ""})
        with mock.patch.object(win.subprocess, "run", side_effect=fake):
            win.launch("Visual Studio Code.app", platform="darwin")
            win.launch("Spotify", platform="darwin")
        self.assertEqual(fake.calls, [["open", "-a", "Visual Studio Code.app"], ["open", "-a", "Spotify"]])


class LaunchWindowsTest(unittest.TestCase):
    def setUp(self):
        # Programs for the user go through the SDK, so they outlive the plugin's job
        # object and start in their own folder (tested in sdk-python).
        patcher = mock.patch.object(win, "open_for_user")
        self.open_for_user = patcher.start()
        self.addCleanup(patcher.stop)
        patcher = mock.patch.object(win, "start_for_user")
        self.start_for_user = patcher.start()
        self.addCleanup(patcher.stop)

    def test_names_and_uris_open_like_a_double_click_command_lines_start(self):
        win.launch("spotify:", platform="win32")
        win.launch('"C:\\Tools\\app.exe" --profile work', platform="win32")
        self.open_for_user.assert_called_once_with("spotify:")
        self.start_for_user.assert_called_once_with('"C:\\Tools\\app.exe" --profile work')

    def test_a_name_the_shell_doesnt_know_runs_as_a_command(self):
        self.open_for_user.side_effect = OSError("[WinError 2] not found")
        win.launch("obs64", platform="win32")
        self.start_for_user.assert_called_once_with("obs64")

    def test_a_file_that_wont_open_is_an_error(self):
        self.open_for_user.side_effect = OSError("[WinError 1155] no app is associated")
        with tempfile.TemporaryDirectory() as folder:
            path = os.path.join(folder, "my file.unknownext")
            open(path, "w").close()
            with self.assertRaisesRegex(OSError, "1155"):
                win.launch(path, platform="win32")
            self.open_for_user.assert_called_once_with(path)
        self.start_for_user.assert_not_called()

    def test_focus_starts_a_program_found_on_path(self):
        b = win.WindowsBackend.__new__(win.WindowsBackend)
        b.api = mock.Mock()
        b.api.list_windows.return_value = []
        found = "C:\\Windows\\System32\\charmap.exe"
        with mock.patch.object(win, "backend", return_value=b), \
                mock.patch.object(win.sys, "platform", "win32"), \
                mock.patch.object(win.shutil, "which", return_value=found):
            self.assertIsNone(win.focus({"app": "charmap"}))
        self.start_for_user.assert_called_once_with([found])


class QuitWindowsTest(unittest.TestCase):
    """One process can hold many apps' or folders' windows."""

    WINDOWS = [
        (1, 50, "CalculatorApp.exe", "Calculator"),
        # Two suspended Store apps: Windows doesn't say which app is in the frame.
        (2, 40, "ApplicationFrameHost.exe", "Settings"),
        (3, 40, "ApplicationFrameHost.exe", "Microsoft Store"),
        (4, 60, "explorer.exe", "Desktop - File Explorer"),
        (5, 60, "explorer.exe", "perritodeck - File Explorer"),
        (6, 70, "vmconnect.exe", "dev on DESKTOP-1 - Virtual Machine Connection"),
    ]

    def closed(self, query):
        b = win.WindowsBackend.__new__(win.WindowsBackend)
        b.api = mock.Mock()
        b.api.list_windows.return_value = self.WINDOWS
        b.quit(query, b.find(query))
        return [c.args[0] for c in b.api.close.call_args_list]

    def test_a_title_match_closes_only_the_windows_it_names(self):
        self.assertEqual(self.closed("Settings"), [2])
        self.assertEqual(self.closed("Desktop"), [4, 6])
        self.assertEqual(self.closed("Calculator"), [1])

    def test_a_process_match_closes_the_whole_app(self):
        self.assertEqual(self.closed("explorer"), [4, 5])
        self.assertEqual(self.closed("CalculatorApp"), [1])


if __name__ == "__main__":
    unittest.main()
