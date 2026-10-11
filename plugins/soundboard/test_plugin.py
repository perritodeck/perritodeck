import base64
import os
import sys
import tempfile
import time
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as soundboard  # noqa: E402

PY = sys.executable
LONG = [PY, "-c", "import time; time.sleep(30)"]
SHORT = [PY, "-c", "pass"]
FAIL = [PY, "-c", "import sys; sys.stderr.write('unsupported format\\n'); sys.exit(1)"]


def which_all(name):
    return "/usr/bin/" + name


def which_none(name):
    return None


def wait_until(check, timeout=3.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if check():
            return True
        time.sleep(0.02)
    return check()


class SoundboardTest(unittest.TestCase):
    def setUp(self):
        self.sent = []
        soundboard.plugin._send = self.sent.append
        soundboard.plugin.settings = {}
        soundboard.plugin.watching = set()
        soundboard._published.clear()
        self.dir = tempfile.TemporaryDirectory()
        self.wav = os.path.join(self.dir.name, "airhorn.wav")
        self.mp3 = os.path.join(self.dir.name, "applause.mp3")
        for path in (self.wav, self.mp3):
            with open(path, "wb") as f:
                f.write(b"RIFF")

    def tearDown(self):
        soundboard.stop_all()
        wait_until(lambda: not soundboard._sounds)
        self.dir.cleanup()

    def published(self):
        return [(m["id"], m["value"]) for m in self.sent if m.get("event") == "value"]

    def logs(self):
        return [m["message"] for m in self.sent if m.get("event") == "log"]

    def fake_players(self, *argvs):
        return mock.patch.object(soundboard, "player_commands", return_value=[(list(a), {}) for a in argvs])

    # --- pure parts ---

    def test_clean_path(self):
        self.assertEqual(soundboard.clean_path(f'  "{self.wav}" '), self.wav)
        self.assertEqual(soundboard.clean_path(""), "")
        with mock.patch.dict(os.environ, {"SOUNDS": self.dir.name}):
            self.assertEqual(soundboard.clean_path("$SOUNDS/airhorn.wav"), self.wav)

    def test_parse_volume(self):
        self.assertEqual(soundboard.parse_volume(None), 100)
        self.assertEqual(soundboard.parse_volume(""), 100)
        self.assertEqual(soundboard.parse_volume("40"), 40)
        self.assertEqual(soundboard.parse_volume(55.6), 56)
        self.assertEqual(soundboard.parse_volume(250), 100)
        self.assertEqual(soundboard.parse_volume(-3), 0)
        with self.assertRaisesRegex(ValueError, "isn't a number"):
            soundboard.parse_volume("loud")

    def test_linux_players_in_order(self):
        cmds = soundboard.player_commands("/s/a.wav", 50, "", "linux", which_all)
        self.assertEqual(
            [argv for argv, _ in cmds],
            [
                ["/usr/bin/pw-play", "--volume", "0.500", "/s/a.wav"],
                ["/usr/bin/paplay", "--volume=32768", "/s/a.wav"],
                ["/usr/bin/aplay", "-q", "/s/a.wav"],
                ["/usr/bin/ffplay", "-nodisp", "-autoexit", "-loglevel", "error", "-volume", "50", "/s/a.wav"],
            ],
        )

    def test_linux_device_and_mp3(self):
        cmds = soundboard.player_commands("/s/a.mp3", 100, "alsa_output.usb", "linux", which_all)
        argvs = [argv for argv, _ in cmds]
        self.assertIn(["/usr/bin/pw-play", "--volume", "1.000", "--target", "alsa_output.usb", "/s/a.mp3"], argvs)
        self.assertIn(["/usr/bin/paplay", "--volume=65536", "--device=alsa_output.usb", "/s/a.mp3"], argvs)
        self.assertNotIn("/usr/bin/aplay", [a[0] for a in argvs])  # aplay only plays WAV

    def test_linux_only_ffplay(self):
        only = lambda name: "/opt/ffplay" if name == "ffplay" else None  # noqa: E731
        cmds = soundboard.player_commands("/s/a.ogg", 0, "", "linux", only)
        self.assertEqual(cmds, [(["/opt/ffplay", "-nodisp", "-autoexit", "-loglevel", "error", "-volume", "0", "/s/a.ogg"], {})])
        self.assertEqual(soundboard.player_commands("/s/a.ogg", 0, "", "linux", which_none), [])

    def test_macos_afplay(self):
        cmds = soundboard.player_commands("/Users/me/a.mp3", 25, "ignored", "darwin", which_none)
        self.assertEqual(cmds, [(["/usr/bin/afplay", "-v", "0.250", "/Users/me/a.mp3"], {})])

    def test_windows_powershell(self):
        cmds = soundboard.player_commands("C:\\Sounds\\a b.mp3", 70, "", "win32", which_none)
        self.assertEqual(len(cmds), 1)
        argv, env = cmds[0]
        self.assertEqual(argv[:5], ["powershell.exe", "-NoProfile", "-NonInteractive", "-STA", "-EncodedCommand"])
        script = base64.b64decode(argv[5]).decode("utf-16-le")
        self.assertIn("System.Windows.Media.MediaPlayer", script)
        self.assertIn("System.Media.SoundPlayer", script)
        self.assertNotIn("a b.mp3", script)  # the path travels in the environment, unquoted
        self.assertEqual(env, {"PERRITODECK_SOUND": "C:\\Sounds\\a b.mp3", "PERRITODECK_VOLUME": "0.700"})

    def test_spawn_hides_the_console_on_windows(self):
        with mock.patch.object(soundboard.sys, "platform", "win32"), mock.patch.object(
            soundboard.subprocess, "Popen"
        ) as popen:
            soundboard.spawn(["powershell.exe"], {"PERRITODECK_SOUND": "x"})
        kwargs = popen.call_args.kwargs
        self.assertEqual(kwargs["creationflags"], soundboard.CREATE_NO_WINDOW)
        self.assertEqual(kwargs["stdin"], soundboard.subprocess.DEVNULL)
        self.assertEqual(kwargs["stdout"], soundboard.subprocess.DEVNULL)
        self.assertEqual(kwargs["env"]["PERRITODECK_SOUND"], "x")

    # --- playing with real child processes ---

    def test_play_and_value_follows(self):
        soundboard.plugin.watching = {f"playing:{self.wav}", f"playing:{self.mp3}"}
        soundboard.watch_changed(sorted(soundboard.plugin.watching))
        self.assertIn((f"playing:{self.wav}", "off"), self.published())
        with self.fake_players(SHORT):
            soundboard.play_action({"file": self.wav, "volume": 80})
        self.assertIn((f"playing:{self.wav}", "on"), self.published())
        self.assertTrue(wait_until(lambda: (f"playing:{self.wav}", "off") == self.published()[-1]))
        self.assertNotIn((f"playing:{self.mp3}", "on"), self.published())

    def test_modes(self):
        key = soundboard.sound_key(self.wav)
        with self.fake_players(LONG):
            self.assertEqual(soundboard.play(self.wav, 100, "play"), "started")
            first = soundboard._sounds[key][0]
            self.assertEqual(soundboard.play(self.wav, 100, "play"), "playing")
            self.assertEqual(len(soundboard._sounds[key]), 1)
            self.assertEqual(soundboard.play(self.wav, 100, "overlap"), "started")
            self.assertEqual(len(soundboard._sounds[key]), 2)
            self.assertEqual(soundboard.play(self.wav, 100, "restart"), "started")
            self.assertTrue(first.stopped)
            self.assertTrue(wait_until(lambda: len(soundboard._sounds.get(key, [])) == 1))
            self.assertEqual(soundboard.play(self.wav, 100, "toggle"), "stopped")
            self.assertTrue(wait_until(lambda: key not in soundboard._sounds))
            self.assertEqual(soundboard.play(self.wav, 100, "toggle"), "started")
        self.assertTrue(soundboard.is_playing(key))

    def test_stop_all(self):
        soundboard.plugin.watching = {f"playing:{self.wav}"}
        with self.fake_players(LONG):
            soundboard.play(self.wav, 100, "overlap")
            soundboard.play(self.wav, 100, "overlap")
            soundboard.play(self.mp3, 100, "play")
        self.assertEqual(self.published()[-1], (f"playing:{self.wav}", "on"))
        soundboard.stop_all_action({})
        self.assertEqual(soundboard._sounds, {})
        self.assertEqual(self.published()[-1], (f"playing:{self.wav}", "off"))
        self.assertFalse(any("couldn't play" in m for m in self.logs()))  # stopping isn't a failure

    def test_falls_back_to_the_next_player(self):
        key = soundboard.sound_key(self.mp3)
        with self.fake_players(FAIL, LONG):
            soundboard.play(self.mp3, 100, "play")
            self.assertTrue(wait_until(lambda: soundboard._sounds[key][0].proc.args == LONG))
        self.assertTrue(soundboard.is_playing(key))
        self.assertTrue(any("unsupported format" in m for m in self.logs()))

    def test_every_player_fails(self):
        key = soundboard.sound_key(self.mp3)
        with self.fake_players(FAIL, FAIL):
            soundboard.play(self.mp3, 100, "play")
            self.assertTrue(wait_until(lambda: key not in soundboard._sounds))
        self.assertTrue(wait_until(lambda: any("no player could play" in m for m in self.logs())))

    def test_missing_player_program_is_skipped(self):
        with self.fake_players(["/nonexistent/pw-play"], LONG):
            soundboard.play(self.wav, 100, "play")
        self.assertTrue(soundboard.is_playing(soundboard.sound_key(self.wav)))

    def test_errors(self):
        with self.assertRaisesRegex(ValueError, "choose a sound file"):
            soundboard.play_action({"file": "  "})
        with self.assertRaisesRegex(FileNotFoundError, "can't find the sound file"):
            soundboard.play_action({"file": os.path.join(self.dir.name, "missing.wav")})
        with self.assertRaisesRegex(ValueError, "unknown mode"):
            soundboard.play(self.wav, 100, "shuffle")
        with mock.patch.object(soundboard, "player_commands", return_value=[]):
            with self.assertRaisesRegex(RuntimeError, "no sound player.*ffmpeg"):
                soundboard.play_action({"file": self.wav})
        with self.fake_players(["/nonexistent/player"]):
            with self.assertRaisesRegex(RuntimeError, "could be started"):
                soundboard.play_action({"file": self.wav})

    def test_device_setting_reaches_the_player(self):
        soundboard.plugin.settings = {"device": " alsa_output.usb "}
        with mock.patch.object(soundboard, "player_commands", return_value=[(SHORT, {})]) as cmds:
            soundboard.play_action({"file": self.wav, "volume": "30"})
        self.assertEqual(cmds.call_args.args, (self.wav, 30, "alsa_output.usb"))

    def test_too_many_sounds(self):
        with mock.patch.object(soundboard, "MAX_SOUNDS", 2), self.fake_players(LONG):
            soundboard.play(self.wav, 100, "overlap")
            soundboard.play(self.wav, 100, "overlap")
            with self.assertRaisesRegex(RuntimeError, "already playing"):
                soundboard.play(self.wav, 100, "overlap")

    def test_device_setting_warns_off_linux(self):
        with mock.patch.object(soundboard.sys, "platform", "darwin"):
            soundboard.settings_changed({"device": "Headphones"})
        self.assertTrue(any("only used on Linux" in m for m in self.logs()))
        self.sent.clear()
        with mock.patch.object(soundboard.sys, "platform", "linux"):
            soundboard.settings_changed({"device": "Headphones"})
        self.assertEqual(self.logs(), [])


if __name__ == "__main__":
    unittest.main()
