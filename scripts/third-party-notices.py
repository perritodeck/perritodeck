#!/usr/bin/env python3
"""Build the third-party licence inventory that ships with PerritoDeck releases.

    python3 scripts/third-party-notices.py                 # rewrite THIRD_PARTY_NOTICES.md
    python3 scripts/third-party-notices.py --check         # fail if it is out of date
    python3 scripts/third-party-notices.py --full out.txt  # also write every licence text

Sources:
- Rust: `cargo metadata --locked` for host/ and host/app/src-tauri/ (normal and build
  dependencies of the workspace members, every target platform, dev-dependencies left
  out). Needs the crates downloaded (`cargo fetch`) only for --full.
- npm: host/app/package-lock.json production dependencies, plus what Vite bundles into
  the app from dev dependencies (the Svelte runtime and the generated Tabler icons).
- Android, fonts, icons and vendored code: the hand-kept tables in this file. Update
  ANDROID when android/gradle/libs.versions.toml changes.

The --full file collects LICENSE/COPYING/NOTICE files, de-duplicated, for the release
bundles (MIT, BSD and Apache-2.0 NOTICE terms need the actual texts to travel with the
binaries). Standard library only.
"""

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "THIRD_PARTY_NOTICES.md"
RUST_WORKSPACES = [
    ("Command-line host and shared crates", ROOT / "host" / "Cargo.toml"),
    ("Desktop app (Tauri)", ROOT / "host" / "app" / "src-tauri" / "Cargo.toml"),
]
NPM_LOCK = ROOT / "host" / "app" / "package-lock.json"
# Dev dependencies whose code Vite copies into the app bundle.
NPM_BUNDLED_FROM_DEV = ["svelte", "clsx", "esm-env", "@tabler/icons", "vite"]

# Declared in android/app/build.gradle.kts via gradle/libs.versions.toml. Gradle can't be
# resolved here, so transitive libraries the declared ones are known to pull in are
# listed too. All are Apache-2.0; none needs Google Play Services.
ANDROID = [
    ("androidx.core:core-ktx", "1.16.0", "Apache-2.0"),
    ("androidx.activity:activity-compose", "1.10.1", "Apache-2.0"),
    ("androidx.lifecycle:lifecycle-viewmodel-compose", "2.9.1", "Apache-2.0"),
    ("androidx.lifecycle:lifecycle-runtime-compose", "2.9.1", "Apache-2.0"),
    ("androidx.compose:compose-bom (ui, foundation, material3, ui-tooling-preview)", "2025.06.01", "Apache-2.0"),
    ("androidx.camera:camera-camera2, camera-lifecycle, camera-view", "1.4.2", "Apache-2.0"),
    ("org.jetbrains.kotlinx:kotlinx-coroutines-android", "1.10.2", "Apache-2.0"),
    ("org.jetbrains.kotlinx:kotlinx-serialization-json", "1.9.0", "Apache-2.0"),
    ("com.squareup.okhttp3:okhttp", "4.12.0", "Apache-2.0"),
    ("com.google.zxing:core", "3.5.3", "Apache-2.0"),
    ("Transitive: org.jetbrains.kotlin:kotlin-stdlib", "2.2.0", "Apache-2.0"),
    ("Transitive: com.squareup.okio:okio", "3.x", "Apache-2.0"),
    ("Transitive: other androidx.* (annotation, collection, savedstate, ...)", "-", "Apache-2.0"),
    ("Transitive: com.google.guava:listenablefuture (CameraX)", "1.0", "Apache-2.0"),
    ("Transitive: org.jspecify:jspecify, org.jetbrains:annotations", "-", "Apache-2.0"),
]
ANDROID_TEST_ONLY = "JUnit 4.13.2 (EPL-1.0), kotlinx-coroutines-test, OkHttp MockWebServer, androidx.test, compose ui-test (Apache-2.0)"

