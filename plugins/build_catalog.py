#!/usr/bin/env python3
"""Build the online plugin catalogue: one zip per plugin plus index.json.

    python3 plugins/build_catalog.py [OUT_DIR]

OUT_DIR defaults to website/plugins/. Publish that folder at
https://perritodeck.dev/plugins/ (the app reads index.json there). Zips are
reproducible: same files in, same bytes (and SHA-256) out. Tests and caches are
left out. Standard library only.
"""

import hashlib
import json
import os
import sys
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
SKIP_DIRS = {"__pycache__", "node_modules", ".git"}
# Plugin folders that aren't plugins themselves.
NOT_PLUGINS = {"sdk-python", "sdk-node"}
FIXED_TIME = (2026, 1, 1, 0, 0, 0)


def plugin_files(folder):
    for root, dirs, files in os.walk(folder):
        dirs[:] = sorted(d for d in dirs if d not in SKIP_DIRS)
        for name in sorted(files):
            if name.startswith("test_") or name.endswith((".pyc", ".pyo")):
                continue
            yield os.path.join(root, name)


def build_zip(folder, dest):
    with zipfile.ZipFile(dest, "w", zipfile.ZIP_DEFLATED) as z:
        for path in plugin_files(folder):
            rel = os.path.relpath(path, folder).replace(os.sep, "/")
            info = zipfile.ZipInfo(rel, FIXED_TIME)
            info.compress_type = zipfile.ZIP_DEFLATED
            executable = os.access(path, os.X_OK)
            info.external_attr = (0o100755 if executable else 0o100644) << 16
            with open(path, "rb") as f:
                z.writestr(info, f.read())


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "..", "website", "plugins")
    os.makedirs(out, exist_ok=True)
    entries = []
    for name in sorted(os.listdir(HERE)):
        folder = os.path.join(HERE, name)
        manifest_path = os.path.join(folder, "perritodeck-plugin.json")
        if name in NOT_PLUGINS or not os.path.isfile(manifest_path):
            continue
        with open(manifest_path, encoding="utf-8") as f:
            m = json.load(f)
        zip_name = f"{m['id']}-{m['version']}.zip"
        build_zip(folder, os.path.join(out, zip_name))
        with open(os.path.join(out, zip_name), "rb") as f:
            digest = hashlib.sha256(f.read()).hexdigest()
        entries.append(
            {
                "id": m["id"],
                "name": m["name"],
                "version": m["version"],
                "description": m.get("description", ""),
                "author": m.get("author", ""),
                "url": zip_name,
                "sha256": digest,
            }
        )
        print(f"{zip_name}  {digest}")
    with open(os.path.join(out, "index.json"), "w", encoding="utf-8") as f:
        json.dump({"version": 1, "plugins": entries}, f, indent=2, ensure_ascii=False)
        f.write("\n")
    print(f"{len(entries)} plugin(s) in {os.path.normpath(out)}/index.json")


if __name__ == "__main__":
    main()
