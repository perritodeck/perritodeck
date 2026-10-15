"""Tests for build_catalog.py: plugin zips, plugin icons, Experimental marks and the
catalogue page, plus the website's Content-Security-Policy, which the catalogue page shares
with every other page."""

import contextlib
import glob
import io
import json
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
EXPERIMENTAL = dict(
    ENTRY, id="dev.example.e", name="Edgy", experimental=True, tested="Tested against a stand-in & nothing else."
)


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


def manifests():
    """{plugin id: plugin folder} for every first-party plugin."""
    found = {}
    for path in glob.glob(os.path.join(bc.HERE, "*", "perritodeck-plugin.json")):
        if os.path.basename(os.path.dirname(path)) in bc.NOT_PLUGINS:
            continue
        with open(path, encoding="utf-8") as f:
            found[json.load(f)["id"]] = os.path.dirname(path)
    return found


# How a plugin's README says it has only met stand-ins: "Not yet tried against a real GitLab
# instance", or a "## Not tested yet" section that starts "Against a real Hue Bridge".
UNTRIED = re.compile(
    r"not yet tried against (a|the) (real|live)\b|^## not tested yet\s+against (a |the )?(real|live)\b", re.I | re.M
)


class Experimental(unittest.TestCase):
    """Plugins tested only against stand-ins are marked Experimental in index.json and on the
    page, from catalogue_status.json, without touching their manifests or zips."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.src = os.path.join(self.tmp.name, "plugins")
        for plugin_id in ("dev.example.a", "dev.example.b"):
            manifest = {"id": plugin_id, "name": plugin_id[-1].upper(), "version": "1.0.0", "runtime": "python"}
            write(os.path.join(self.src, plugin_id, "perritodeck-plugin.json"), json.dumps(manifest))
            write(os.path.join(self.src, plugin_id, "plugin.py"), "print('hi')\n")

    def tearDown(self):
        self.tmp.cleanup()

    def status(self, listed):
        path = os.path.join(self.tmp.name, "status.json")
        write(path, json.dumps({"about": ["…"], "experimental": listed}))
        return bc.load_status(path)

    def build(self, out, experimental):
        out = os.path.join(self.tmp.name, out)
        with contextlib.redirect_stdout(io.StringIO()):
            entries = bc.build(self.src, out, experimental)
        return out, entries

    def test_listed_plugins_are_marked_in_the_index_and_on_the_page_and_their_zips_dont_change(self):
        notes = self.status({"dev.example.b": {"tested": " Tested against\n a stand-in. ", "source": "its README"}})
        self.assertEqual(notes, {"dev.example.b": "Tested against a stand-in."})
        _, plain = self.build("plain", {})
        out, marked = self.build("marked", notes)
        with open(os.path.join(out, "index.json"), encoding="utf-8") as f:
            index = {p["id"]: p for p in json.load(f)["plugins"]}
        self.assertNotIn("experimental", index["dev.example.a"])
        self.assertNotIn("tested", index["dev.example.a"])
        self.assertIs(index["dev.example.b"]["experimental"], True)
        self.assertEqual(index["dev.example.b"]["tested"], "Tested against a stand-in.")
        self.assertNotIn("source", index["dev.example.b"])
        # The same zips with the same hashes: only index.json and the page say Experimental.
        self.assertEqual([(p["url"], p["sha256"]) for p in plain], [(p["url"], p["sha256"]) for p in marked])
        with open(os.path.join(out, "index.html"), encoding="utf-8") as f:
            page = f.read()
        self.assertIn('<span class="tag">Experimental</span></h2>', page)
        self.assertIn('<p class="tested">Tested against a stand-in.</p>', page)

    def test_a_listed_plugin_that_isnt_in_the_catalogue_stops_the_build(self):
        with self.assertRaises(SystemExit) as stopped:
            self.build("out", {"dev.example.gone": "Tested against a stand-in."})
        self.assertIn("dev.example.gone", str(stopped.exception.code))
        # Before anything is written, zips included.
        self.assertFalse(os.path.exists(os.path.join(self.tmp.name, "out")))

    def test_every_listed_plugin_needs_a_tested_note(self):
        for bad in ({}, {"tested": ""}, {"tested": "  "}, {"tested": 3}, "Tested against a stand-in."):
            with self.subTest(bad), self.assertRaises(SystemExit):
                self.status({"dev.example.a": bad})

    def test_the_list_maps_plugin_ids_to_notes(self):
        for bad in (["dev.example.a"], "dev.example.a", None):
            with self.subTest(bad), self.assertRaises(SystemExit):
                self.status(bad)
        path = os.path.join(self.tmp.name, "status.json")
        write(path, '["dev.example.a"]')
        with self.assertRaises(SystemExit):
            bc.load_status(path)

    def test_a_plugin_listed_twice_stops_the_build(self):
        # json.load alone would keep the second entry and quietly drop the first.
        path = os.path.join(self.tmp.name, "status.json")
        write(path, '{"experimental": {"dev.example.a": {"tested": "One."}, "dev.example.a": {"tested": "Two."}}}')
        with self.assertRaises(SystemExit) as stopped:
            bc.load_status(path)
        self.assertIn("dev.example.a", str(stopped.exception.code))

    def test_the_real_list_names_real_plugins_with_a_short_note_and_where_it_comes_from(self):
        with open(bc.STATUS, encoding="utf-8") as f:
            listed = json.load(f)["experimental"]
        self.assertTrue(listed)
        plugins = manifests()
        for plugin_id, about in listed.items():
            with self.subTest(plugin_id):
                self.assertIn(plugin_id, plugins)
                self.assertTrue(about.get("source"), "say where the plugin's test status is written down")
                # It starts with the file in the repository that says so.
                where = about["source"].split()[0].rstrip(",")
                self.assertTrue(os.path.isfile(os.path.join(bc.HERE, "..", *where.split("/"))), f"{where} isn't in the repo")
                self.assertLessEqual(len(about["tested"]), 160, "one short sentence")
                self.assertTrue(about["tested"].endswith("."))
        self.assertEqual(sorted(bc.load_status()), sorted(listed))

    def test_a_readme_saying_the_plugin_hasnt_met_the_real_thing_means_experimental(self):
        # The catalogue mustn't present a plugin as finished when its own README says it has
        # only been tested against stand-ins.
        listed = bc.load_status()
        for plugin_id, folder in sorted(manifests().items()):
            readme = os.path.join(folder, "README.md")
            if not os.path.isfile(readme):
                continue
            with open(readme, encoding="utf-8") as f:
                untried = UNTRIED.search(f.read())
            if untried:
                with self.subTest(plugin_id, readme_says=untried.group(0)):
                    self.assertIn(plugin_id, listed)

    def test_the_readme_wording_check_finds_both_ways_of_saying_it(self):
        for text in (
            "Tested with a fake server. Not yet tried against a real GitLab instance.",
            "**Not yet tried against the real Spotify service.**",
            "## Not tested yet\n\nAgainst a real Hue Bridge: the tests use a fake bridge.",
            "## Not tested yet\r\n\r\nAgainst the live Open-Meteo service.",
            "## Not tested yet\n\nAgainst real lights: the tests use fake lights.",
        ):
            with self.subTest(text):
                self.assertTrue(UNTRIED.search(text))
        for text in (
            "## Not tested yet\n\nOBS reading the text file.",
            "Tried against a real OBS 31.",
        ):
            with self.subTest(text):
                self.assertFalse(UNTRIED.search(text))


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
        elif tag == "link" and a.get("href") and (a.get("rel") or "").lower() != "canonical":
            # A canonical link names the page's own public address; the browser loads nothing.
            # <meta> tags (Open Graph and the rest) load nothing either.
            self.loads.append(a["href"])
        elif tag in ("img", "source"):
            self.loads += [a["src"]] if a.get("src") else []
            self.loads += [s.split()[0] for s in (a.get("srcset") or "").split(",") if s.strip()]


class Head(HTMLParser):
    """A page's title, canonical link and <meta> tags (by name or property), unescaped."""

    def __init__(self, text):
        super().__init__()
        self.title, self.canonical, self.meta, self._in_title = "", None, {}, False
        self.feed(text)
        self.close()

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag == "title":
            self._in_title = True
        elif tag == "link" and (a.get("rel") or "").lower() == "canonical":
            self.canonical = a.get("href")
        elif tag == "meta" and (a.get("property") or a.get("name")):
            self.meta[a.get("property") or a.get("name")] = a.get("content")

    def handle_endtag(self, tag):
        if tag == "title":
            self._in_title = False

    def handle_data(self, data):
        if self._in_title:
            self.title += data


