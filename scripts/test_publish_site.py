"""Tests for publish_site.py: publishing a small repository (a website/ and one plugin, built by
the real build_catalog.py) into a stand-in for a clone of the site repository, the guards, the
checks of the result, and the weekly check of the live site.

    cd scripts && python3 -m unittest test_publish_site
"""

import contextlib
import http.client
import io
import json
import os
import shutil
import socket
import ssl
import subprocess
import tempfile
import unittest
import zipfile
from unittest import mock

import publish_site as ps

ZIP = "plugins/dev.example.clock-1.0.0.zip"
WEBSITE = {
    "website/index.html": "<!doctype html><title>Home</title>\n",
    "website/404.html": "<!doctype html><title>Not found</title>\n",
    "website/style.css": "body { margin: 0; }\n",
    "website/guide/index.html": "<!doctype html><title>Guide</title>\n",
    "website/_headers": "/*\n  X-Robots-Tag: noindex\n",
}
PUBLISHED = [
    ".nojekyll", "404.html", "CNAME", "guide/index.html", "index.html",
    "plugins/dev.example.clock-1.0.0.zip", "plugins/index.html", "plugins/index.json", "style.css",
]


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)


def read(path):
    with open(path, encoding="utf-8", newline="") as f:
        return f.read()


def git(cwd, *args):
    return subprocess.run(["git", "-C", cwd, *args], check=True, capture_output=True, text=True).stdout


class Isolated(unittest.TestCase):
    """A temporary folder, and git as on a fresh runner: none of this computer's settings
    (commit signing, line endings, per-folder identities)."""

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.tmp = tmp.name
        config = os.path.join(self.tmp, "gitconfig")
        write(config, "")
        env = mock.patch.dict(os.environ, {
            "GIT_CONFIG_GLOBAL": config, "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_AUTHOR_NAME": "test", "GIT_AUTHOR_EMAIL": "test@example.invalid",
            "GIT_COMMITTER_NAME": "test", "GIT_COMMITTER_EMAIL": "test@example.invalid",
        })
        env.start()
        self.addCleanup(env.stop)

    def path(self, *parts):
        return os.path.join(self.tmp, *parts)


class Repo(Isolated):
    """The repository to publish from, with origin/main at HEAD, and an empty clone of the site."""

    def setUp(self):
        super().setUp()
        self.repo = self.path("repo")
        for rel, text in WEBSITE.items():
            write(os.path.join(self.repo, rel), text)
        write(os.path.join(self.repo, ".gitignore"), "website/plugins/\n.env\n")
        plugins = os.path.join(self.repo, "plugins")
        write(os.path.join(plugins, "catalogue_status.json"), '{"experimental": {}}\n')
        write(os.path.join(plugins, "clock", "perritodeck-plugin.json"),
              '{"id": "dev.example.clock", "name": "Clock", "version": "1.0.0", "runtime": "python"}\n')
        write(os.path.join(plugins, "clock", "plugin.py"), "print('tick')\n")
        for name in ("build_catalog.py", "LICENSE"):
            shutil.copy(os.path.join(ps.ROOT, "plugins", name), plugins)
        git(self.repo, "init", "-q", "-b", "main")
        self.commit("base")
        self.site = self.path("site")
        os.makedirs(self.site)
        git(self.site, "init", "-q", "-b", "main")
        git(self.site, "remote", "add", "origin", f"https://github.com/{ps.SITE_REPO}.git")

    def commit(self, message, origin=True):
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "-m", message)
        if origin:
            git(self.repo, "update-ref", "refs/remotes/origin/main", "HEAD")

    def publish(self, verify=False, site=None):
        """Runs a publish and returns what it printed."""
        with contextlib.redirect_stdout(io.StringIO()) as out:
            ps.publish(self.repo, site or self.site, verify)
        return out.getvalue()

    def refused(self, verify=False, site=None):
        with self.assertRaises(SystemExit) as e:
            self.publish(verify, site)
        return str(e.exception.code)

    def site_file(self, rel):
        return os.path.join(self.site, *rel.split("/"))


