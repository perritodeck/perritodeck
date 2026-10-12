import os
import sys
import threading
import time
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as soundboard  # noqa: E402
import winmm  # noqa: E402


class FakeMci:
    """Answers MCI strings like the mpegvideo device: playing for `plays` polls."""

    def __init__(self, plays=3, fail=None):
        self.plays, self.fail = plays, fail
        self.sent = []
        self.lock = threading.Lock()

    def __call__(self, command):
        with self.lock:
            self.sent.append(command)
        verb = command.split()[0]
        if self.fail and verb == self.fail:
            raise winmm.MciError(275, "The specified file cannot be found.")
        if verb == "status":
            if self.plays is None:
                return "playing"
            self.plays -= 1
            return "playing" if self.plays >= 0 else "stopped"
        return ""

    def verbs(self):
        with self.lock:
            return [c.split()[0] for c in self.sent]


class MciSoundTest(unittest.TestCase):
    def setUp(self):
        patcher = mock.patch.object(winmm, "POLL_SECONDS", 0.005)
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_plays_to_the_end_then_closes(self):
        fake = FakeMci(plays=3)
        sound = winmm.MciSound("C:\\Sounds\\a b.mp3", 70, send=fake)
        self.assertEqual(sound.communicate(), (None, b""))
        self.assertEqual(sound.returncode, 0)
        self.assertEqual(sound.poll(), 0)
        alias = sound.alias
        self.assertEqual(fake.sent[0], f'open "C:\\Sounds\\a b.mp3" type mpegvideo alias {alias}')
        self.assertEqual(fake.sent[1], f"setaudio {alias} volume to 700")
        self.assertEqual(fake.sent[2], f"play {alias}")
        self.assertEqual(fake.sent[-2:], [f"stop {alias}", f"close {alias}"])

    def test_every_sound_has_its_own_alias(self):
        a = winmm.MciSound("a.wav", 100, send=FakeMci(plays=0))
        b = winmm.MciSound("b.wav", 100, send=FakeMci(plays=0))
        a.communicate()
        b.communicate()
        self.assertNotEqual(a.alias, b.alias)

    def test_terminate_stops_and_closes(self):
        fake = FakeMci(plays=None)  # would play forever
        sound = winmm.MciSound("a.mp3", 50, send=fake)
        time.sleep(0.03)
        self.assertIsNone(sound.poll())
        sound.terminate()
        sound.communicate()
        self.assertEqual(fake.verbs()[-2:], ["stop", "close"])

    def test_a_file_mci_cannot_open_fails_like_a_player(self):
        fake = FakeMci(fail="open")
        sound = winmm.MciSound("a.ogg", 100, send=fake)
        _, err = sound.communicate()
        self.assertEqual(sound.returncode, 1)
        self.assertIn(b"cannot be found", err)
        self.assertEqual(fake.verbs(), ["open"])  # nothing to close

    def test_volume_failure_still_plays(self):
        fake = FakeMci(plays=1, fail="setaudio")
        sound = winmm.MciSound("a.wav", 30, send=fake)
        sound.communicate()
        self.assertEqual(sound.returncode, 0)
        self.assertIn("play", fake.verbs())

    def test_loading_winmm_off_windows_is_an_oserror(self):
        if sys.platform == "win32":
            self.skipTest("winmm exists here")
        with self.assertRaises(OSError):
            winmm.MciSound("a.wav", 100)


class SpawnTest(unittest.TestCase):
    def test_spawn_uses_mci_for_the_mci_player(self):
        with mock.patch.object(soundboard.winmm, "MciSound") as mci_sound:
            soundboard.spawn([soundboard.MCI_PLAYER, "C:\\a.wav", "40"], {})
        mci_sound.assert_called_once_with("C:\\a.wav", 40)


if __name__ == "__main__":
    unittest.main()
