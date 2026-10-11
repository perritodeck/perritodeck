import datetime
import json
import os
import sys
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as clock  # noqa: E402

UTC = datetime.timezone.utc
HERE = datetime.timezone(datetime.timedelta(hours=1))  # the "computer's" zone in these tests


def at(*args, tz=UTC):
    return datetime.datetime(*args, tzinfo=tz)


class ClockTest(unittest.TestCase):
    def setUp(self):
        clock.LOCAL_TZ = HERE
        self.sent = []
        clock.plugin._send = self.sent.append
        clock.plugin.settings = {}
        clock.plugin.watching = set()
        clock._published.clear()
        clock._hinted.clear()
        clock._zones.clear()

    def values(self):
        return {m["id"]: m["value"] for m in self.sent if m.get("event") == "value"}

    def logs(self):
        return [m["message"] for m in self.sent if m.get("event") == "log"]

    def tick(self, moment):
        self.sent.clear()
        clock.tick(moment)
        return self.values()

    # --- fixed values ---

    def test_local_time_date_and_week_always_published(self):
        got = self.tick(at(2026, 10, 11, 14, 7, 30))  # 15:07 here
        self.assertEqual(got, {"time": "15:07", "date": "Sun 11 Oct", "week": "41"})

    def test_publishes_only_on_change(self):
        self.tick(at(2026, 10, 11, 14, 7, 0))
        self.assertEqual(self.tick(at(2026, 10, 11, 14, 7, 59)), {})
        self.assertEqual(self.tick(at(2026, 10, 11, 14, 8, 0)), {"time": "15:08"})

    def test_12_hour_format(self):
        clock.plugin.settings = {"format": "12h"}
        self.assertEqual(self.tick(at(2026, 10, 11, 14, 7))["time"], "3:07 PM")
        clock._published.clear()
        self.assertEqual(self.tick(at(2026, 10, 10, 23, 5))["time"], "12:05 AM")
        clock._published.clear()
        self.assertEqual(self.tick(at(2026, 10, 11, 11, 0))["time"], "12:00 PM")

    def test_settings_change_resends(self):
        clock.plugin.settings = {}
        self.tick(at(2026, 10, 11, 14, 7))
        clock.configure({"format": "12h", "date_format": "iso"})
        clock.plugin.settings = {"format": "12h", "date_format": "iso"}
        self.assertEqual(self.tick(at(2026, 10, 11, 14, 7)), {"time": "3:07 PM", "date": "2026-10-11", "week": "41"})

    def test_date_formats(self):
        moment = at(2026, 3, 5, 12, 0, tz=HERE)
        expected = {"short": "Thu 5 Mar", "long": "5 Mar 2026", "iso": "2026-03-05", "us": "3/5/2026", "eu": "05/03/2026"}
        for style, text in expected.items():
            self.assertEqual(clock.format_date(moment, {"date_format": style}), text)

    # --- time zones ---

    def test_named_time_zone(self):
        clock.plugin.watching = {"time:Asia/Tokyo", "date:America/New_York", "week:Pacific/Auckland"}
        got = self.tick(at(2026, 10, 11, 14, 7))
        self.assertEqual(got["time:Asia/Tokyo"], "23:07")
        self.assertEqual(got["date:America/New_York"], "Sun 11 Oct")
        # Monday 12 October in Auckland already: week 42
        self.assertEqual(got["week:Pacific/Auckland"], "42")

    def test_fixed_offsets(self):
        for text, expected in [("UTC+5:30", "19:37"), ("+05:30", "19:37"), ("UTC-3", "11:07"), ("GMT+0530", "19:37"),
                               ("UTC", "14:07"), ("utc+530", "19:37"), ("local", "15:07")]:
            clock.plugin.watching = {f"time:{text}"}
            clock._published.clear()
            self.assertEqual(self.tick(at(2026, 10, 11, 14, 7))[f"time:{text}"], expected, text)

    def test_bad_zone_shows_dash_and_logs_once(self):
        clock.plugin.watching = {"time:Mars/Olympus", "time:UTC+15"}
        got = self.tick(at(2026, 10, 11, 14, 7))
        self.assertEqual(got["time:Mars/Olympus"], "–")
        self.assertEqual(got["time:UTC+15"], "–")
        self.assertEqual(len(self.logs()), 2)
        self.assertIn("unknown time zone", " ".join(self.logs()))
        self.tick(at(2026, 10, 11, 14, 8))
        self.assertEqual(self.logs(), [])

    def test_no_time_zone_database_hint(self):
        fake = mock.Mock()
        fake.ZoneInfo.side_effect = KeyError("Europe/Lisbon")
        fake.available_timezones.return_value = set()
        with mock.patch.object(clock, "zoneinfo", fake):
            clock.plugin.watching = {"time:Europe/Lisbon", "time:UTC+1"}
            got = self.tick(at(2026, 10, 11, 14, 7))
        self.assertEqual(got["time:Europe/Lisbon"], "–")
        self.assertEqual(got["time:UTC+1"], "15:07")
        self.assertIn("tzdata", self.logs()[0])

    def test_python_without_zoneinfo(self):
        with mock.patch.object(clock, "zoneinfo", None):
            with self.assertRaisesRegex(LookupError, "Python 3.9"):
                clock.zone("Europe/Lisbon")
            self.assertEqual(clock.zone("UTC-2").utcoffset(None), datetime.timedelta(hours=-2))

    # --- countdowns ---

    def test_countdown_formats(self):
        target = clock.parse_moment("2026-12-24T18:00")  # local (UTC+1)
        cases = [
            (datetime.timedelta(days=12, hours=4, minutes=30), "12d 4h"),
            (datetime.timedelta(hours=3, minutes=12, seconds=30), "3h 12m"),
            (datetime.timedelta(hours=1), "1h 0m"),
            (datetime.timedelta(minutes=4, seconds=59), "4:59"),
            (datetime.timedelta(minutes=4, seconds=58, milliseconds=200), "4:59"),
            (datetime.timedelta(seconds=1), "0:01"),
            (datetime.timedelta(0), "now"),
            (datetime.timedelta(seconds=-30), "now"),
        ]
        for left, text in cases:
            self.assertEqual(clock.format_countdown(target, target - left), text, left)

    def test_countdown_value_ticks_each_second_under_an_hour(self):
        clock.plugin.watching = {"countdown:2026-10-11T16:00"}  # local, so 15:00 UTC
        self.assertEqual(self.tick(at(2026, 10, 11, 14, 55, 0))["countdown:2026-10-11T16:00"], "5:00")
        self.assertEqual(self.tick(at(2026, 10, 11, 14, 55, 1)), {"countdown:2026-10-11T16:00": "4:59"})
        self.assertEqual(self.tick(at(2026, 10, 11, 15, 0, 0))["countdown:2026-10-11T16:00"], "now")

    def test_countdown_with_offset_and_z(self):
        moment = at(2026, 10, 11, 12, 0)
        self.assertEqual(clock.format_countdown(clock.parse_moment("2026-10-11T15:00Z"), moment), "3h 0m")
        self.assertEqual(clock.format_countdown(clock.parse_moment("2026-10-11T15:00+02:00"), moment), "1h 0m")
        self.assertEqual(clock.format_countdown(clock.parse_moment("2026-10-13"), moment), "1d 11h")

    def test_bad_countdown(self):
        clock.plugin.watching = {"countdown:christmas"}
        self.assertEqual(self.tick(at(2026, 10, 11, 12, 0))["countdown:christmas"], "–")
        self.assertIn("2026-12-24T18:00", self.logs()[0])

    # --- days until ---

    def test_days_until(self):
        moment = at(2026, 10, 11, 23, 30)  # already 12 October here
        self.assertEqual(clock.days_until("2026-12-24", moment), 73)
        self.assertEqual(clock.days_until("2026-10-12", moment), 0)
        self.assertEqual(clock.days_until("2026-10-10", moment), -2)
        self.assertEqual(clock.days_until("2026-12-24T18:00", moment), 73)

    def test_days_until_every_year(self):
        moment = at(2026, 12, 25, 12, 0)
        self.assertEqual(clock.days_until("12-24", moment), 364)
        self.assertEqual(clock.days_until("12-25", moment), 0)
        self.assertEqual(clock.days_until("02-29", moment), 431)  # 29 February 2028
        with self.assertRaises(ValueError):
            clock.days_until("13-01", moment)
        with self.assertRaises(ValueError):
            clock.days_until("02-30", moment)

    def test_days_until_value(self):
        clock.plugin.watching = {"days_until:2026-12-24", "days_until:soon"}
        got = self.tick(at(2026, 10, 11, 12, 0))
        self.assertEqual(got["days_until:2026-12-24"], "74")
        self.assertEqual(got["days_until:soon"], "–")

    # --- watch list ---

    def test_only_watched_values_and_forgets_unwatched(self):
        clock.plugin.watching = {"time:UTC", "up:nas"}
        got = self.tick(at(2026, 10, 11, 14, 7))
        self.assertEqual(set(got), {"time", "date", "week", "time:UTC"})
        clock.plugin.watching = set()
        self.sent.clear()
        clock.watching([])
        self.assertNotIn("time:UTC", clock._published)
        clock.plugin.watching = {"time:UTC"}
        self.sent.clear()
        clock.watching(["time:UTC"])
        self.assertIn("time:UTC", self.values())  # sent again when watched again

    def test_manifest(self):
        with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "perritodeck-plugin.json"), encoding="utf-8") as f:
            manifest = json.load(f)
        self.assertEqual(manifest["id"], "dev.perritodeck.clock")
        self.assertEqual(manifest["actions"], [])
        self.assertEqual({v["id"] for v in manifest["values"]}, set(clock.ARG_VALUES))


if __name__ == "__main__":
    unittest.main()