class Publish(Repo):
    def test_the_checkout_gets_the_committed_website_and_a_fresh_catalogue(self):
        said = self.publish()
        self.assertEqual(ps.files(self.site), PUBLISHED)
        self.assertEqual(read(self.site_file("index.html")), WEBSITE["website/index.html"])
        self.assertEqual(read(self.site_file("CNAME")).strip(), "perritodeck.dev")
        self.assertEqual(read(self.site_file(".nojekyll")), "")
        index = json.loads(read(self.site_file("plugins/index.json")))
        self.assertEqual([p["url"] for p in index["plugins"]], [os.path.basename(ZIP)])
        # The checkout's git status, and the commit message the workflow uses too.
        commit = git(self.repo, "rev-parse", "--short=7", "HEAD").strip()
        self.assertIn("?? plugins/index.json", said)
        self.assertIn(f"site: publish from perritodeck@{commit}", said)

    def test_only_what_is_committed_at_head_is_published(self):
        write(os.path.join(self.repo, "website", "index.html"), "edited, not committed\n")
        write(os.path.join(self.repo, "website", "draft.html"), "untracked\n")
        # git-ignored: an old version's zip from an earlier build, and a plugin's secrets.
        write(os.path.join(self.repo, "website", "plugins", "dev.example.clock-0.9.0.zip"), "old\n")
        write(os.path.join(self.repo, "plugins", "clock", ".env"), "TOKEN=secret\n")
        self.publish(verify=True)
        self.assertEqual(ps.files(self.site), PUBLISHED)
        self.assertEqual(read(self.site_file("index.html")), WEBSITE["website/index.html"])
        with zipfile.ZipFile(self.site_file(ZIP)) as z:
            self.assertEqual(sorted(z.namelist()), ["LICENSE", "perritodeck-plugin.json", "plugin.py"])

    def test_everything_else_in_the_checkout_goes_but_git_and_the_domain_stay(self):
        for rel in ("README.md", "old.html", "plugins/dev.example.clock-0.9.0.zip", "deep/er/x.txt", "_headers"):
            write(self.site_file(rel), "from an earlier publish\n")
        write(self.site_file("CNAME"), "perritodeck.dev")  # as GitHub writes it: no newline
        write(self.site_file(".git/kept"), "")
        self.publish()
        self.assertEqual(ps.files(self.site), PUBLISHED)
        self.assertFalse(os.path.exists(self.site_file("deep")))
        self.assertEqual(read(self.site_file("CNAME")), "perritodeck.dev")
        self.assertTrue(os.path.exists(self.site_file(".git/kept")))

    def test_a_cname_for_another_domain_is_put_right(self):
        write(self.site_file("CNAME"), "www.example.com\n")
        self.publish()
        self.assertEqual(read(self.site_file("CNAME")).strip(), "perritodeck.dev")

    def test_publishing_the_same_commit_again_changes_nothing(self):
        self.publish()
        git(self.site, "add", "-A")
        git(self.site, "commit", "-q", "-m", "first")
        self.assertIn("site: no change", self.publish())  # the zips are byte for byte the same

    def test_a_publish_refuses_a_working_tree_with_changes(self):
        for rel in ("website/index.html", "website/new.html"):  # changed, then untracked
            with self.subTest(rel):
                write(os.path.join(self.repo, *rel.split("/")), "not committed\n")
                self.assertIn("working tree has changes", self.refused())
                self.assertEqual(os.listdir(self.site), [".git"])
                git(self.repo, "checkout", "--", "website/index.html")

    def test_a_publish_refuses_a_head_that_isnt_origin_main(self):
        write(os.path.join(self.repo, "website", "new.html"), "<!doctype html>\n")
        self.commit("not pushed", origin=False)
        self.assertIn("isn't origin/main", self.refused())
        git(self.repo, "update-ref", "-d", "refs/remotes/origin/main")
        self.assertIn("isn't origin/main (not there)", self.refused())
        self.assertEqual(os.listdir(self.site), [".git"])

    def test_verify_builds_without_the_guards(self):
        write(os.path.join(self.repo, "website", "new.html"), "<!doctype html>\n")
        self.commit("not pushed", origin=False)
        write(os.path.join(self.repo, "website", "index.html"), "not committed\n")
        self.publish(verify=True)
        self.assertEqual(ps.files(self.site), sorted(PUBLISHED + ["new.html"]))

    def test_the_zips_carry_gits_modes(self):
        # build_catalog.py asks git, so the build has to happen in a git checkout; outside one
        # run.sh would be 0644. On Windows the disk would say every file is executable.
        script = os.path.join(self.repo, "plugins", "clock", "run.sh")
        write(script, "#!/bin/sh\n")
        os.chmod(script, 0o755)  # for git on Linux; Windows has no such bit, hence --chmod
        git(self.repo, "add", "plugins/clock/run.sh")
        git(self.repo, "update-index", "--chmod=+x", "plugins/clock/run.sh")
        self.commit("a script")
        self.publish()
        with zipfile.ZipFile(self.site_file(ZIP)) as z:
            modes = {i.filename: oct(i.external_attr >> 16) for i in z.infolist()}
        self.assertEqual(modes, {"perritodeck-plugin.json": "0o100644", "plugin.py": "0o100644",
                                 "run.sh": "0o100755", "LICENSE": "0o100644"})

    def test_a_publish_needs_a_clone_of_the_site_repository(self):
        git(self.site, "remote", "set-url", "origin", "https://github.com/perritodeck/perritodeck.git")
        self.assertIn("isn't a clone of", self.refused())
        plain = self.path("plain")
        os.makedirs(plain)
        self.assertIn("isn't a clone of", self.refused(site=plain))

    def test_verify_wont_empty_a_folder_that_isnt_the_site(self):
        folder = self.path("somewhere")
        write(os.path.join(folder, "precious.txt"), "keep me\n")
        self.assertIn("isn't the site", self.refused(verify=True, site=folder))
        self.assertEqual(os.listdir(folder), ["precious.txt"])
        # A new folder is fine, and so is running again into what that run left.
        stub = self.path("stub")
        for _ in range(2):
            self.assertIn("isn't a git checkout", self.publish(verify=True, site=stub))
        self.assertEqual(ps.files(stub), PUBLISHED)

    def test_a_publish_fails_when_the_result_would_serve_a_markdown_file(self):
        write(os.path.join(self.repo, "website", "README.md"), "# Notes\n")
        self.commit("a readme")
        self.assertIn("README.md would be served", self.refused())

    def test_a_committed_file_in_website_plugins_is_refused(self):
        write(os.path.join(self.repo, "website", "plugins", "extra.json"), "{}\n")
        git(self.repo, "add", "-f", "website/plugins/extra.json")
        git(self.repo, "commit", "-q", "-m", "extra")
        self.assertIn("where the catalogue is built", self.refused(verify=True))


