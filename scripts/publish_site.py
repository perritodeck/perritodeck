#!/usr/bin/env python3
"""Publish website/ to perritodeck.dev, which GitHub Pages serves from the public repository
perritodeck/perritodeck.github.io (Deploy from a branch: main, /).

    python3 scripts/publish_site.py <site-checkout>            # into a clone of the site repository
    python3 scripts/publish_site.py <site-checkout> --verify   # the same, no guards (pull requests)
    python3 scripts/publish_site.py --check-live               # is perritodeck.dev still GitHub Pages?

The site repository holds generated output only: website/ here is the one source. A publish
builds from a fresh checkout of HEAD, so no untracked or git-ignored file goes out (not even
one in a plugin's folder, which build_catalog.py would zip), and build_catalog.py can take each
file's mode from git. The catalogue goes into a new folder, so no old version's zip is left
over. Then the site checkout is made to hold exactly website/ plus the catalogue as plugins/,
where the app's Look online downloads it. .git stays, and so do CNAME (perritodeck.dev) and
.nojekyll, which are created if missing. Everything else in the site checkout is deleted, and
no README is written. Last, it checks the result: the pages are there, index.json parses and
every file it lists is beside it with its SHA-256, and no _headers, _redirects or .md file
would be served.

It doesn't commit or push. It prints the site checkout's `git status`, so whoever runs it by
hand sees the change before committing it. On main, .github/workflows/site-publish.yml commits
and pushes. Until the repository is public Actions don't run, so publish by hand from WSL.

A publish refuses to run on a working tree with changes, on a HEAD that isn't origin/main (git
fetch first), and into anything but a clone of the site repository. --verify skips these
guards, but it still won't empty a folder that isn't the site.

--check-live is the weekly check (.github/workflows/site-check.yml) for what took evi-ai.dev
down: its DNS records were switched to proxied, GitHub couldn't renew its certificate, and once
that expired Cloudflare answered 526 for weeks. It fails unless perritodeck.dev and its www
resolve only to GitHub Pages, / and /plugins/index.json answer 200 and www 301 to the bare
domain, all with Server: GitHub.com, and the certificate has more than 21 days left.

Standard library only.
"""

import filecmp
import hashlib
import http.client
import ipaddress
import json
import os
import re
import shutil
import socket
import ssl
import subprocess
import sys
import tempfile
import time

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
DOMAIN = "perritodeck.dev"
SITE_REPO = "perritodeck/perritodeck.github.io"
SITE_ORIGIN = re.compile(r"[/:]perritodeck/perritodeck\.github\.io(\.git)?/?$", re.I)
# What GitHub Pages needs beside the build: the custom domain, and no Jekyll (which leaves out
# files whose names start with "_" or ".").
KEPT = ("CNAME", ".nojekyll")
# Cloudflare's configuration files. GitHub Pages doesn't read them; it would serve them as files,
# so a publish that would serve one fails.
UNPUBLISHED = {"_headers", "_redirects"}
# What the site can't do without. The app's Look online reads plugins/index.json.
REQUIRED = ("index.html", "404.html", "plugins/index.html", "plugins/index.json")

# GitHub Pages' addresses (docs.github.com, "Managing a custom domain for your GitHub Pages site").
GITHUB_PAGES = (ipaddress.ip_network("185.199.108.0/22"), ipaddress.ip_network("2606:50c0:8000::/46"))
WWW = "www." + DOMAIN
# What --check-live asks for, and the status each should answer: www redirects to the bare domain.
LIVE = ((DOMAIN, "/", 200), (DOMAIN, "/plugins/index.json", 200), (WWW, "/", 301))
# GitHub's Let's Encrypt certificates last 90 days and are renewed weeks before they run out, so
# one with three weeks left isn't being renewed. Lower this if certificates get shorter.
CERT_DAYS = 21
DAY = 86400


def git(cwd, *args):
    """git's output, as bytes; git's own message ends the run if it fails."""
    r = subprocess.run(["git", "-C", cwd, *args], capture_output=True)
    if r.returncode:
        sys.exit(f"git {' '.join(args)} in {cwd}: {r.stderr.decode(errors='replace').strip()}")
    return r.stdout


