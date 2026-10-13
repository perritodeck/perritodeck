"""Tests for build_catalog.py: plugin zips, plugin icons and the catalogue page, plus the
website's Content-Security-Policy, which the catalogue page shares with every other page."""

import glob
import os
import re
import tempfile
import unittest
import zipfile
from html.parser import HTMLParser

import build_catalog as bc

WEBSITE = os.path.normpath(os.path.join(bc.HERE, "..", "website"))
ENTRY = {
    "id": "dev.example.x", "name": "Example", "version": "1.2.3", "description": "Does things",
    "author": "", "url": "dev.example.x-1.2.3.zip", "sha256": "0" * 64, "runtime": "python",
    "actions": ["Do it"], "values": ["Count"], "size": 2048,
}


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)


class Zips(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.folder = os.path.join(self.tmp.name, "plugin")
        write(os.path.join(self.folder, "perritodeck-plugin.json"), '{"id": "dev.example.x"}')
        write(os.path.join(self.folder, "plugin.py"), "print('hi')\n")
        write(os.path.join(self.folder, "img", "icon.svg"), "<svg/>")
        write(os.path.join(self.folder, "test_plugin.py"), "")
        write(os.path.join(self.folder, "__pycache__", "plugin.cpython-312.pyc"), "")

    def tearDown(self):
        self.tmp.cleanup()

    def build(self, name="a.zip"):
        dest = os.path.join(self.tmp.name, name)
        bc.build_zip(self.folder, dest)
        return dest

    def test_a_zip_holds_the_plugin_and_the_mit_licence_but_not_tests_or_caches(self):
        with zipfile.ZipFile(self.build()) as z:
            self.assertEqual(z.namelist(), ["perritodeck-plugin.json", "plugin.py", "img/icon.svg", "LICENSE"])
            with open(bc.LICENSE, "rb") as f:
                self.assertEqual(z.read("LICENSE"), f.read())
            self.assertIn(b"MIT License", z.read("LICENSE"))
            self.assertEqual({i.date_time for i in z.infolist()}, {bc.FIXED_TIME})

    def test_a_plugins_own_licence_is_kept_instead(self):
        for own in ("LICENSE", "LICENSE.md", "licence.txt", "COPYING", "COPYING.txt", "LICENSE-MIT", "License-Apache.md"):
            with self.subTest(own):
                path = os.path.join(self.folder, own)
                write(path, "Its own terms\n")
                try:
                    with zipfile.ZipFile(self.build(own + ".zip")) as z:
                        names = z.namelist()
                        self.assertEqual(names.count(own), 1)
                        self.assertEqual(z.read(own), b"Its own terms\n")
                        if own != "LICENSE":
                            self.assertNotIn("LICENSE", names)
                finally:
                    os.remove(path)

    def test_a_file_that_only_starts_like_a_licence_isnt_one(self):
        for name in ("license_check.py", "copying.py", "licenses.json", "licence.py", "COPYING-helper.js"):
            with self.subTest(name):
                path = os.path.join(self.folder, name)
                write(path, "# not a licence\n")
                try:
                    with zipfile.ZipFile(self.build(name + ".zip")) as z:
                        self.assertIn(name, z.namelist())
                        self.assertEqual(z.namelist()[-1], "LICENSE")
                        self.assertIn(b"MIT License", z.read("LICENSE"))
                finally:
                    os.remove(path)

    def test_a_licence_in_a_subfolder_doesnt_count(self):
        write(os.path.join(self.folder, "vendor", "LICENSE"), "A dependency's terms\n")
        with zipfile.ZipFile(self.build()) as z:
            self.assertIn("vendor/LICENSE", z.namelist())
            self.assertIn("LICENSE", z.namelist())

    def test_zips_are_reproducible(self):
        first = self.build("first.zip")
        os.utime(os.path.join(self.folder, "plugin.py"), (1_000_000_000, 1_000_000_000))
        second = self.build("second.zip")
        with open(first, "rb") as a, open(second, "rb") as b:
            self.assertEqual(a.read(), b.read())

    def test_every_real_plugin_zip_says_it_is_mit_licensed(self):
        # None of the first-party plugins has a licence file of its own.
        for manifest in glob.glob(os.path.join(bc.HERE, "*", "perritodeck-plugin.json")):
            folder = os.path.dirname(manifest)
            with self.subTest(os.path.basename(folder)):
                self.assertEqual(bc.zip_entries(folder)[-1], ("LICENSE", bc.LICENSE))


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


class Inline(HTMLParser):
    """What a page has that `script-src 'self'; style-src 'self'` would block, and the files
    it loads (scripts, stylesheets, icons and images)."""

    def __init__(self, text):
        super().__init__()
        self.blocked, self.loads = [], []
        self.feed(text)
        self.close()

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag == "script" and not a.get("src"):
            self.blocked.append("an inline <script>")
        if tag == "style":
            self.blocked.append("a <style> block")
        for name, value in attrs:
            if name == "style":
                self.blocked.append(f"a style attribute on <{tag}>")
            elif name.startswith("on"):
                self.blocked.append(f"an {name} attribute on <{tag}>")
            elif name in ("href", "src") and (value or "").strip().lower().startswith("javascript:"):
                self.blocked.append(f"a javascript: link on <{tag}>")
        if tag == "script" and a.get("src"):
            self.loads.append(a["src"])
        elif tag == "link" and a.get("href"):
            self.loads.append(a["href"])
        elif tag in ("img", "source"):
            self.loads += [a["src"]] if a.get("src") else []
            self.loads += [s.split()[0] for s in (a.get("srcset") or "").split(",") if s.strip()]


def missing_files(page_dir, loads):
    """The loads that aren't files in website/, or that come from another site."""
    bad = []
    for url in loads:
        if re.match(r"^[a-z]+:|^//", url, re.I):
            bad.append(url)
            continue
        path = url.split("#")[0].split("?")[0]
        local = os.path.join(WEBSITE, path.lstrip("/")) if path.startswith("/") else os.path.join(page_dir, path)
        if not os.path.isfile(os.path.normpath(local)):
            bad.append(url)
    return bad


def css_loads(text):
    """The files a stylesheet loads: every url(...) (fonts, images, `@import url(...)`) and
    every `@import "..."`. Comments are left out."""
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    urls = re.findall(r"url\(\s*['\"]?([^'\")]+)", text) + re.findall(r"@import\s+['\"]([^'\"]+)", text)
    return [url.strip() for url in urls]


class Page(unittest.TestCase):
    def test_entries_show_their_icon_or_initials(self):
        html = bc.page([dict(ENTRY, icon="dev.example.x-1.2.3.svg"), dict(ENTRY, id="dev.example.y", name="Why Not")])
        self.assertIn('<img class="icon" src="dev.example.x-1.2.3.svg"', html)
        self.assertIn('aria-hidden="true">WN</span>', html)

    def test_the_search_box_matches_everything_an_entry_says(self):
        html = bc.page([ENTRY])
        self.assertIn('data-words="example does things do it count"', html)
        self.assertIn('<div class="finder" hidden>', html)

    def test_the_page_has_no_inline_code_and_loads_only_the_websites_own_files(self):
        page = Inline(bc.page([ENTRY]))
        self.assertEqual(page.blocked, [])
        for url in ("../style.css", "../catalogue.css", "../catalogue.js"):
            self.assertIn(url, page.loads)
        # It's published as website/plugins/index.html.
        self.assertEqual(missing_files(os.path.join(WEBSITE, "plugins"), page.loads), [])


class Website(unittest.TestCase):
    """The site's Content-Security-Policy (website/_headers) allows only the site's own files,
    with no inline code. Anything inline would silently stop working once deployed."""

    def pages(self):
        found = glob.glob(os.path.join(WEBSITE, "*.html")) + glob.glob(os.path.join(WEBSITE, "guide", "*.html"))
        self.assertGreater(len(found), 10)
        for path in sorted(found):
            with open(path, encoding="utf-8") as f:
                yield path, f.read()

    def test_no_page_has_inline_code_or_loads_from_another_site(self):
        for path, text in self.pages():
            with self.subTest(os.path.relpath(path, WEBSITE)):
                page = Inline(text)
                self.assertEqual(page.blocked, [])
                self.assertEqual(missing_files(os.path.dirname(path), page.loads), [])

    def test_stylesheets_load_only_the_websites_own_files(self):
        # font-src and img-src are 'self' too: a data: URI, another site or a missing file
        # in a url() or @import would break once deployed, with nothing on the page to say so.
        found = glob.glob(os.path.join(WEBSITE, "**", "*.css"), recursive=True)
        self.assertGreaterEqual(len(found), 4)
        for path in sorted(found):
            with open(path, encoding="utf-8") as f:
                text = f.read()
            with self.subTest(os.path.relpath(path, WEBSITE)):
                self.assertEqual(missing_files(os.path.dirname(path), css_loads(text)), [])

    def test_the_stylesheet_check_finds_what_the_policy_blocks(self):
        css = """
          /* url(commented-out.png) doesn't count */
          .a { background: url(data:image/svg+xml,%3Csvg%3E); }
          .b { background: url("https://example.com/b.png"); }
          @font-face { src: url('//fonts.example.com/f.woff2') format("woff2"); }
          .c { background: url( missing.png ); }
          .d { background: url("fonts/LICENSE-atkinson-hyperlegible.txt"); }
          @import "http://example.com/e.css";
          @import url(missing.css);
        """
        self.assertEqual(
            missing_files(WEBSITE, css_loads(css)),
            [
                "data:image/svg+xml,%3Csvg%3E",
                "https://example.com/b.png",
                "//fonts.example.com/f.woff2",
                "missing.png",
                "missing.css",
                "http://example.com/e.css",
            ],
        )

    def test_the_not_found_page_works_at_any_depth(self):
        with open(os.path.join(WEBSITE, "404.html"), encoding="utf-8") as f:
            text = f.read()
        self.assertIn('<meta name="robots" content="noindex">', text)
        for url in re.findall(r'(?:href|src)="([^"]*)"', text):
            with self.subTest(url):
                self.assertTrue(url.startswith(("/", "https://")), url)

    def test_there_is_one_strict_policy(self):
        with open(os.path.join(WEBSITE, "_headers"), encoding="utf-8") as f:
            policies = [line.split(":", 1)[1].strip() for line in f if line.strip().startswith("Content-Security-Policy:")]
        self.assertEqual(len(policies), 1, "two CSP headers on one path are two policies that must both pass")
        policy = policies[0]
        self.assertIn("default-src 'none'", policy)
        self.assertIn("script-src 'self';", policy)
        self.assertIn("style-src 'self';", policy)
        for loose in ("unsafe-", "sha256-", "nonce-", "*", "http:", "https:", "data:"):
            self.assertNotIn(loose, policy)


if __name__ == "__main__":
    unittest.main()