class Check(Repo):
    """check() on copies of a published checkout and of the build it came from."""

    def setUp(self):
        super().setUp()
        self.publish()
        self.n = 0

    def check_after(self, change):
        self.n += 1
        site = shutil.copytree(self.site, self.path(f"site{self.n}"))
        build = shutil.copytree(self.site, self.path(f"build{self.n}"),
                                ignore=shutil.ignore_patterns(".git", *ps.KEPT))
        change(site, build)
        return ps.check(site, build)

    def test_a_fresh_publish_passes(self):
        self.assertEqual(self.check_after(lambda site, build: None), [])

    def test_a_checkout_that_isnt_the_build(self):
        cases = {
            "guide/index.html is missing": lambda site, build: os.remove(os.path.join(site, "guide", "index.html")),
            "old.html isn't part of the build": lambda site, build: write(os.path.join(site, "old.html"), ""),
            "style.css differs from the build": lambda site, build: write(os.path.join(site, "style.css"), "x"),
            "CNAME says 'example.com'": lambda site, build: write(os.path.join(site, "CNAME"), "example.com\n"),
            ".nojekyll is missing": lambda site, build: os.remove(os.path.join(site, ".nojekyll")),
        }
        for expected, change in cases.items():
            with self.subTest(expected):
                self.assertTrue(any(expected in p for p in self.check_after(change)), expected)

    def test_a_build_that_would_be_wrong_live(self):
        def both(rel, text=None):
            """Removes rel from the checkout and the build, or writes text there."""
            def change(site, build):
                for root in (site, build):
                    path = os.path.join(root, *rel.split("/"))
                    if text is None:
                        os.remove(path)
                    else:
                        write(path, text)
            return change

        index = json.loads(read(self.site_file("plugins/index.json")))
        cases = {
            "index.html is missing": both("index.html"),
            "404.html is missing": both("404.html"),
            "README.md would be served": both("README.md", "# Notes\n"),
            "guide/notes.md would be served": both("guide/notes.md", "notes\n"),
            "_headers would be served": both("_headers", "/*\n"),
            "plugins/index.json doesn't parse": both("plugins/index.json", "{"),
            "plugins/index.json lists no plugins": both("plugins/index.json", '{"version": 1, "plugins": []}'),
            f"{ZIP} doesn't match its SHA-256": both(ZIP, "not that zip"),
            "lists '../index.html', which isn't beside it": both(
                "plugins/index.json", json.dumps({**index, "plugins": [{**index["plugins"][0], "url": "../index.html"}]})
            ),
        }
        for expected, change in cases.items():
            with self.subTest(expected):
                self.assertTrue(any(expected in p for p in self.check_after(change)), expected)


