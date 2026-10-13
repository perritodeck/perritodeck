import base64
import os
import subprocess
import sys
import threading
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as media  # noqa: E402

SEP = "\x1f"


def done(stdout="", returncode=0, stderr=""):
    return subprocess.CompletedProcess([], returncode, stdout=stdout, stderr=stderr)


# What Windows PowerShell 5.1 wrote on stderr for an uncaught error under
# -EncodedCommand; shortened.
CLIXML_ERROR = (
    "#< CLIXML\r\n"
    '<Objs Version="1.1.0.1" xmlns="http://schemas.microsoft.com/powershell/2004/04">'
    '<S S="Error">Unable to find type [Windows.Media.Control.X &amp; Y]._x000D__x000A_</S>'
    '<S S="Error">At line:30 char:5_x000D__x000A_</S>'
    '<S S="Error"> _x000D__x000A_</S></Objs>'
)

# An error holding an emoji (U+1F44B): each UTF-16 half is escaped on its own.
CLIXML_EMOJI_ERROR = (
    "#< CLIXML\r\n"
    '<Objs Version="1.1.0.1" xmlns="http://schemas.microsoft.com/powershell/2004/04">'
    '<S S="Error">hi _xD83D__xDC4B_ there_x000D__x000A_</S>'
    '<S S="Error">At line:2 char:1_x000D__x000A_</S></Objs>'
)


class MediaTest(unittest.TestCase):
    def setUp(self):
        media.forget_state()

    def test_describe(self):
        self.assertEqual(media.describe("Artist", "Song"), "Artist – Song")
        self.assertEqual(media.describe("", "Song"), "Song")
        self.assertEqual(media.describe("", ""), media.NOTHING)

    def test_missing_playerctl_is_explained(self):
        with mock.patch.object(media.sys, "platform", "linux"), mock.patch.object(media, "PLAYERCTL", None):
            with self.assertRaisesRegex(RuntimeError, "playerctl"):
                media.play_pause({})


class DispatchTest(unittest.TestCase):
    def setUp(self):
        media.forget_state()

    def test_backend_by_platform(self):
        self.assertEqual(media.backend("win32"), (media.windows_state, media.windows_control))
        self.assertEqual(media.backend("darwin"), (media.mac_state, media.mac_control))
        self.assertEqual(media.backend("linux"), (media.linux_state, media.linux_control))
        self.assertEqual(media.backend("freebsd14"), (media.linux_state, media.linux_control))

    def test_actions_use_the_platform_backend(self):
        for platform, program in (("win32", "powershell.exe"), ("darwin", "osascript")):
            with self.subTest(platform=platform):
                with mock.patch.object(media.sys, "platform", platform), mock.patch.object(
                    media.subprocess, "run", return_value=done("")
                ) as run:
                    with self.assertRaises(RuntimeError):  # empty output: no player / no answer
                        media.next_track({})
                self.assertEqual(run.call_args_list[0].args[0][0], program)

    def test_linux_uses_playerctl(self):
        with mock.patch.object(media.sys, "platform", "linux"), mock.patch.object(
            media, "PLAYERCTL", "/usr/bin/playerctl"
        ), mock.patch.object(media.subprocess, "run", return_value=done("")) as run:
            media.next_track({})
        run.assert_called_once()
        self.assertEqual(run.call_args.args[0], ["/usr/bin/playerctl", "next"])

    def test_state_is_cached_briefly(self):
        state = mock.Mock(return_value=("playing", "A – B"))
        with mock.patch.object(media, "backend", return_value=(state, None)):
            self.assertEqual(media.current_state(), ("playing", "A – B"))
            self.assertEqual(media.current_state(), ("playing", "A – B"))
            self.assertEqual(state.call_count, 1)
            media.forget_state()
            media.current_state()
            self.assertEqual(state.call_count, 2)

    def test_actions_forget_the_cache(self):
        state = mock.Mock(return_value=("paused", "x"))
        with mock.patch.object(media, "backend", return_value=(state, mock.Mock())):
            media.current_state()
            media.play_pause({})
            media.current_state()
        self.assertEqual(state.call_count, 2)

    def test_a_press_doesnt_wait_for_a_status_query(self):
        # A Windows status query takes about 2 s; a key press during one used to wait
        # for it to finish before the key got its answer.
        started, release = threading.Event(), threading.Event()
        answers = iter([("paused", "before the press"), ("playing", "after the press")])

        def slow_state():
            started.set()
            release.wait(5)
            return next(answers)

        control = mock.Mock()
        polled = []
        with mock.patch.object(media, "backend", return_value=(slow_state, control)):
            poller = threading.Thread(target=lambda: polled.append(media.current_state()))
            poller.start()
            self.assertTrue(started.wait(5))
            pressed = threading.Thread(target=media.play_pause, args=({},))
            pressed.start()
            pressed.join(2)
            self.assertFalse(pressed.is_alive(), "the press waited for the status query")
            control.assert_called_once_with("play-pause")
            release.set()
            poller.join(5)
            self.assertEqual(polled, [("paused", "before the press")])
            # That answer may predate the press, so it isn't cached: the next poll asks again.
            self.assertEqual(media.current_state(), ("playing", "after the press"))
            self.assertEqual(media.current_state(), ("playing", "after the press"))  # cached now


