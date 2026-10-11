#!/usr/bin/env python3
"""Show, check or set the release version everywhere it lives.

    python3 scripts/release-version.py                 # list every version; fails if they differ
    python3 scripts/release-version.py --check 0.2.0   # what the Release workflow runs on a tag
    python3 scripts/release-version.py 0.2.0           # bump everything (does not commit or tag)

Setting a version also increments the Android versionCode and the iOS build number, and
updates the Cargo.lock entries of PerritoDeck's own crates so `cargo build --locked`
still passes. Plugins have their own versions (each plugin's perritodeck-plugin.json)
and are not touched. Standard library only; runs on Windows, macOS and Linux.
"""

import json
import os
import re
import sys

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
SEMVER = re.compile(r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$")


def path(rel):
    return os.path.join(ROOT, *rel.split("/"))


def read(rel):
    with open(path(rel), encoding="utf-8", newline="") as f:
        return f.read()


def write(rel, text):
    with open(path(rel), "w", encoding="utf-8", newline="") as f:
        f.write(text)


# (file, regex whose group 1 is the version). Each pattern must match exactly once.
VERSION_SPOTS = [
    ("host/Cargo.toml", r'(?ms)^\[workspace\.package\]\s*^version = "([^"]+)"'),
    ("host/app/src-tauri/Cargo.toml", r'(?ms)^\[package\][^\[]*?^version = "([^"]+)"'),
    ("host/app/src-tauri/tauri.conf.json", r'(?m)^  "version": "([^"]+)"'),
    ("host/app/package.json", r'(?m)^  "version": "([^"]+)"'),
    ("android/app/build.gradle.kts", r'(?m)^\s*versionName = "([^"]+)"'),
    ("ios/project.yml", r'(?m)^\s*MARKETING_VERSION: "([^"]+)"'),
]
LOCKFILES = ["host/Cargo.lock", "host/app/src-tauri/Cargo.lock"]
ANDROID_CODE = ("android/app/build.gradle.kts", r"(?m)^(\s*versionCode = )(\d+)")
IOS_BUILD = ("ios/project.yml", r'(?m)^(\s*CURRENT_PROJECT_VERSION: ")(\d+)(")')
DOWNLOAD_PAGE = "website/download.html"


def find_one(rel, pattern):
    matches = list(re.finditer(pattern, read(rel)))
    if len(matches) != 1:
        sys.exit(f"{rel}: expected one match for {pattern!r}, found {len(matches)}")
    return matches[0]


def lock_versions(rel):
    """Versions of PerritoDeck's own crates (no `source`) in a Cargo.lock."""
    out = {}
    for block in read(rel).split("[[package]]")[1:]:
        name = re.search(r'(?m)^name = "([^"]+)"', block)
        version = re.search(r'(?m)^version = "([^"]+)"', block)
        if name and version and name.group(1).startswith("perritodeck-") and "\nsource = " not in block:
            out[name.group(1)] = version.group(1)
    return out


def package_lock_versions():
    data = json.loads(read("host/app/package-lock.json"))
    return [data.get("version"), data.get("packages", {}).get("", {}).get("version")]


def current():
    found = {rel: find_one(rel, pattern).group(1) for rel, pattern in VERSION_SPOTS}
    for i, v in enumerate(package_lock_versions()):
        found[f"host/app/package-lock.json [{i}]"] = v
    for rel in LOCKFILES:
        for name, v in lock_versions(rel).items():
            found[f"{rel} ({name})"] = v
    return found


def android_code():
    return int(find_one(*ANDROID_CODE).group(2))


def changelog(code):
    return f"fastlane/metadata/android/en-US/changelogs/{code}.txt"


def show(expected=None):
    found = current()
    width = max(len(k) for k in found)
    for k, v in found.items():
        mark = "" if expected is None or v == expected else "   <-- expected " + expected
        print(f"{k.ljust(width)}  {v}{mark}")
    code = android_code()
    print(f"{'android versionCode'.ljust(width)}  {code}")
    problems = []
    values = set(found.values())
    if expected is not None and values != {expected}:
        problems.append(f"not every version is {expected}")
    elif len(values) != 1:
        problems.append("versions differ: " + ", ".join(sorted(values)))
    note = changelog(code)
    if not os.path.isfile(path(note)):
        problems.append(f"missing {note} (the F-Droid/Play changelog for versionCode {code})")
    elif len(read(note).strip()) > 500:
        problems.append(f"{note} is longer than 500 characters")
    for p in problems:
        print(f"error: {p}", file=sys.stderr)
    return not problems


def sub_one(rel, pattern, repl):
    text = read(rel)
    new, n = re.subn(pattern, repl, text, count=1)
    if n != 1:
        sys.exit(f"{rel}: pattern {pattern!r} not found")
    write(rel, new)


def set_version(new):
    old = find_one(*VERSION_SPOTS[0]).group(1)
    for rel, pattern in VERSION_SPOTS:
        m = find_one(rel, pattern)
        text = read(rel)
        write(rel, text[: m.start(1)] + new + text[m.end(1) :])

    # package-lock.json: the root "version" and packages[""].version, edited as text to keep formatting.
    rel = "host/app/package-lock.json"
    text = read(rel)
    text, n1 = re.subn(r'(?m)^(  "version": ")[^"]+(")', rf"\g<1>{new}\g<2>", text, count=1)
    text, n2 = re.subn(r'(?ms)(^    "": \{\s*\n\s*"name": "[^"]+",\s*\n\s*"version": ")[^"]+(")', rf"\g<1>{new}\g<2>", text, count=1)
    if (n1, n2) != (1, 1):
        sys.exit(f"{rel}: couldn't find both version fields")
    write(rel, text)

    for rel in LOCKFILES:
        blocks = read(rel).split("[[package]]")
        for i, block in enumerate(blocks[1:], 1):
            name = re.search(r'(?m)^name = "([^"]+)"', block)
            if name and name.group(1).startswith("perritodeck-") and "\nsource = " not in block:
                blocks[i] = re.sub(r'(?m)^version = "[^"]+"', f'version = "{new}"', block, count=1)
        write(rel, "[[package]]".join(blocks))

    code = android_code() + 1
    sub_one(ANDROID_CODE[0], ANDROID_CODE[1], rf"\g<1>{code}")
    build = int(find_one(*IOS_BUILD).group(2)) + 1
    sub_one(IOS_BUILD[0], IOS_BUILD[1], rf"\g<1>{build}\g<3>")

    if os.path.isfile(path(DOWNLOAD_PAGE)):
        text = read(DOWNLOAD_PAGE)
        text = text.replace(f"/download/v{old}/", f"/download/v{new}/")
        text = text.replace(f"perritodeck-{old}-", f"perritodeck-{new}-")
        text = text.replace(f"perritodeck-host-{old}-", f"perritodeck-host-{new}-")
        text = text.replace(f"perrito-{old}.apk", f"perrito-{new}.apk")
        text = text.replace(f'data-version="{old}"', f'data-version="{new}"')
        text = text.replace(f"Version {old}", f"Version {new}")
        write(DOWNLOAD_PAGE, text)

    print(f"Version {old} -> {new}; Android versionCode {code}; iOS build {build}.")
    note = changelog(code)
    if not os.path.isfile(path(note)):
        print(f"Now write {note} (500 characters at most), then commit.")


def main(argv):
    if not argv:
        return 0 if show() else 1
    if argv[0] == "--check":
        if len(argv) != 2:
            sys.exit(__doc__)
        return 0 if show(argv[1].removeprefix("v")) else 1
    if len(argv) != 1 or argv[0].startswith("-"):
        sys.exit(__doc__)
    new = argv[0].removeprefix("v")
    if not SEMVER.match(new):
        sys.exit(f"{new!r} isn't a version like 1.2.3 or 1.2.3-rc.1")
    set_version(new)
    show(new)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