GITHUB = ["185.199.108.153", "185.199.111.153", "2606:50c0:8000::153", "2606:50c0:8003::153"]
CLOUDFLARE = ["104.21.22.139", "172.67.205.29"]
NOW = 1_800_000_000
# The header only the Cloudflare Worker sends (from website/_headers); GitHub Pages can't.
WORKER = {"Content-Security-Policy": "default-src 'none'; frame-ancestors 'none'"}


def headers(fields):
    message = http.client.HTTPMessage()
    for name, value in fields.items():
        for one in value if isinstance(value, list) else [value]:
            message[name] = one
    return message


class CheckLive(unittest.TestCase):
    """check_live() with DNS and HTTPS faked. By default both names are GitHub Pages: the bare
    domain answers 200 and www 301 to it."""

    def run_check(self, addresses=GITHUB, www=None, status=None, servers=("GitHub.com",), extra=None,
                  location="https://perritodeck.dev/", days=60, error=None, host_error=None):
        self.fetched = []

        def resolve(host):
            found = addresses if host == ps.DOMAIN else www if www is not None else addresses
            if isinstance(found, Exception):
                raise found
            return found

        def fetch(host, path):
            self.fetched.append(f"https://{host}{path}")
            if error and host_error in (None, host):
                raise error
            apex = host == ps.DOMAIN
            fields = {"Server": list(servers), **(extra or {})}
            if not apex:
                fields["Location"] = location
            return status or (200 if apex else 301), headers(fields), NOW + days * ps.DAY

        with contextlib.redirect_stdout(io.StringIO()) as said:
            problems = ps.check_live(resolve=resolve, fetch=fetch, now=lambda: NOW)
        return problems, said.getvalue()

    def test_github_pages_passes(self):
        problems, said = self.run_check()
        self.assertEqual(problems, [])
        self.assertEqual(self.fetched, ["https://perritodeck.dev/", "https://perritodeck.dev/plugins/index.json",
                                        "https://www.perritodeck.dev/"])
        self.assertIn("perritodeck.dev and www.perritodeck.dev are GitHub Pages", said)

    def test_before_the_cutover_it_skips_itself(self):
        # Today: the Worker, behind Cloudflare's addresses, sends website/_headers' headers.
        problems, said = self.run_check(CLOUDFLARE, servers=["cloudflare"], extra=WORKER)
        self.assertEqual(problems, [])
        self.assertIn("Skipped: perritodeck.dev resolves to 104.21.22.139", said)

    def test_the_proxy_switched_back_on_fails(self):
        # evi-ai.dev's failure: the records proxied again, while the old certificate still works.
        # No Worker answers any more, whatever is still in the repository.
        problems, _ = self.run_check(CLOUDFLARE, servers=["cloudflare"])
        self.assertIn("set the apex and www records back to DNS only", problems[0])
        self.assertIn("https://perritodeck.dev/ came through Cloudflare (Server: cloudflare): set the apex and www "
                      "records to DNS only", problems)

    def test_only_www_proxied_fails(self):
        problems, _ = self.run_check(www=CLOUDFLARE)
        self.assertTrue(any(p.startswith("www.perritodeck.dev resolves to 104.21.22.139") for p in problems), problems)

    def test_once_it_points_at_github_the_worker_header_doesnt_skip_it(self):
        problems, _ = self.run_check(status=404, extra=WORKER)
        self.assertIn("https://perritodeck.dev/ answered 404, not 200", problems)

    def test_what_fails_it(self):
        cases = {
            "https://perritodeck.dev/ answered 503, not 200": {"status": 503},
            "https://www.perritodeck.dev/ answered 200, not 301": {"status": 200},
            "https://www.perritodeck.dev/ redirects to https://perritodeck.github.io/, not https://perritodeck.dev/":
                {"location": "https://perritodeck.github.io/"},
            "has Server: nginx, not GitHub.com": {"servers": ["nginx"]},
            "has Server: (none), not GitHub.com": {"servers": []},
            "expires on 2027-02-05, in 21 days": {"days": 21},
            "certificate has expired": {"error": ssl.SSLCertVerificationError("certificate has expired")},
            "https://www.perritodeck.dev/: timed out": {"error": TimeoutError("timed out"), "host_error": ps.WWW},
            "perritodeck.dev resolves to 192.0.2.1, not only to GitHub Pages": {"addresses": GITHUB + ["192.0.2.1"]},
            "perritodeck.dev doesn't resolve": {"addresses": socket.gaierror("Name or service not known")},
            "www.perritodeck.dev doesn't resolve": {"www": socket.gaierror("Name or service not known")},
            # Cloudflare's addresses and an error page: nothing shows the Worker still serves the site.
            "https://perritodeck.dev/ answered 526, not 200": {"addresses": CLOUDFLARE, "status": 526},
        }
        for expected, case in cases.items():
            with self.subTest(expected):
                problems, _ = self.run_check(**case)
                self.assertTrue(any(expected in p for p in problems), problems)
        self.assertEqual(self.run_check(days=22)[0], [])


class Main(unittest.TestCase):
    def test_arguments(self):
        with mock.patch.object(ps, "publish") as publish:
            self.assertEqual(ps.main(["../site", "--verify"]), 0)
            publish.assert_called_with(ps.ROOT, "../site", True)
            self.assertEqual(ps.main(["../site"]), 0)
            publish.assert_called_with(ps.ROOT, "../site", False)
        for argv in ([], ["--verify"], ["a", "b"], ["--live"]):
            with self.subTest(argv), self.assertRaises(SystemExit):
                ps.main(argv)

    def test_check_live_fails_the_run_on_a_problem(self):
        with contextlib.redirect_stdout(io.StringIO()):
            with mock.patch.object(ps, "check_live", return_value=["broken"]):
                self.assertEqual(ps.main(["--check-live"]), 1)
            with mock.patch.object(ps, "check_live", return_value=[]):
                self.assertEqual(ps.main(["--check-live"]), 0)


if __name__ == "__main__":
    unittest.main()