class WindowsTest(unittest.TestCase):
    def run_windows(self, stdout, fn, *args, stderr=""):
        with mock.patch.object(media.subprocess, "run", return_value=done(stdout, stderr=stderr)) as run:
            return fn(*args), run

    def test_status_parsing(self):
        line = '{"ok":true,"session":true,"artist":"Caf\\u00e9 Band","title":"Song","status":"Playing"}'
        state, run = self.run_windows("noise\r\n" + line + "\r\n", media.windows_state)
        self.assertEqual(state, ("playing", "Café Band – Song"))
        args = run.call_args
        self.assertEqual(args.args[0][:4], ["powershell.exe", "-NoProfile", "-NonInteractive", "-EncodedCommand"])
        script = base64.b64decode(args.args[0][4]).decode("utf-16-le")
        self.assertTrue(script.startswith("$cmd = 'status'\n"))
        self.assertIn("GlobalSystemMediaTransportControlsSessionManager", script)
        self.assertEqual(args.kwargs["creationflags"], 0x08000000)
        self.assertEqual(args.kwargs["timeout"], media.WINDOWS_TIMEOUT)
        self.assertIs(args.kwargs["stdin"], subprocess.DEVNULL)

    def test_status_values(self):
        for raw, expected in (("Paused", "paused"), ("Stopped", "stopped"), ("Changing", "stopped"), (None, "stopped")):
            self.assertEqual(media.windows_status(raw), expected)

    def test_no_session(self):
        state, _ = self.run_windows('{"ok":true,"session":false}', media.windows_state)
        self.assertEqual(state, ("stopped", media.NOTHING))

    def test_missing_title_fields(self):
        state, _ = self.run_windows('{"ok":true,"session":true,"artist":null,"title":"","status":"Paused"}', media.windows_state)
        self.assertEqual(state, ("paused", media.NOTHING))

    def test_errors(self):
        with self.assertRaisesRegex(RuntimeError, "boom"):
            self.run_windows('{"ok":false,"error":"boom"}', media.windows_state)
        with self.assertRaisesRegex(RuntimeError, "no player is running"):
            self.run_windows('{"ok":false,"error":"no player is running"}', media.windows_control, "next")
        with self.assertRaisesRegex(RuntimeError, "refused"):
            self.run_windows('{"ok":false}', media.windows_control, "next")
        with self.assertRaisesRegex(RuntimeError, "not recognized"):
            self.run_windows("", media.windows_state, stderr="powershell.exe is not recognized")

    def test_clixml_errors_become_their_message(self):
        with self.assertRaises(RuntimeError) as caught:
            self.run_windows("", media.windows_state, stderr=CLIXML_ERROR)
        self.assertEqual(str(caught.exception), "Unable to find type [Windows.Media.Control.X & Y].")
        self.assertEqual(media.error_text(CLIXML_ERROR.split("<S S=")[0] + "</Objs>"), "")
        self.assertEqual(media.error_text(CLIXML_EMOJI_ERROR), "hi \U0001F44B there")
        self.assertEqual(media.error_text(CLIXML_EMOJI_ERROR.replace("_xDC4B_", "")), "hi � there")
        self.assertEqual(media.error_text("one\ntwo\n"), "two")

    def test_controls(self):
        for command in media.COMMANDS:
            _, run = self.run_windows('{"ok":true}', media.windows_control, command)
            script = base64.b64decode(run.call_args.args[0][4]).decode("utf-16-le")
            self.assertTrue(script.startswith(f"$cmd = '{command}'\n"))

    def test_unknown_command_never_reaches_powershell(self):
        with mock.patch.object(media.subprocess, "run") as run:
            with self.assertRaises(ValueError):
                media.windows_run("x'; Remove-Item C:\\ #")
        run.assert_not_called()

    def test_script_mentions_every_winrt_call(self):
        for name in ("TryTogglePlayPauseAsync", "TrySkipNextAsync", "TrySkipPreviousAsync",
                     "TryGetMediaPropertiesAsync", "GetPlaybackInfo", "System.WindowsRuntimeSystemExtensions"):
            self.assertIn(name, media.WINDOWS_SCRIPT)