def png_size(path):
    with open(path, "rb") as f:
        head = f.read(24)
    assert head[:8] == b"\x89PNG\r\n\x1a\n" and head[12:16] == b"IHDR", f"{path} isn't a PNG"
    return int.from_bytes(head[16:20], "big"), int.from_bytes(head[20:24], "big")


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

    def test_experimental_entries_have_a_tag_and_say_what_they_were_tested_against(self):
        html = bc.page([ENTRY, EXPERIMENTAL])
        self.assertIn('<h2>Edgy <span class="version">1.2.3</span> <span class="tag">Experimental</span></h2>', html)
        self.assertIn('<h2>Example <span class="version">1.2.3</span></h2>', html)
        self.assertIn('<p class="tested">Tested against a stand-in &amp; nothing else.</p>', html)
        self.assertEqual(html.count('class="tested"'), 1)
        # Searching for "experimental" finds them.
        self.assertIn('data-words="edgy does things do it count experimental"', html)
        # One line under the introduction says what the tag means.
        self.assertIn('<p class="note"><span class="tag">Experimental</span> plugins have only been tested against', html)
        self.assertEqual(html.count('<span class="tag">Experimental</span>'), 2)

    def test_without_experimental_plugins_nothing_says_experimental(self):
        html = bc.page([ENTRY])
        self.assertNotIn('class="tag"', html)
        self.assertNotIn('class="tested"', html)
        self.assertNotIn('class="note"', html)

    def test_the_tag_uses_a_style_the_site_has(self):
        # The page has no styles of its own (the CSP), so the tag needs the site's .tag rule.
        with open(os.path.join(WEBSITE, "style.css"), encoding="utf-8") as f:
            self.assertRegex(f.read(), r"(?m)^\.tag\s*\{")

    def test_the_page_has_its_address_and_the_sites_link_preview(self):
        # What Website checks on every other page, at the address the catalogue is published on.
        head = Head(bc.page([ENTRY]))
        self.assertEqual(head.canonical, "https://perritodeck.dev/plugins/")
        self.assertEqual(head.meta["og:url"], head.canonical)
        self.assertEqual(head.meta["og:title"], head.title)
        self.assertEqual(head.meta["og:description"], head.meta["description"])
        with open(os.path.join(WEBSITE, "index.html"), encoding="utf-8") as f:
            home = Head(f.read())
        for shared in ("og:type", "og:site_name", "og:image", "og:image:width", "og:image:height", "og:image:alt", "twitter:card"):
            with self.subTest(shared):
                self.assertIsNotNone(home.meta.get(shared))
                self.assertEqual(head.meta.get(shared), home.meta[shared])

    def test_the_page_has_no_inline_code_and_loads_only_the_websites_own_files(self):
        page = Inline(bc.page([ENTRY, EXPERIMENTAL]))
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

    def test_every_page_has_its_address_and_a_link_preview(self):
        # The canonical link, og:url and sitemap.xml all use the address Pages serves a page
        # on: https://perritodeck.dev, no ".html", folders ending in "/". The preview image is
        # an absolute URL too, so it's checked here rather than as something the page loads.
        site = "https://perritodeck.dev/"
        with open(os.path.join(WEBSITE, "sitemap.xml"), encoding="utf-8") as f:
            sitemap = re.findall(r"<loc>([^<]+)</loc>", f.read())
        expected = {site + "plugins/"}  # the generated catalogue page
        for path, text in self.pages():
            rel = os.path.relpath(path, WEBSITE).replace(os.sep, "/")
            head = Head(text)
            with self.subTest(rel):
                if rel == "404.html":  # served at any address, so it has none of its own
                    self.assertIsNone(head.canonical)
                    continue
                url = site + re.sub(r"\.html$", "", re.sub(r"(^|/)index\.html$", r"\1", rel))
                expected.add(url)
                self.assertEqual(head.canonical, url)
                self.assertEqual(head.meta.get("og:url"), url)
                self.assertEqual(head.meta.get("og:title"), head.title)
                self.assertEqual(head.meta.get("og:description"), head.meta.get("description"))
                self.assertEqual(head.meta.get("og:type"), "website")
                self.assertEqual(head.meta.get("og:site_name"), "PerritoDeck")
                self.assertEqual(head.meta.get("twitter:card"), "summary_large_image")
                self.assertTrue(head.meta.get("og:image:alt"))
                image = head.meta.get("og:image") or ""
                self.assertTrue(image.startswith(site + "img/"), image)
                local = os.path.join(WEBSITE, *image[len(site):].split("/"))
                self.assertTrue(os.path.isfile(local), image)
                self.assertEqual(
                    (head.meta.get("og:image:width"), head.meta.get("og:image:height")),
                    tuple(str(n) for n in png_size(local)),
                )
        self.assertEqual(sorted(sitemap), sorted(expected), "sitemap.xml should list every page once")
        self.assertEqual(len(sitemap), len(set(sitemap)))

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

    def test_the_folder_holds_only_files_the_site_serves(self):
        # Cloudflare publishes every file under website/, so a README, notes or a script there
        # would be live at https://perritodeck.dev/<name>. They go in docs/website.md and scripts/.
        served = {".html", ".css", ".js", ".png", ".webp", ".ico", ".svg", ".woff2"}
        named = {"_headers", "_redirects", "robots.txt", "sitemap.xml"}

        def unpublishable(root):
            for folder, subfolders, files in os.walk(root):
                if folder == root and "plugins" in subfolders:
                    subfolders.remove("plugins")  # build_catalog.py's output, git-ignored
                for name in files:
                    rel = os.path.relpath(os.path.join(folder, name), root).replace(os.sep, "/")
                    if name in (".DS_Store", "Thumbs.db"):  # git-ignored, so never published
                        continue
                    if name in named or os.path.splitext(name)[1].lower() in served:
                        continue
                    if re.fullmatch(r"fonts/LICENSE-[\w-]+\.txt", rel):  # the OFL travels with the fonts
                        continue
                    yield rel

        self.assertEqual(sorted(unpublishable(WEBSITE)), [], "this would be published; see docs/website.md")
        with tempfile.TemporaryDirectory() as tmp:
            for rel in ("index.html", "img/og.png", "fonts/LICENSE-x.txt", "_headers", "plugins/index.json",
                        "README.md", "notes.txt", "img/og-image.py", "guide/LICENSE-x.txt"):
                write(os.path.join(tmp, *rel.split("/")), "")
            self.assertEqual(
                sorted(unpublishable(tmp)), ["README.md", "guide/LICENSE-x.txt", "img/og-image.py", "notes.txt"]
            )


if __name__ == "__main__":
    unittest.main()
