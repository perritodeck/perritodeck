"""Tests for build_catalog.py: plugin icons and the catalogue page."""

import os
import tempfile
import unittest

import build_catalog as bc

ENTRY = {
    "id": "dev.example.x", "name": "Example", "version": "1.2.3", "description": "Does things",
    "author": "", "url": "dev.example.x-1.2.3.zip", "sha256": "0" * 64, "runtime": "python",
    "actions": ["Do it"], "values": ["Count"], "size": 2048,
}


class Icons(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.folder = os.path.join(self.tmp.name, "plugin")
        self.out = os.path.join(self.tmp.name, "out")
        os.makedirs(os.path.join(self.folder, "img"))
        os.makedirs(self.out)
        with open(os.path.join(self.folder, "img", "icon.svg"), "w", encoding="utf-8") as f:
            f.write("<svg xmlns='http://www.w3.org/2000/svg'/>")

    def tearDown(self):
        self.tmp.cleanup()

    def test_an_icon_is_copied_next_to_the_zip(self):
        name = bc.copy_icon(self.folder, {"id": "dev.example.x", "version": "1.2.3", "icon": "img/icon.svg"}, self.out)
        self.assertEqual(name, "dev.example.x-1.2.3.svg")
        self.assertTrue(os.path.isfile(os.path.join(self.out, name)))

    def test_no_icon_is_fine(self):
        self.assertIsNone(bc.copy_icon(self.folder, {"id": "a", "version": "1"}, self.out))

    def test_icons_outside_the_folder_or_of_other_types_are_refused(self):
        for bad in ("../outside.svg", "img/missing.svg", "img/icon.gif"):
            with self.subTest(bad), self.assertRaises(SystemExit):
                bc.copy_icon(self.folder, {"id": "a", "version": "1", "icon": bad}, self.out)


class Page(unittest.TestCase):
    def test_entries_show_their_icon_or_initials(self):
        html = bc.page([dict(ENTRY, icon="dev.example.x-1.2.3.svg"), dict(ENTRY, id="dev.example.y", name="Why Not")])
        self.assertIn('<img class="icon" src="dev.example.x-1.2.3.svg"', html)
        self.assertIn('aria-hidden="true">WN</span>', html)

    def test_the_search_box_matches_everything_an_entry_says(self):
        html = bc.page([ENTRY])
        self.assertIn('data-words="example does things do it count"', html)
        self.assertIn('<div class="finder" hidden>', html)

    def test_the_page_makes_no_third_party_requests(self):
        html = bc.page([ENTRY])
        self.assertNotIn("fonts.googleapis", html)
        self.assertNotIn("<script src=", html)


if __name__ == "__main__":
    unittest.main()
