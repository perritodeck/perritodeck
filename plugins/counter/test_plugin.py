import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as counter  # noqa: E402


class CounterTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        counter.STATE_FILE = os.path.join(self.dir.name, "cfg", "counters-state.json")
        self.sent = []
        counter.plugin._send = self.sent.append
        counter.plugin.settings = {}
        counter.plugin.watching = {"count:deaths"}
        counter.load()

    def tearDown(self):
        self.dir.cleanup()

    def published(self):
        return [(m["id"], m["value"]) for m in self.sent if m.get("event") == "value"]

    def test_count_up_down_set_reset(self):
        self.assertIsNone(counter.increment({"name": "deaths"}))
        counter.increment({"name": "deaths", "step": "5"})
        self.assertEqual(self.published()[-1], ("count:deaths", "6"))
        counter.decrement({"name": "deaths", "step": 2})
        self.assertEqual(counter.counts["deaths"], 4)
        counter.decrement({"name": "deaths", "step": 10, "stop_at_zero": True})
        self.assertEqual(counter.counts["deaths"], 0)
        counter.decrement({"name": "deaths"})
        self.assertEqual(counter.counts["deaths"], -1)
        counter.set_to({"name": "deaths", "value": 42})
        self.assertEqual(self.published()[-1], ("count:deaths", "42"))
        counter.reset({"name": "deaths"})
        self.assertEqual(self.published()[-1], ("count:deaths", "0"))

    def test_only_watched_counters_are_published(self):
        counter.increment({"name": "coffee"})
        self.assertEqual(self.published(), [])
        counter.plugin.watching = {"count:coffee", "count:new"}
        counter.watching(sorted(counter.plugin.watching))
        self.assertEqual(sorted(self.published()), [("count:coffee", "1"), ("count:new", "0")])

    def test_counts_are_kept(self):
        counter.increment({"name": "laps", "step": 3})
        with open(counter.STATE_FILE, encoding="utf-8") as f:
            self.assertEqual(json.load(f), {"counters": {"laps": 3}})
        counter.counts = {}
        counter.started()
        self.assertEqual(counter.counts, {"laps": 3})
        self.assertIn(("count:deaths", "0"), self.published())

    def test_errors(self):
        with self.assertRaisesRegex(ValueError, "name"):
            counter.increment({})
        with self.assertRaisesRegex(ValueError, "isn't a number"):
            counter.increment({"name": "a", "step": "lots"})
        with self.assertRaisesRegex(ValueError, "whole number"):
            counter.increment({"name": "a", "step": 1.5})
        with self.assertRaisesRegex(ValueError, "number to count from"):
            counter.set_to({"name": "a"})

    def test_text_file_for_obs(self):
        folder = os.path.join(self.dir.name, "obs")
        counter.increment({"name": "boss: wolf"})
        counter.plugin.settings = {"folder": folder}
        counter.configure(counter.plugin.settings)  # the folder was just set: write what we have
        with open(os.path.join(folder, "boss_ wolf.txt"), encoding="utf-8") as f:
            self.assertEqual(f.read(), "1")
        counter.increment({"name": "boss: wolf"})
        with open(os.path.join(folder, "boss_ wolf.txt"), encoding="utf-8") as f:
            self.assertEqual(f.read(), "2")

    def test_unwritable_folder_is_a_warning(self):
        blocker = os.path.join(self.dir.name, "file")
        open(blocker, "w").close()
        counter.plugin.settings = {"folder": blocker}
        self.assertIn("couldn't write", counter.increment({"name": "a"}))
        self.assertEqual(counter.counts["a"], 1)

    def test_file_names(self):
        self.assertEqual(counter.file_name('a/b\\c:d*?"<>|'), "a_b_c_d______.txt")
        self.assertEqual(counter.file_name(".."), "counter.txt")


if __name__ == "__main__":
    unittest.main()