OTHER = [
    ("Inter (font)", "host/crates/render/assets/Inter-SemiBold.otf",
     "OFL-1.1", "Copyright (c) 2016-2023 The Inter Project Authors. Embedded in the host to draw key titles. Licence: host/crates/render/assets/OFL.txt"),
    ("Bricolage Grotesque (font)", "@fontsource-variable/bricolage-grotesque",
     "OFL-1.1", "Copyright 2022 The Bricolage Grotesque Project Authors. Bundled in the desktop app. The website serves its own copy (website/fonts, licence alongside)."),
    ("Atkinson Hyperlegible (font)", "@fontsource/atkinson-hyperlegible",
     "OFL-1.1", "Copyright 2020 Braille Institute of America, Inc. Bundled in the desktop app. The website serves its own copy (website/fonts, licence alongside)."),
    ("Tabler Icons", "host/app/src/lib/icons.generated.ts",
     "MIT", "Copyright (c) 2020-2026 Pawel Kuna. A subset is generated into the app by host/app/scripts/build-icons.mjs."),
    ("enigo 0.6.1 (vendored, modified)", "host/vendor/enigo",
     "MIT", "Upstream authors are listed in host/vendor/enigo/Cargo.toml; licence in host/vendor/enigo/LICENSE; changes described in host/vendor/enigo/PERRITODECK.md."),
]

LICENSE_FILE = re.compile(r"^(licen[cs]e|copying|notice|copyright|unlicense)", re.I)


def cargo_metadata(manifest):
    env = dict(os.environ)
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--manifest-path", str(manifest)],
        check=True, capture_output=True, text=True, encoding="utf-8", env=env,
    ).stdout
    return json.loads(out)


def shipped_rust_packages(meta):
    """Packages reachable from workspace members through normal or build edges."""
    pkgs = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    seen, stack = set(), list(meta["workspace_members"])
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes[pid]["deps"]:
            kinds = {k["kind"] for k in dep["dep_kinds"]}
            if kinds & {None, "build"}:
                stack.append(dep["pkg"])
    members = set(meta["workspace_members"])
    return [pkgs[i] for i in seen if i not in members]


def rust_license(p):
    if p.get("license"):
        return p["license"].replace("/", " OR ")
    if p.get("license_file"):
        return "see " + p["license_file"]
    return "UNKNOWN"


def npm_packages():
    lock = json.loads(NPM_LOCK.read_text(encoding="utf-8"))["packages"]
    rows = {}
    for path, p in lock.items():
        if not path.startswith("node_modules/"):
            continue
        name = path[len("node_modules/"):]
        shipped = not p.get("dev") and not p.get("devOptional")
        if shipped or name in NPM_BUNDLED_FROM_DEV:
            note = "dependency" if shipped else "bundled by Vite"
            rows[name] = (p.get("version", "?"), p.get("license", "UNKNOWN"), note)
    return rows


def md_table(header, rows):
    lines = ["| " + " | ".join(header) + " |", "|" + "---|" * len(header)]
    lines += ["| " + " | ".join(str(c) for c in r) + " |" for r in rows]
    return "\n".join(lines)


def build(metas):
    parts = [
        "# Third-party notices",
        "",
        "<!-- Generated by scripts/third-party-notices.py. Do not edit by hand. -->",
        "",
        "PerritoDeck is licensed under Apache-2.0 (see [LICENSE](LICENSE)); the plugin SDKs",
        "and first-party plugins in `plugins/` are MIT (see [plugins/LICENSE](plugins/LICENSE)).",
        "It is built on the open-source work listed below. Release bundles carry the full",
        "licence texts (`third-party-licenses.txt`, from `--full`).",
        "",
        "No dependency is GPL, AGPL or LGPL-only. Copyleft-adjacent items, all used",
        "unmodified unless noted:",
        "",
        "- **MPL-2.0** (file-level): `option-ext` (host, via `directories`); `cssparser`,",
        "  `cssparser-macros`, `dtoa-short`, `selectors` (desktop app, via Tauri). Their",
        "  source is available from crates.io.",
        "- **OFL-1.1**: the Inter, Bricolage Grotesque and Atkinson Hyperlegible fonts.",
        "- **`r-efi`** is `MIT OR Apache-2.0 OR LGPL-2.1-or-later`; PerritoDeck uses it under",
        "  MIT/Apache-2.0.",
        "",
        "## Fonts, icons and vendored code",
        "",
        md_table(["Component", "Where", "Licence", "Notes"], OTHER),
        "",
    ]
    for title, meta in metas:
        pkgs = sorted(shipped_rust_packages(meta), key=lambda p: (p["name"], p["version"]))
        rows = []
        for p in pkgs:
            if p["name"].startswith("perritodeck-"):
                continue
            src = "vendored" if p.get("source") is None else ""
            rows.append((p["name"], p["version"] + (" (" + src + ")" if src else ""), rust_license(p)))
        parts += [
            "## Rust crates: " + title,
            "",
            f"{len(rows)} crates (all target platforms; build-time crates included, test-only crates left out).",
            "",
            "<details><summary>Show the list</summary>",
            "",
            md_table(["Crate", "Version", "Licence"], rows),
            "",
            "</details>",
            "",
        ]
    npm = npm_packages()
    parts += [
        "## JavaScript in the desktop app",
        "",
        md_table(["Package", "Version", "Licence", "How it ships"],
                 [(n, *npm[n]) for n in sorted(npm)]),
        "",
        "## Android app (Perrito)",
        "",
        md_table(["Library", "Version", "Licence"], ANDROID),
        "",
        "Test-only, not shipped: " + ANDROID_TEST_ONLY + ".",
        "",
        "## iOS app and plugins",
        "",
        "The iOS app uses only Apple frameworks and the in-repo `PerritoKit` package. The",
        "Python and Node.js plugin SDKs and the first-party plugins use only their language's",
        "standard library.",
        "",
    ]
    return "\n".join(parts)


