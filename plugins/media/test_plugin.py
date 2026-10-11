import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as media  # noqa: E402


class MediaTest(unittest.TestCase):
    def test_describe(self):
        self.assertEqual(media.describe("Artist", "Song"), "Artist – Song")
        self.assertEqual(media.describe("", "Song"), "Song")
        self.assertEqual(media.describe("", ""), media.NOTHING)

    def test_missing_playerctl_is_explained(self):
        saved, media.PLAYERCTL = media.PLAYERCTL, None
        try:
            with self.assertRaisesRegex(RuntimeError, "playerctl"):
                media.play_pause({})
        finally:
            media.PLAYERCTL = saved


if __name__ == "__main__":
    unittest.main()