def files(root):
    """Every file under root as a path with "/", sorted, leaving out a .git at the top."""
    found = []
    for folder, dirs, names in os.walk(root):
        if folder == root:
            dirs[:] = [d for d in dirs if d != ".git"]
            names = [n for n in names if n != ".git"]
        found += [os.path.relpath(os.path.join(folder, n), root).replace(os.sep, "/") for n in names]
    return sorted(found)


def read_text(folder, name):
    try:
        with open(os.path.join(folder, name), encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def copy(src, dest):
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    shutil.copyfile(src, dest)


def annotate(level, text):
    """Prints text, as a GitHub Actions annotation ("warning" or "error") when running there."""
    print(f"::{level}::{text}" if os.environ.get("GITHUB_ACTIONS") == "true" else text)


def is_site_clone(site):
    if not os.path.exists(os.path.join(site, ".git")):
        return False
    r = subprocess.run(["git", "-C", site, "remote", "get-url", "origin"], capture_output=True, text=True)
    return r.returncode == 0 and bool(SITE_ORIGIN.search(r.stdout.strip()))


def refuse_unsafe_site(site, verify):
    """Everything in the checkout that isn't part of the build is deleted, so make sure it's the
    site: a clone of the site repository, or for --verify a new or empty folder, or one that an
    earlier run filled (it has the site's CNAME)."""
    if is_site_clone(site):
        return
    if not verify:
        sys.exit(f"{site} isn't a clone of https://github.com/{SITE_REPO}")
    if os.path.exists(site) and not os.path.isdir(site):
        sys.exit(f"{site} isn't a folder")
    if os.path.isdir(site) and os.listdir(site) and read_text(site, "CNAME").strip() != DOMAIN:
        sys.exit(f"{site} isn't empty and has no CNAME for {DOMAIN}, so it isn't the site; nothing in it "
                 "was touched")


def refuse_unless_publishable(repo):
    if git(repo, "status", "--porcelain").strip():
        sys.exit("The working tree has changes: commit or stash them first (--verify builds anyway)")
    head = git(repo, "rev-parse", "HEAD").decode().strip()
    main = git(repo, "for-each-ref", "--format=%(objectname)", "refs/remotes/origin/main").decode().strip()
    if head != main:
        sys.exit(f"HEAD ({head[:7]}) isn't origin/main ({main[:7] or 'not there'}): git fetch, then check out "
                 "origin/main")


def checkout(repo, dest):
    """A fresh checkout of repo's HEAD at dest: what's committed and nothing else. It's a git
    checkout, because build_catalog.py asks git for each file's mode (the disk can't say on
    Windows); outside one every file would be 0644. --shared borrows repo's objects, which
    works from a shallow clone too, and writes nothing into repo."""
    head = git(repo, "rev-parse", "HEAD").decode().strip()
    git(repo, "clone", "--quiet", "--shared", "--no-checkout", ".", os.path.abspath(dest))
    git(dest, "checkout", "--quiet", "--detach", head)


def build(src, out):
    """The site as published, in the new folder `out`: src/website, and the catalogue built
    from src/plugins as out/plugins."""
    website = os.path.join(src, "website")
    os.makedirs(out)
    for rel in files(website):
        if rel.split("/")[0] == "plugins":
            sys.exit(f"website/{rel} is committed, but website/plugins/ is where the catalogue is built")
        copy(os.path.join(website, *rel.split("/")), os.path.join(out, *rel.split("/")))
    builder = os.path.join(src, "plugins", "build_catalog.py")
    r = subprocess.run([sys.executable, builder, os.path.join(out, "plugins")], capture_output=True)
    sys.stdout.write(r.stdout.decode(errors="replace"))
    if r.returncode:
        sys.exit(r.stderr.decode(errors="replace").strip() or "build_catalog.py failed")


def mirror(out, site):
    """Makes `site` hold exactly the files in `out`, plus CNAME and .nojekyll. .git isn't touched."""
    wanted = files(out)
    keep = {*wanted, *KEPT}
    for rel in files(site):
        if rel not in keep:
            os.remove(os.path.join(site, *rel.split("/")))
    folders = []
    for folder, dirs, _ in os.walk(site):
        if folder == site:
            dirs[:] = [d for d in dirs if d != ".git"]
        folders += [os.path.join(folder, d) for d in dirs]
    for folder in reversed(folders):  # the deepest first
        if not os.listdir(folder):
            os.rmdir(folder)
    for rel in wanted:
        copy(os.path.join(out, *rel.split("/")), os.path.join(site, *rel.split("/")))
    if read_text(site, "CNAME").strip() != DOMAIN:
        with open(os.path.join(site, "CNAME"), "w", encoding="utf-8", newline="\n") as f:
            f.write(DOMAIN + "\n")
    if not os.path.exists(os.path.join(site, ".nojekyll")):
        open(os.path.join(site, ".nojekyll"), "w").close()


def catalogue_problems(site):
    """index.json parses, lists plugins, and every file it names is beside it with its SHA-256."""
    folder = os.path.join(site, "plugins")
    try:
        with open(os.path.join(folder, "index.json"), encoding="utf-8") as f:
            index = json.load(f)
    except FileNotFoundError:
        return []  # reported as missing
    except (OSError, ValueError) as e:
        return [f"plugins/index.json doesn't parse: {e}"]
    entries = index.get("plugins") if isinstance(index, dict) else None
    if not isinstance(entries, list) or not entries:
        return ["plugins/index.json lists no plugins"]
    problems = []
    for entry in entries:
        url = entry.get("url") if isinstance(entry, dict) else None
        path = os.path.join(folder, url) if isinstance(url, str) else ""
        if not url or url != os.path.basename(url) or not os.path.isfile(path):
            problems.append(f"plugins/index.json lists {url!r}, which isn't beside it")
            continue
        with open(path, "rb") as f:
            if hashlib.sha256(f.read()).hexdigest() != entry.get("sha256"):
                problems.append(f"plugins/{url} doesn't match its SHA-256 in index.json")
    return problems


def check(site, out):
    """What's wrong with `site` as the published copy of the build in `out`; [] if nothing."""
    built = files(out)
    present = set(files(site))
    problems = [f"{rel} is missing" for rel in sorted({*built, *KEPT, *REQUIRED}) if rel not in present]
    problems += [f"{rel} isn't part of the build" for rel in sorted(present - {*built, *KEPT})]
    problems += [
        f"{rel} differs from the build"
        for rel in built
        if rel in present
        and not filecmp.cmp(os.path.join(out, *rel.split("/")), os.path.join(site, *rel.split("/")), shallow=False)
    ]
    cname = read_text(site, "CNAME").strip()
    if "CNAME" in present and cname != DOMAIN:
        problems.append(f"CNAME says {cname!r}, not {DOMAIN}")
    problems += [
        f"{rel} would be served at https://{DOMAIN}/{rel}"
        for rel in sorted(present)
        if rel.split("/")[-1] in UNPUBLISHED or rel.lower().endswith(".md")
    ]
    return problems + catalogue_problems(site)


def report(site, commit, verify):
    """The checkout's git status, and for a publish how to send it."""
    if not os.path.exists(os.path.join(site, ".git")):
        print(f"{site} isn't a git checkout, so there's nothing to commit")
        return
    status = git(site, "status", "--short", "--untracked-files=all").decode()
    if not status.strip():
        print("site: no change")
        return
    print(f"git status in {site}: {len(status.splitlines())} change(s)")
    print(status, end="")
    if not verify:
        print("Look it over, then commit and push it:")
        print(f"  git -C {site} add -A")
        print(f'  git -C {site} commit -s -m "site: publish from perritodeck@{commit}"')
        print(f"  git -C {site} push")


def publish(repo, site, verify=False):
    """Builds the site from `repo`'s HEAD into the checkout `site`, and checks it."""
    refuse_unsafe_site(site, verify)
    if not verify:
        refuse_unless_publishable(repo)
    commit = git(repo, "rev-parse", "--short=7", "HEAD").decode().strip()
    with tempfile.TemporaryDirectory() as tmp:
        src, out = os.path.join(tmp, "src"), os.path.join(tmp, "out")
        checkout(repo, src)
        build(src, out)
        os.makedirs(site, exist_ok=True)
        mirror(out, site)
        problems = check(site, out)
        count = len(files(out))
    if problems:
        sys.exit("The site isn't right:\n" + "\n".join(f"  {p}" for p in problems))
    print(f"{site}: {count} files built from perritodeck@{commit}, and checked")
    report(site, commit, verify)


def resolve(host):
    return sorted({info[4][0] for info in socket.getaddrinfo(host, 443, type=socket.SOCK_STREAM)})


def fetch(host, path):
    """(status, headers, when the certificate expires) for https://host/path, not following a
    redirect."""
    conn = http.client.HTTPSConnection(host, timeout=30, context=ssl.create_default_context())
    try:
        conn.connect()
        expires = ssl.cert_time_to_seconds(conn.sock.getpeercert()["notAfter"])
        conn.request("GET", path, headers={"User-Agent": "PerritoDeck site check"})
        response = conn.getresponse()
        response.read()
        return response.status, response.headers, expires
    finally:
        conn.close()


def check_live(resolve=resolve, fetch=fetch, now=time.time):
    """What's wrong with perritodeck.dev and www as served; [] if nothing."""
    problems, elsewhere = [], {}
    for host in (DOMAIN, WWW):
        try:
            found = resolve(host)
        except OSError as e:
            problems.append(f"{host} doesn't resolve: {e}")
            continue
        elsewhere[host] = [a for a in found if not any(ipaddress.ip_address(a) in net for net in GITHUB_PAGES)]
    answers = {}
    for host, path, _ in LIVE:
        try:
            answers[host, path] = fetch(host, path)
        except (OSError, http.client.HTTPException) as e:
            answers[host, path] = e
    for host, found in elsewhere.items():
        if found:
            problems.append(
                f"{host} resolves to {', '.join(found)}, not only to GitHub Pages. If that's Cloudflare, its "
                "proxy is on (an orange cloud): set the apex and www records back to DNS only, or GitHub can't "
                "renew its certificate and the site goes down when this one expires."
            )
    expiry = []
    for host, path, expected in LIVE:
        url = f"https://{host}{path}"
        answer = answers[host, path]
        if isinstance(answer, Exception):
            problems.append(f"{url}: {answer}")
            continue
        status, headers, expires = answer
        servers = headers.get_all("Server") or []
        expiry.append(expires)
        if status != expected:
            problems.append(f"{url} answered {status}, not {expected}")
        elif status == 301 and headers.get("Location") != f"https://{DOMAIN}{path}":
            problems.append(f"{url} redirects to {headers.get('Location')}, not https://{DOMAIN}{path}")
        if any("cloudflare" in s.lower() for s in servers):
            problems.append(f"{url} came through Cloudflare (Server: cloudflare): set the apex and www records to DNS only")
        elif servers != ["GitHub.com"]:
            problems.append(f"{url} has Server: {', '.join(servers) or '(none)'}, not GitHub.com")
    if expiry:
        days = (min(expiry) - now()) / DAY
        until = time.strftime("%Y-%m-%d", time.gmtime(min(expiry)))
        if days <= CERT_DAYS:
            problems.append(
                f"The certificate for {DOMAIN} expires on {until}, in {days:.0f} days, so GitHub isn't renewing "
                f"it: see Settings > Pages in {SITE_REPO}"
            )
        elif not problems:
            print(f"{DOMAIN} and {WWW} are GitHub Pages, and the certificate is good until {until}")
    return problems


def main(argv):
    if argv == ["--check-live"]:
        problems = check_live()
        for problem in problems:
            annotate("error", problem)
        return 1 if problems else 0
    verify = "--verify" in argv
    rest = [a for a in argv if a != "--verify"]
    if len(rest) != 1 or rest[0].startswith("-"):
        sys.exit(__doc__)
    publish(ROOT, rest[0], verify)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