def license_texts(metas):
    """Map text hash -> (text, set of package labels)."""
    texts = {}

    def add(label, folder):
        folder = Path(folder)
        if not folder.is_dir():
            return
        for f in sorted(folder.iterdir()):
            if f.is_file() and LICENSE_FILE.match(f.name):
                try:
                    t = f.read_text(encoding="utf-8", errors="replace").strip()
                except OSError:
                    continue
                key = hashlib.sha256(re.sub(r"\s+", " ", t).encode()).hexdigest()
                texts.setdefault(key, [t, set()])[1].add(label)

    for _, meta in metas:
        for p in shipped_rust_packages(meta):
            if not p["name"].startswith("perritodeck-"):
                add(f'{p["name"]} {p["version"]}', Path(p["manifest_path"]).parent)
    nm = NPM_LOCK.parent / "node_modules"
    for name, (ver, _, _) in npm_packages().items():
        add(f"{name} {ver} (npm)", nm / name)
    add("Inter font", ROOT / "host" / "crates" / "render" / "assets")
    return texts


def write_full(metas, path):
    out = [
        "PerritoDeck third-party licence texts",
        "Generated by scripts/third-party-notices.py --full.",
        "Android libraries are Apache-2.0; the Apache-2.0 text below applies to them.",
        "",
    ]
    texts = license_texts(metas)
    covered = set().union(*(labels for _, labels in texts.values())) if texts else set()
    missing = []
    for _, meta in metas:
        for p in shipped_rust_packages(meta):
            label = f'{p["name"]} {p["version"]}'
            if not p["name"].startswith("perritodeck-") and label not in covered:
                missing.append(f"{label}: {rust_license(p)}")
    for t, labels in sorted(texts.values(), key=lambda x: sorted(x[1])[0]):
        out += ["=" * 78, "Used by: " + ", ".join(sorted(labels)), "-" * 78, t, ""]
    if missing:
        out += [
            "=" * 78,
            "Crates published without a licence file. The standard texts of their",
            "licences apply (MIT and Apache-2.0 texts appear above; the Apache-2.0 text",
            "is also PerritoDeck's own LICENSE):",
            "-" * 78,
            *sorted(set(missing)),
            "",
        ]
    out += ["=" * 78, "Apache License 2.0 (Android libraries, and the crates above that name it)",
            "-" * 78, (ROOT / "LICENSE").read_text(encoding="utf-8").strip(), ""]
    Path(path).write_text("\n".join(out), encoding="utf-8")
    print(f"wrote {path} ({len(texts)} distinct texts, {len(set(missing))} crates without a licence file)")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--check", action="store_true", help="fail if THIRD_PARTY_NOTICES.md is stale")
    ap.add_argument("--full", metavar="FILE", help="also write all licence texts to FILE")
    args = ap.parse_args()

    metas = [(title, cargo_metadata(m)) for title, m in RUST_WORKSPACES]
    text = build(metas)
    if args.check:
        current = OUT.read_text(encoding="utf-8") if OUT.exists() else ""
        if current != text:
            print("THIRD_PARTY_NOTICES.md is out of date: run python3 scripts/third-party-notices.py", file=sys.stderr)
            return 1
        print("THIRD_PARTY_NOTICES.md is up to date")
    else:
        OUT.write_text(text, encoding="utf-8", newline="\n")
        print(f"wrote {OUT.relative_to(ROOT)}")
    if args.full:
        write_full(metas, args.full)
    return 0


if __name__ == "__main__":
    sys.exit(main())
