#!/usr/bin/env python3
"""Build the online plugin catalogue: one zip per plugin, index.json for the app, and
index.html, a page listing them for people.

    python3 plugins/build_catalog.py [OUT_DIR]

OUT_DIR defaults to website/plugins/. Publish that folder at
https://perritodeck.dev/plugins/ (the app reads index.json there). Zips are
reproducible: same files in, same bytes (and SHA-256) out. Tests and caches are
left out, and each zip carries this folder's MIT LICENSE unless the plugin has
its own. The page loads ../style.css, ../catalogue.css and ../catalogue.js from
the website, so it has no inline code for the site's Content-Security-Policy to
allow. Plugins listed in catalogue_status.json are marked Experimental, with a
note on what they were tested against, in index.json and on the page; their
manifests and zips stay as they are. Standard library only.
"""

import hashlib
import html
import json
import os
import re
import sys
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
SKIP_DIRS = {"__pycache__", "node_modules", ".git"}
# Plugin folders that aren't plugins themselves.
NOT_PLUGINS = {"sdk-python", "sdk-node"}
FIXED_TIME = (2026, 1, 1, 0, 0, 0)
# Everything in this folder is MIT-licensed; a zip on its own should still say so.
LICENSE = os.path.join(HERE, "LICENSE")
# A plugin's own licence file, at the top of its folder: LICENSE, LICENCE or COPYING, in
# any case, optionally with a qualifier and a text extension (LICENSE-MIT, LICENSE.md,
# COPYING.txt). A name that only starts like one, such as license_check.py, copying.py or
# licenses.json, is a file of the plugin's, not a licence.
LICENCE_FILE = re.compile(r"^(licen[cs]e|copying)(-[a-z0-9]+)*(\.(md|markdown|txt|text|rst))?$", re.I)
# Which plugins the catalogue marks Experimental, and why (see its "about").
STATUS = os.path.join(HERE, "catalogue_status.json")


def plugin_files(folder):
    for root, dirs, files in os.walk(folder):
        dirs[:] = sorted(d for d in dirs if d not in SKIP_DIRS)
        for name in sorted(files):
            if name.startswith("test_") or name.endswith((".pyc", ".pyo")):
                continue
            yield os.path.join(root, name)


def zip_entries(folder):
    """(name in the zip, file on disk) for everything a plugin's zip holds, in a fixed order:
    the plugin's own files, then LICENSE unless the plugin has a licence file of its own."""
    entries = [(os.path.relpath(path, folder).replace(os.sep, "/"), path) for path in plugin_files(folder)]
    if not any("/" not in rel and LICENCE_FILE.fullmatch(rel) for rel, _ in entries):
        entries.append(("LICENSE", LICENSE))
    return entries


def build_zip(folder, dest):
    with zipfile.ZipFile(dest, "w", zipfile.ZIP_DEFLATED) as z:
        for rel, path in zip_entries(folder):
            info = zipfile.ZipInfo(rel, FIXED_TIME)
            info.compress_type = zipfile.ZIP_DEFLATED
            executable = os.access(path, os.X_OK)
            info.external_attr = (0o100755 if executable else 0o100644) << 16
            with open(path, "rb") as f:
                z.writestr(info, f.read())


def load_status(path=STATUS):
    """{plugin id: what it was tested against} for the plugins marked Experimental."""
    name = os.path.basename(path)

    def once(pairs):
        # json.load would keep the last of two entries with the same key and drop the other.
        keys = [key for key, _ in pairs]
        for key in keys:
            if keys.count(key) > 1:
                sys.exit(f'{name}: "{key}" is there twice')
        return dict(pairs)

    with open(path, encoding="utf-8") as f:
        status = json.load(f, object_pairs_hook=once)
    listed = status.get("experimental", {}) if isinstance(status, dict) else None
    if not isinstance(listed, dict):
        sys.exit(f'{name}: "experimental" should map plugin ids to their notes')
    notes = {}
    for plugin_id, about in listed.items():
        tested = about.get("tested", "") if isinstance(about, dict) else ""
        if not isinstance(tested, str) or not tested.strip():
            sys.exit(f"{name}: {plugin_id} needs a \"tested\" note")
        notes[plugin_id] = " ".join(tested.split())
    return notes


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "..", "website", "plugins")
    entries = build(HERE, out, load_status())
    print(f"{len(entries)} plugin(s) in {os.path.normpath(out)}/index.json and index.html")


