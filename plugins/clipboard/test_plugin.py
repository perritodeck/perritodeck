import base64
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as clip  # noqa: E402
import winapi  # noqa: E402  (must import on every platform)


def done(stdout=b"", returncode=0, stderr=b""):
    return subprocess.CompletedProcess([], returncode, stdout=stdout, stderr=stderr)


# What Windows PowerShell 5.1 wrote on stderr for an uncaught error under
# -EncodedCommand, after the progress record a module load writes; shortened.
CLIXML_ERROR = (
    "#< CLIXML\r\n"
    '<Objs Version="1.1.0.1" xmlns="http://schemas.microsoft.com/powershell/2004/04">'
    '<Obj S="progress" RefId="0"><TN RefId="0"><T>System.Management.Automation.PSCustomObject</T>'
    '<T>System.Object</T></TN><MS><I64 N="SourceId">1</I64><PR N="Record"><AV>Preparing modules for first use.</AV>'
    "<AI>0</AI><Nil /><PI>-1</PI><PC>-1</PC><T>Completed</T><SR>-1</SR><SD> </SD></PR></MS></Obj>"
    '<S S="Error">Requested Clipboard operation did not succeed. &lt;Für_x005F_x0041_&gt;_x000D__x000A_</S>'
    '<S S="Error">At line:3 char:1_x000D__x000A_</S>'
    '<S S="Error">+ Get-Clipboard -Raw_x000D__x000A_</S>'
    '<S S="Error"> _x000D__x000A_</S></Objs>'
).encode("utf-8")


class FakeClipboard:
    def __init__(self, text=None):
        self.text = text
        self.pasted = []

    def get(self):
        return self.text

    def set(self, text):
        self.text = text

    def paste(self, combo):
        self.pasted.append((combo, self.text))


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        patcher = mock.patch.object(clip.plugin, "_data_dir", self.tmp.name)
        patcher.start()
        self.addCleanup(patcher.stop)
        patcher = mock.patch.object(clip.plugin, "set_value")
        self.set_value = patcher.start()
        self.addCleanup(patcher.stop)
        patcher = mock.patch.object(clip.plugin, "log")
        self.log = patcher.start()
        self.addCleanup(patcher.stop)
        clip.plugin.settings = {"restore_after": 0.1}
        self.addCleanup(setattr, clip.plugin, "settings", {})
        clip.plugin.watching = set()
        self.addCleanup(setattr, clip.plugin, "watching", set())
        self.cb = FakeClipboard("old stuff")
        patcher = mock.patch.object(clip, "_clipboard", self.cb)
        patcher.start()
        self.addCleanup(patcher.stop)
        self.addCleanup(self.finish_restores)

    def finish_restores(self):
        timer = clip._restore["timer"]
        if timer is not None:
            timer.cancel()
        clip._restore.update(timer=None, old=None, ours=None)

    def wait_restore(self):
        timer = clip._restore["timer"]
        if timer is not None:
            timer.join(2)
            time.sleep(0.01)