class MacTest(unittest.TestCase):
    def fake_osascript(self, found, answer="ok"):
        """subprocess.run that says `found` is running and answers app scripts with `answer`."""

        def run(argv, **kwargs):
            self.assertEqual(argv[:2], ["osascript", "-e"])
            self.assertIs(kwargs["stdin"], subprocess.DEVNULL)
            self.assertEqual(kwargs["timeout"], media.TIMEOUT)
            if argv[2] == media.MAC_FIND_SCRIPT:
                return done(found + "\n")
            return done(answer + "\n")

        return mock.patch.object(media.subprocess, "run", side_effect=run)

    def test_find_script_never_launches(self):
        self.assertNotIn("tell", media.MAC_FIND_SCRIPT)
        self.assertNotIn("launch", media.MAC_FIND_SCRIPT)
        self.assertNotIn("activate", media.MAC_FIND_SCRIPT)
        self.assertLess(media.MAC_FIND_SCRIPT.index('"Spotify"'), media.MAC_FIND_SCRIPT.index('"Music"'))

    def test_status_parsing(self):
        with self.fake_osascript("Spotify", f"state{SEP}playing{SEP}Artist{SEP}Song") as run:
            self.assertEqual(media.mac_state(), ("playing", "Artist – Song"))
        app = run.call_args_list[1].args[0]
        self.assertIn('tell application "Spotify"', app[2])
        self.assertNotIn("Music", app[2])
        self.assertEqual(app[3:], ["status"])

    def test_status_values(self):
        cases = {
            f"state{SEP}paused{SEP}A{SEP}B": ("paused", "A – B"),
            f"state{SEP}stopped{SEP}{SEP}": ("stopped", media.NOTHING),
            f"state{SEP}playing{SEP}missing value{SEP}Song": ("playing", "Song"),
            f"state{SEP}«constant ****kPSp»{SEP}{SEP}T": ("paused", "T"),
            f"state{SEP}«constant ****kPSP»{SEP}{SEP}T": ("playing", "T"),
            f"state{SEP}playing{SEP}A{SEP}Title{SEP}with sep": ("playing", f"A – Title{SEP}with sep"),
            "none": ("stopped", media.NOTHING),
        }
        for output, expected in cases.items():
            self.assertEqual(media.parse_mac(output), expected, output)

    def test_music_when_spotify_is_not_running(self):
        with self.fake_osascript("Music", "ok") as run:
            media.mac_control("previous")
        app = run.call_args_list[1].args[0]
        self.assertIn('tell application "Music"', app[2])
        self.assertEqual(app[3:], ["previous"])

    def test_controls(self):
        for command, word in (("play-pause", "playpause"), ("next", "next"), ("previous", "previous")):
            with self.fake_osascript("Spotify") as run:
                media.mac_control(command)
            self.assertEqual(run.call_args.args[0][3:], [word])
        for word in ("playpause", "next track", "previous track"):
            self.assertIn(word, media.MAC_APP_SCRIPT)

    def test_nothing_launches_when_no_player_runs(self):
        with self.fake_osascript("") as run:
            self.assertEqual(media.mac_state(), ("stopped", media.NOTHING))
            with self.assertRaisesRegex(RuntimeError, "running"):
                media.mac_control("play-pause")
        for call in run.call_args_list:  # only the "is it running?" check ran
            self.assertEqual(call.args[0][2], media.MAC_FIND_SCRIPT)

    def test_app_that_quit_meanwhile_is_not_relaunched(self):
        self.assertIn('if application "%APP%" is not running then return "none"', media.MAC_APP_SCRIPT)
        self.assertLess(media.MAC_APP_SCRIPT.index("is not running"), media.MAC_APP_SCRIPT.index("tell application"))
        with self.fake_osascript("Spotify", "none"):
            with self.assertRaisesRegex(RuntimeError, "isn't running"):
                media.mac_control("next")

    def test_osascript_failure(self):
        with mock.patch.object(media.subprocess, "run", return_value=done("", 1, "execution error")):
            with self.assertRaisesRegex(RuntimeError, "execution error"):
                media.mac_state()


class ErrorTextTest(unittest.TestCase):
    def test_timeouts_do_not_put_the_command_line_on_the_key(self):
        slow = subprocess.TimeoutExpired(["powershell.exe", "-EncodedCommand", "A" * 4000], 8)
        with mock.patch.object(media.sys, "platform", "win32"), mock.patch.object(
            media.subprocess, "run", side_effect=slow
        ):
            with self.assertRaises(RuntimeError) as caught:
                media.next_track({})
        self.assertEqual(str(caught.exception), "the player didn't answer in 8 s")

    def test_long_errors_are_shortened(self):
        long = "line one\n" + "x" * 500
        with mock.patch.object(media.sys, "platform", "win32"), mock.patch.object(
            media.subprocess, "run", return_value=done("", stderr=long)
        ):
            with self.assertRaises(RuntimeError) as caught:
                media.next_track({})
        self.assertLessEqual(len(str(caught.exception)), media.MAX_ERROR)
        self.assertNotIn("\n", str(caught.exception))


class PollIntervalTest(unittest.TestCase):
    def test_windows_polls_less_when_idle(self):
        now = media._last_action
        self.assertEqual(media.poll_interval("win32", now + 5), media.POLL_SECONDS)
        self.assertEqual(media.poll_interval("win32", now + 31), media.IDLE_POLL_SECONDS)
        self.assertEqual(media.poll_interval("linux", now + 300), media.POLL_SECONDS)


class NoPlayerTest(unittest.TestCase):
    def test_windows_without_a_session_runs_no_control(self):
        with mock.patch.object(media.subprocess, "run", return_value=done('{"ok":true,"session":false}')) as run:
            self.assertEqual(media.windows_state(), ("stopped", media.NOTHING))
        run.assert_called_once()  # just the status query


if __name__ == "__main__":
    unittest.main()