def build(src, out, experimental):
    """Zips every plugin in `src` into `out` and writes index.json and index.html there.
    `experimental` is load_status()'s {plugin id: tested note}."""
    plugins = []
    for name in sorted(os.listdir(src)):
        folder = os.path.join(src, name)
        manifest_path = os.path.join(folder, "perritodeck-plugin.json")
        if name in NOT_PLUGINS or not os.path.isfile(manifest_path):
            continue
        with open(manifest_path, encoding="utf-8") as f:
            plugins.append((folder, json.load(f)))
    # A mistake in catalogue_status.json stops the build before it writes anything.
    unknown = sorted(set(experimental) - {m["id"] for _, m in plugins})
    if unknown:
        sys.exit(f"{os.path.basename(STATUS)} lists plugins that aren't here: {', '.join(unknown)}")
    os.makedirs(out, exist_ok=True)
    entries = []
    for folder, m in plugins:
        zip_name = f"{m['id']}-{m['version']}.zip"
        build_zip(folder, os.path.join(out, zip_name))
        with open(os.path.join(out, zip_name), "rb") as f:
            digest = hashlib.sha256(f.read()).hexdigest()
        icon = copy_icon(folder, m, out)
        entries.append(
            {
                **({"icon": icon} if icon else {}),
                "id": m["id"],
                "name": m["name"],
                "version": m["version"],
                "description": m.get("description", ""),
                "author": m.get("author", ""),
                "url": zip_name,
                "sha256": digest,
                "runtime": m.get("runtime", ""),
                "actions": [a["name"] for a in m.get("actions", [])],
                "values": [
                    v["name"] + (f" (per {v['arg'].split('(')[0].strip().lower()})" if v.get("arg") else "")
                    for v in m.get("values", [])
                ],
                "size": os.path.getsize(os.path.join(out, zip_name)),
                # Apps that don't know these fields ignore them.
                **({"experimental": True, "tested": experimental[m["id"]]} if m["id"] in experimental else {}),
            }
        )
        print(f"{zip_name}  {digest}")
    with open(os.path.join(out, "index.json"), "w", encoding="utf-8") as f:
        json.dump({"version": 1, "plugins": entries}, f, indent=2, ensure_ascii=False)
        f.write("\n")
    with open(os.path.join(out, "index.html"), "w", encoding="utf-8") as f:
        f.write(page(entries))
    return entries


ICON_TYPES = (".svg", ".png", ".webp")


def copy_icon(folder, manifest, out):
    """Copies the manifest's optional `icon` next to the zip; returns its file name."""
    rel = manifest.get("icon")
    if not rel:
        return None
    path = os.path.normpath(os.path.join(folder, rel))
    ext = os.path.splitext(path)[1].lower()
    if not path.startswith(os.path.normpath(folder) + os.sep) or ext not in ICON_TYPES or not os.path.isfile(path):
        sys.exit(f"{manifest['id']}: icon must be an .svg, .png or .webp file inside the plugin folder: {rel}")
    name = f"{manifest['id']}-{manifest['version']}{ext}"
    with open(path, "rb") as src, open(os.path.join(out, name), "wb") as dest:
        dest.write(src.read())
    return name


RUNTIMES = {"python": "Python 3", "node": "Node.js", "exec": "nothing extra"}