class PasteTest(Base):
    def test_copy(self):
        clip.copy_text({"text": "héllo 👋"})
        self.assertEqual(self.cb.text, "héllo 👋")
        with self.assertRaisesRegex(ValueError, "text"):
            clip.copy_text({"text": ""})

    def test_paste_restores_the_old_clipboard(self):
        clip.paste_text({"text": "snippet"})
        self.assertEqual(self.cb.pasted, [("ctrl+v", "snippet")])
        self.assertEqual(self.cb.text, "snippet")
        self.wait_restore()
        self.assertEqual(self.cb.text, "old stuff")

    def test_no_restore_when_off(self):
        clip.paste_text({"text": "snippet", "restore": False, "keys": "ctrl+shift+v"})
        self.assertEqual(self.cb.pasted, [("ctrl+shift+v", "snippet")])
        self.assertIsNone(clip._restore["timer"])
        self.assertEqual(self.cb.text, "snippet")

    def test_no_restore_if_something_new_was_copied(self):
        clip.paste_text({"text": "snippet"})
        self.cb.text = "copied meanwhile"
        self.wait_restore()
        self.assertEqual(self.cb.text, "copied meanwhile")

    def test_quick_presses_keep_the_original(self):
        clip.plugin.settings = {"restore_after": 1}
        clip.paste_text({"text": "one"})
        clip.paste_text({"text": "two"})
        self.assertEqual(clip._restore["old"], "old stuff")
        self.assertEqual(self.cb.pasted, [("ctrl+v", "one"), ("ctrl+v", "two")])
        clip.put_back(clip._restore["timer"])  # as if the timer fired now
        self.assertEqual(self.cb.text, "old stuff")

    def test_stale_timer_does_nothing(self):
        clip.plugin.settings = {"restore_after": 1}
        clip.paste_text({"text": "one"})
        stale = clip._restore["timer"]
        clip.paste_text({"text": "two"})
        clip.put_back(stale)
        self.assertEqual(self.cb.text, "two")

    def test_empty_or_picture_clipboard_isnt_restored(self):
        self.cb.text = None
        clip.paste_text({"text": "x"})
        self.assertIsNone(clip._restore["timer"])

    def test_bad_shortcut(self):
        with self.assertRaisesRegex(ValueError, "shortcut"):
            clip.paste_text({"text": "x", "keys": "ctrl+alt+del"})

    def test_restore_delay_is_clamped(self):
        for value, expected in ((0, 0.1), ("2", 2.0), (60, 5.0), ("soon", 0.5)):
            clip.plugin.settings = {"restore_after": value}
            self.assertEqual(clip.restore_delay(), expected)


class SlotTest(Base):
    def test_save_and_paste_a_slot(self):
        clip.plugin.watching = {"slot:2", "slot:9", "slot:x"}
        self.cb.text = "line one\n  line two is quite a bit longer"
        self.assertIsNone(clip.save_slot({"slot": 2}))
        self.assertEqual(self.set_value.call_count, 1)
        self.assertEqual(self.set_value.call_args.args, ("slot", "line one line two is…"))
        self.assertEqual(self.set_value.call_args.kwargs, {"arg": "2"})
        with open(os.path.join(self.tmp.name, "slots.json"), encoding="utf-8") as f:
            self.assertEqual(json.load(f), {"2": "line one\n  line two is quite a bit longer"})
        self.cb.text = "other"
        clip.paste_slot({"slot": "2.0", "restore": False})
        self.assertEqual(self.cb.pasted[-1], ("ctrl+v", "line one\n  line two is quite a bit longer"))
        clip.copy_slot({"slot": 2})
        self.assertEqual(self.cb.text, "line one\n  line two is quite a bit longer")

    def test_watch_publishes_every_watched_slot(self):
        clip.store_slot("1", "short")
        clip.plugin.watching = {"slot:1", "slot:3", "slot:nope"}
        clip.watching(sorted(clip.plugin.watching))
        published = {c.kwargs["arg"]: c.args[1] for c in self.set_value.call_args_list}
        self.assertEqual(published, {"1": "short", "3": "–", "nope": "–"})

    def test_errors(self):
        with self.assertRaisesRegex(RuntimeError, "slot 4 is empty"):
            clip.paste_slot({"slot": 4})
        with self.assertRaisesRegex(ValueError, "slot number"):
            clip.save_slot({"slot": ""})
        with self.assertRaisesRegex(ValueError, "start at 1"):
            clip.save_slot({"slot": 0})
        self.cb.text = None
        self.assertIn("no text", clip.save_slot({"slot": 1}))

    def test_preview(self):
        self.assertEqual(clip.preview(None), "–")
        self.assertEqual(clip.preview(" \n "), "–")
        self.assertEqual(clip.preview("exactly twenty chars"), "exactly twenty chars")

    def test_corrupt_slots_file(self):
        with open(os.path.join(self.tmp.name, "slots.json"), "w") as f:
            f.write("{nope")
        self.assertEqual(clip.load_slots(), {})


