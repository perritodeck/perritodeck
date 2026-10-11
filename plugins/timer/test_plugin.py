import json
import os
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as timer  # noqa: E402


class FakeClock:
    def __init__(self, now=1_000_000.0):
        self.now = now

    def __call__(self):
        return self.now


class TimerTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        timer.STATE_FILE = os.path.join(self.dir.name, "sub", "timer-state.json")
        self.clock = timer.clock = FakeClock()
        self.sent = []
        timer.plugin._send = self.sent.append
        timer.plugin.settings = {}
        timer.plugin.watching = set()
        timer._published.clear()
        timer.load()

    def tearDown(self):
        self.dir.cleanup()

    def watch(self, *values):
        timer.plugin.watching = set(values)

    def value(self, key):
        """The latest published value for key."""
        timer.publish()
        found = [m["value"] for m in self.sent if m.get("event") == "value" and m["id"] == key]
        return found[-1] if found else None

    def test_formatting(self):
        self.assertEqual(timer.clock_text(0), "0:00")
        self.assertEqual(timer.clock_text(65), "1:05")
        self.assertEqual(timer.clock_text(3725), "1:02:05")
        self.assertEqual(timer.countdown_text(4.2), "0:05")
        self.assertEqual(timer.countdown_text(0), "0:00")

    def test_countdown_runs_pauses_and_finishes(self):
        self.watch("remaining:tea", "running:tea", "done:tea", "progress:tea", "remaining:other")
        self.assertIsNone(timer.start({"name": "tea", "minutes": "3", "seconds": 30}))
        self.assertEqual(self.value("remaining:tea"), "3:30")
        self.assertEqual(self.value("running:tea"), "on")
        self.assertEqual(self.value("remaining:other"), "–")
        self.clock.now += 105
        self.assertEqual(self.value("remaining:tea"), "1:45")
        self.assertEqual(self.value("progress:tea"), "50")

        timer.toggle({"name": "tea"})  # pause
        self.assertEqual(self.value("running:tea"), "off")
        self.clock.now += 1000
        self.assertEqual(self.value("remaining:tea"), "1:45")
        timer.toggle({"name": "tea"})  # carry on
        self.clock.now += 105
        timer.tick()
        self.assertEqual(self.value("remaining:tea"), "0:00")
        self.assertEqual(self.value("done:tea"), "on")
        self.assertEqual(self.value("running:tea"), "off")
        self.assertTrue(any("finished" in m.get("message", "") for m in self.sent))

        timer.reset({"name": "tea"})
        self.assertEqual(self.value("remaining:tea"), "3:30")
        self.assertEqual(self.value("done:tea"), "off")

    def test_toggle_restarts_finished_timer_and_reuses_length(self):
        timer.start({"name": "egg", "minutes": 1})
        self.clock.now += 61
        timer.tick()
        timer.toggle({"name": "egg"})
        t = timer.state["timers"]["egg"]
        self.assertFalse(t["done"])
        self.assertEqual(t["end"], self.clock.now + 60)

    def test_add_time(self):
        self.watch("remaining:t")
        timer.start({"name": "t", "minutes": 1})
        timer.add({"name": "t", "minutes": 2})
        self.assertEqual(self.value("remaining:t"), "3:00")
        timer.add({"name": "t", "minutes": -1, "seconds": "-30"})
        self.assertEqual(self.value("remaining:t"), "1:30")
        timer.pause({"name": "t"})
        timer.add({"name": "t", "minutes": -5})
        self.assertTrue(timer.state["timers"]["t"]["done"])
        self.assertIn("finished", timer.add({"name": "t", "seconds": -5}))
        timer.add({"name": "t", "seconds": 30})
        self.assertEqual(self.value("remaining:t"), "0:30")

    def test_errors(self):
        with self.assertRaisesRegex(ValueError, "name"):
            timer.start({"minutes": 1})
        with self.assertRaisesRegex(ValueError, "how long"):
            timer.start({"name": "x"})
        with self.assertRaisesRegex(ValueError, "number"):
            timer.start({"name": "x", "minutes": "five"})
        with self.assertRaisesRegex(ValueError, "no timer called"):
            timer.pause({"name": "nope"})
        with self.assertRaisesRegex(ValueError, "how much"):
            timer.add({"name": "nope"})

    def test_state_survives_a_restart(self):
        timer.start({"name": "bread", "minutes": 40})
        with open(timer.STATE_FILE, encoding="utf-8") as f:
            self.assertIn("bread", json.load(f)["timers"])
        timer.state = {"timers": {}, "stopwatches": {}, "pomodoro": timer.new_pomodoro()}
        self.clock.now += 600
        timer.load()
        self.watch("remaining:bread")
        self.assertEqual(self.value("remaining:bread"), "30:00")

    def test_broken_state_file_is_ignored(self):
        os.makedirs(os.path.dirname(timer.STATE_FILE))
        with open(timer.STATE_FILE, "w") as f:
            f.write("{nope")
        timer.load()
        self.assertEqual(timer.state["timers"], {})

    def test_stopwatch(self):
        self.watch("stopwatch:run", "lap:run", "running:run")
        self.assertEqual(self.value("stopwatch:run"), "0:00")
        self.assertEqual(self.value("lap:run"), "–")
        timer.stopwatch_toggle({"name": "run"})
        self.clock.now += 75.5
        self.assertEqual(self.value("stopwatch:run"), "1:15")
        self.assertEqual(self.value("running:run"), "on")
        timer.stopwatch_lap({"name": "run"})
        self.clock.now += 30
        timer.stopwatch_lap({"name": "run"})
        self.assertEqual(self.value("lap:run"), "0:30")
        timer.stopwatch_toggle({"name": "run"})
        self.clock.now += 500
        self.assertEqual(self.value("stopwatch:run"), "1:45")
        self.assertEqual(self.value("running:run"), "off")
        timer.stopwatch_reset({"name": "run"})
        self.assertEqual(self.value("stopwatch:run"), "0:00")
        with self.assertRaises(ValueError):
            timer.stopwatch_lap({"name": "other"})

    def test_pomodoro_cycle(self):
        self.assertEqual(self.value("pomodoro_phase"), "Ready")
        self.assertEqual(self.value("pomodoro_remaining"), "25:00")
        self.assertIn("hasn't started", timer.pomodoro_skip({}))
        timer.pomodoro_start({"work": 2, "short": 1, "long": 3, "rounds": 2, "auto": True})
        self.assertEqual(self.value("pomodoro_phase"), "Work")
        self.assertEqual(self.value("pomodoro_round"), "1/2")
        self.assertEqual(self.value("pomodoro_running"), "on")
        self.clock.now += 120
        timer.tick()
        self.assertEqual(self.value("pomodoro_phase"), "Short break")
        self.assertEqual(self.value("pomodoro_remaining"), "1:00")
        timer.pomodoro_skip({})
        self.assertEqual(self.value("pomodoro_phase"), "Work")
        self.assertEqual(self.value("pomodoro_round"), "2/2")
        timer.pomodoro_skip({})
        self.assertEqual(self.value("pomodoro_phase"), "Long break")
        self.clock.now += 90
        self.assertEqual(self.value("pomodoro_progress"), "50")
        timer.pomodoro_start({})  # pause
        self.assertEqual(self.value("pomodoro_running"), "off")
        timer.pomodoro_start({})  # carry on
        self.clock.now += 90
        timer.tick()
        self.assertEqual(self.value("pomodoro_phase"), "Work")
        self.assertEqual(self.value("pomodoro_round"), "1/2")
        timer.pomodoro_reset({})
        self.assertEqual(self.value("pomodoro_phase"), "Ready")
        self.assertEqual(self.value("pomodoro_remaining"), "2:00")

    def test_pomodoro_waits_when_auto_is_off(self):
        timer.pomodoro_start({"work": 1, "auto": False})
        self.clock.now += 60
        timer.tick()
        self.assertEqual(self.value("pomodoro_phase"), "Short break")
        self.assertEqual(self.value("pomodoro_running"), "off")
        timer.pomodoro_reset({})
        with self.assertRaisesRegex(ValueError, "more than 0"):
            timer.pomodoro_start({"work": 0})

    def test_only_changes_are_published(self):
        self.watch("remaining:a")
        timer.publish()
        count = len(self.sent)
        timer.publish()
        self.assertEqual(len(self.sent), count)

    def test_tick_delay(self):
        self.assertIsNone(timer.tick())
        timer.start({"name": "a", "seconds": 10.25})
        self.assertAlmostEqual(timer.tick(), 0.26)

    def test_sound_setting(self):
        timer.plugin.settings = {"sound": True}
        with mock.patch.object(timer, "_popen") as popen, mock.patch.object(timer.sys, "platform", "linux"):
            timer.start({"name": "a", "seconds": 1})
            self.clock.now += 2
            timer.tick()
        popen.assert_called_once()
        self.assertEqual(popen.call_args[0][0][0], "paplay")


if __name__ == "__main__":
    unittest.main()