def page(entries):
    """The catalogue as a web page, styled like the rest of the site."""
    e = html.escape
    items = []
    for i, p in enumerate(sorted(entries, key=lambda p: p["name"].lower())):
        initials = "".join(w[0] for w in p["name"].split()[:2]).upper() or "?"
        actions = "".join(f"<li>{e(a)}</li>" for a in p["actions"][:8])
        more = len(p["actions"]) - 8
        if more > 0:
            actions += f"<li class=more>and {more} more</li>"
        values = "".join(f"<li>{e(v)}</li>" for v in p["values"][:8])
        vmore = len(p["values"]) - 8
        if vmore > 0:
            values += f"<li class=more>and {vmore} more</li>"
        columns = ""
        if actions:
            columns += f"<div><h3>Keys can</h3><ul>{actions}</ul></div>"
        if values:
            columns += f"<div><h3>Titles can show</h3><ul>{values}</ul></div>"
        if p.get("icon"):
            badge = f'<img class="icon" src="{e(p["icon"])}" alt="" width="64" height="64">'
        else:
            badge = f'<span class="key" data-tone="{i % 7 + 1}" aria-hidden="true">{e(initials)}</span>'
        # What the search box matches against: everything the entry says.
        words = " ".join([p["name"], p["description"], *p["actions"], *p["values"]]).lower()
        # Experimental: a tag beside the version (the site's .tag style) and what it was tested against.
        flag, tested = "", ""
        if p.get("experimental"):
            flag = ' <span class="tag">Experimental</span>'
            tested = f'\n          <p class="tested">{e(p.get("tested", ""))}</p>'
            words += " experimental"
        items.append(
            f"""
      <article class="plugin" id="{e(p['id'])}" data-words="{e(words)}">
        {badge}
        <div class="about">
          <h2>{e(p['name'])} <span class="version">{e(p['version'])}</span>{flag}</h2>
          <p>{e(p['description'])}</p>{tested}
          <div class="columns">{columns}</div>
          <p class="get"><a href="{e(p['url'])}" download>Download ({max(1, p['size'] // 1024)} KB)</a>
            · needs {e(RUNTIMES.get(p['runtime'], p['runtime']))}
            <br><span class="sha" title="SHA-256">SHA-256 {e(p['sha256'])}</span></p>
        </div>
      </article>"""
        )
    explain = ""
    if any(p.get("experimental") for p in entries):
        explain = (
            '\n      <p class="note"><span class="tag">Experimental</span> plugins have only been tested against'
            " stand-ins for the service or device they work with, or have known problems with the real one,"
            " so they may not work as described yet.</p>"
        )
    return f"""<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Plugins — PerritoDeck</title>
  <meta name="description" content="Plugins that add actions and live values to PerritoDeck: OBS Studio, Home Assistant helpers, Proxmox, timers and more.">
  <link rel="canonical" href="https://perritodeck.dev/plugins/">
  <meta property="og:type" content="website">
  <meta property="og:site_name" content="PerritoDeck">
  <meta property="og:url" content="https://perritodeck.dev/plugins/">
  <meta property="og:title" content="Plugins — PerritoDeck">
  <meta property="og:description" content="Plugins that add actions and live values to PerritoDeck: OBS Studio, Home Assistant helpers, Proxmox, timers and more.">
  <meta property="og:image" content="https://perritodeck.dev/img/og.png">
  <meta property="og:image:width" content="1200">
  <meta property="og:image:height" content="630">
  <meta property="og:image:alt" content="PerritoDeck: a tablet and a phone showing grids of buttons, and the line “Your old tablet is a control deck now.”">
  <meta name="twitter:card" content="summary_large_image">
  <link rel="icon" href="../paw.svg" type="image/svg+xml">
  <link rel="apple-touch-icon" href="../apple-touch-icon.png">
  <link rel="stylesheet" href="../style.css">
  <link rel="stylesheet" href="../catalogue.css">
</head>
<body>
  <header class="site-header">
    <div class="wrap">
      <a class="brand" href="../">
        <img src="../paw.svg" alt="" width="34" height="34">
        PerritoDeck
      </a>
      <nav class="nav" aria-label="Main">
        <a href="../features.html">Features</a>
        <a href="../plugins/" aria-current="page">Plugins</a>
        <a href="../guide/">Guide</a>
        <a href="../faq.html">FAQ</a>
        <a href="../download.html">Download</a>
        <a href="https://github.com/perritodeck/perritodeck">GitHub</a>
      </nav>
    </div>
  </header>

  <main class="prose">
    <div class="wrap">
      <h1>Plugins</h1>
      <p class="updated">Plugins add things keys can do and values their titles can show. Add them in PerritoDeck from Plugins → Look online, or download one here, unzip it and use Install from folder. They're free and open source (MIT), and they run on your computer with your permissions.</p>{explain}
      <div class="finder" hidden>
        <label for="find">Find a plugin</label>
        <input id="find" type="search" placeholder="OBS, lights, timer…" autocomplete="off">
        <output for="find" id="found" aria-live="polite"></output>
      </div>
      <div class="plugins">{''.join(items)}
        <p class="none" id="none" hidden>No plugin matches that. Try another word, or <a href="../guide/plugins.html#write-your-own">write your own</a>.</p>
      </div>
    </div>
  </main>

  <footer class="site-footer">
    <div class="wrap">
      <span>PerritoDeck is free and open-source software. Not affiliated with Elgato, Corsair or Bitfocus.</span>
      <nav class="footer-nav" aria-label="More">
        <a href="../compatibility.html">Stream Deck and Companion</a>
        <a href="../privacy.html">Privacy</a>
        <a href="https://github.com/perritodeck/perritodeck">GitHub</a>
      </nav>
    </div>
  </footer>
  <!-- The search box: website/catalogue.js, a committed file, because this folder is generated. -->
  <script src="../catalogue.js"></script>
</body>
</html>
"""


if __name__ == "__main__":
    main()