class WindowsTest(Base):
    def test_get_decodes_base64(self):
        payload = base64.b64encode("ünïcode ✓".encode("utf-8"))
        with mock.patch.object(clip.subprocess, "run", return_value=done(payload)) as run:
            self.assertEqual(clip.WindowsClipboard().get(), "ünïcode ✓")
        args = run.call_args.args[0]
        self.assertEqual(args[0], "powershell.exe")
        self.assertIn("Get-Clipboard", base64.b64decode(args[-1]).decode("utf-16-le"))
        with mock.patch.object(clip.subprocess, "run", return_value=done(b"")):
            self.assertIsNone(clip.WindowsClipboard().get())

    def test_errors_are_one_short_line(self):
        # The scripts catch their own errors and print just the message, in UTF-8...
        for script in (clip.WINDOWS_GET, clip.WINDOWS_SET):
            self.assertIn("$ProgressPreference = 'SilentlyContinue'", script)
            self.assertIn("[Console]::OutputEncoding = [Text.Encoding]::UTF8", script)
            self.assertIn("[Console]::Error.WriteLine(", script)
        failed = done(b"", 1, "Der angeforderte Zwischenablagevorgang ist fehlgeschlagen.\r\n".encode("utf-8"))
        with mock.patch.object(clip.subprocess, "run", return_value=failed):
            with self.assertRaisesRegex(RuntimeError, "^Der angeforderte Zwischenablagevorgang ist fehlgeschlagen.$"):
                clip.WindowsClipboard().get()
        # ...and anything that still escapes as CLIXML is read for its message.
        with mock.patch.object(clip.subprocess, "run", return_value=done(b"", 1, CLIXML_ERROR)):
            with self.assertRaises(RuntimeError) as caught:
                clip.WindowsClipboard().get()
        self.assertEqual(str(caught.exception), "Requested Clipboard operation did not succeed. <Für_x0041_>")
        self.assertEqual(clip.error_text(CLIXML_ERROR.decode("utf-8").split("<S S=")[0] + "</Objs>"), "")
        long = done(b"", 1, ("x" * 300).encode())
        with mock.patch.object(clip.subprocess, "run", return_value=long):
            with self.assertRaises(RuntimeError) as caught:
                clip.WindowsClipboard().get()
        self.assertLessEqual(len(str(caught.exception)), clip.MAX_ERROR)

    def test_set_goes_through_a_utf8_file(self):
        # A profile folder can contain ’, which ends a '…' string in PowerShell just
        # like ', so the file's path goes in the environment, not the script.
        data = os.path.join(self.tmp.name, "Team’s 'data'")
        os.makedirs(data)
        seen = {}

        def fake(args, **kwargs):
            script = base64.b64decode(args[-1]).decode("utf-16-le")
            path = kwargs["env"]["PERRITODECK_CLIP_FILE"]
            with open(path, encoding="utf-8", newline="") as f:
                seen["text"] = f.read()
            seen["script"], seen["path"] = script, path
            return done()

        with mock.patch.object(clip.plugin, "_data_dir", data), mock.patch.object(clip.subprocess, "run", side_effect=fake):
            clip.WindowsClipboard().set("naïve 'quote'\r\nnext")
        self.assertEqual(seen["text"], "naïve 'quote'\r\nnext")
        self.assertEqual(os.path.dirname(seen["path"]), data)
        self.assertNotIn(data, seen["script"])
        self.assertIn("$env:PERRITODECK_CLIP_FILE", seen["script"])
        self.assertIn("Set-Clipboard", seen["script"])
        self.assertEqual([f for f in os.listdir(data) if f.startswith("clip-")], [])  # cleaned up

    def test_key_plan(self):
        self.assertEqual(
            winapi.plan("ctrl+shift+v"),
            [(0x11, False, False), (0x10, False, False), (0x56, False, False), (0x56, False, True), (0x10, False, True), (0x11, False, True)],
        )
        self.assertEqual(winapi.plan("shift+insert")[1], (0x2D, True, False))
        with self.assertRaises(ValueError):
            winapi.plan("ctrl+q")


class MacTest(Base):
    def test_pbcopy_pbpaste_utf8(self):
        with mock.patch.object(clip.subprocess, "run", return_value=done("café".encode("utf-8"))) as run:
            mac = clip.MacClipboard()
            self.assertEqual(mac.get(), "café")
            mac.set("thé")
        self.assertEqual(run.call_args.args[0], ["pbcopy"])
        self.assertEqual(run.call_args.kwargs["input"], "thé".encode("utf-8"))
        self.assertEqual(run.call_args.kwargs["env"]["LC_ALL"], "en_US.UTF-8")

    def test_paste_needs_accessibility(self):
        err = b"execution error: System Events got an error: osascript is not allowed to send keystrokes. (1002)\n"
        with mock.patch.object(clip.subprocess, "run", return_value=done(b"", 1, err)):
            with self.assertRaisesRegex(RuntimeError, "send keystrokes"):
                clip.MacClipboard().paste("ctrl+v")
        err = b"System Events got an error: osascript is not allowed assistive access. (-1719)"
        with mock.patch.object(clip.subprocess, "run", return_value=done(b"", 1, err)):
            with self.assertRaisesRegex(RuntimeError, "Accessibility"):
                clip.MacClipboard().paste("ctrl+v")


class LinuxTest(Base):
    def which(self, *present):
        return lambda name: f"/usr/bin/{name}" if name in present else None

    def test_tool_choice(self):
        wl = clip.LinuxClipboard({"WAYLAND_DISPLAY": "wayland-0"}, self.which("wl-copy", "wl-paste", "xclip", "wtype", "xdotool"))
        self.assertEqual((wl.tool, wl.keys), ("wl", "wtype"))
        x = clip.LinuxClipboard({"DISPLAY": ":0"}, self.which("wl-copy", "wl-paste", "xsel", "xdotool", "ydotool"))
        self.assertEqual((x.tool, x.keys), ("xsel", "xdotool"))
        none = clip.LinuxClipboard({"DISPLAY": ":0"}, self.which())
        with self.assertRaisesRegex(RuntimeError, "xclip or xsel"):
            none.get()
        with self.assertRaisesRegex(RuntimeError, "xdotool"):
            none.paste("ctrl+v")

    def test_xclip_get_and_set(self):
        cb = clip.LinuxClipboard({"DISPLAY": ":0"}, self.which("xclip", "xdotool"))
        with mock.patch.object(clip.subprocess, "run", return_value=done("ñ".encode())) as run:
            self.assertEqual(cb.get(), "ñ")
        self.assertEqual(run.call_args.args[0], ["xclip", "-selection", "clipboard", "-o", "-t", "UTF8_STRING"])
        with mock.patch.object(clip.subprocess, "run", return_value=done(b"", 1, b"Error: target UTF8_STRING not available")):
            self.assertIsNone(cb.get())
        with mock.patch.object(clip.subprocess, "Popen") as popen:
            popen.return_value.communicate.return_value = (None, None)
            popen.return_value.returncode = 0
            cb.set("hi")
        self.assertEqual(popen.call_args.args[0], ["xclip", "-selection", "clipboard", "-i"])
        self.assertEqual(popen.call_args.kwargs["stdout"], subprocess.DEVNULL)  # xclip lingers
        popen.return_value.communicate.assert_called_with(b"hi", timeout=clip.TIMEOUT)

    def test_wayland_set_and_clear(self):
        cb = clip.LinuxClipboard({"WAYLAND_DISPLAY": "wayland-0"}, self.which("wl-copy", "wl-paste", "ydotool"))
        with mock.patch.object(clip.subprocess, "Popen") as popen:
            popen.return_value.communicate.return_value = (None, None)
            popen.return_value.returncode = 0
            cb.set("hi")
        self.assertEqual(popen.call_args.args[0], ["wl-copy", "--type", "text/plain;charset=utf-8"])
        with mock.patch.object(clip.subprocess, "run", return_value=done()) as run:
            cb.set("")
            cb.paste("shift+insert")
        self.assertEqual(run.call_args_list[0].args[0], ["wl-copy", "--clear"])
        self.assertEqual(run.call_args_list[1].args[0], ["ydotool", "key", "42:1", "110:1", "110:0", "42:0"])

    def test_paste_keys(self):
        cb = clip.LinuxClipboard({"DISPLAY": ":0"}, self.which("xclip", "xdotool"))
        with mock.patch.object(clip.subprocess, "run", return_value=done()) as run:
            cb.paste("ctrl+shift+v")
        self.assertEqual(run.call_args.args[0], ["xdotool", "key", "--clearmodifiers", "ctrl+shift+v"])

    def test_dispatch(self):
        self.assertIsInstance(clip.make_clipboard("win32"), clip.WindowsClipboard)
        self.assertIsInstance(clip.make_clipboard("darwin"), clip.MacClipboard)
        self.assertIsInstance(clip.make_clipboard("linux"), clip.LinuxClipboard)


if __name__ == "__main__":
    unittest.main()
