#!/usr/bin/env python3
"""Build the online plugin catalogue: one zip per plugin, index.json for the app, and
index.html, a page listing them for people.

    python3 plugins/build_catalog.py [OUT_DIR]

OUT_DIR defaults to website/plugins/. Publish that folder at
https://perritodeck.dev/plugins/ (the app reads index.json there). Zips are
reproducible: same files in, same bytes (and SHA-256) out. Tests and caches are
left out. Standard library only.
"""

import hashlib
import html
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
            }
        )
        print(f"{zip_name}  {digest}")
    with open(os.path.join(out, "index.json"), "w", encoding="utf-8") as f:
        json.dump({"version": 1, "plugins": entries}, f, indent=2, ensure_ascii=False)
        f.write("\n")
    with open(os.path.join(out, "index.html"), "w", encoding="utf-8") as f:
        f.write(page(entries))
    print(f"{len(entries)} plugin(s) in {os.path.normpath(out)}/index.json and index.html")


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
        items.append(
            f"""
      <article class="plugin" id="{e(p['id'])}" data-words="{e(words)}">
        {badge}
        <div class="about">
          <h2>{e(p['name'])} <span class="version">{e(p['version'])}</span></h2>
          <p>{e(p['description'])}</p>
          <div class="columns">{columns}</div>
          <p class="get"><a href="{e(p['url'])}" download>Download ({max(1, p['size'] // 1024)} KB)</a>
            · needs {e(RUNTIMES.get(p['runtime'], p['runtime']))}
            <br><span class="sha" title="SHA-256">SHA-256 {e(p['sha256'])}</span></p>
        </div>
      </article>"""
        )
    return f"""<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Plugins — PerritoDeck</title>
  <meta name="description" content="Plugins that add actions and live values to PerritoDeck: OBS Studio, Home Assistant helpers, Proxmox, timers and more.">
  <link rel="icon" href="../paw.svg" type="image/svg+xml">
  <link rel="stylesheet" href="../style.css">
  <style>
    .plugins {{ display: grid; gap: 0; max-width: 900px; }}
    .plugin {{ display: grid; grid-template-columns: 64px minmax(0, 1fr); gap: 22px; padding: 28px 0; border-top: 1px solid var(--line); }}
    .plugin .key, .plugin .icon {{ width: 64px; height: 64px; cursor: default; font-size: 1.1rem; }}
    .plugin .icon {{ border-radius: var(--radius-key); }}
    .finder {{ display: flex; flex-wrap: wrap; align-items: center; gap: 8px 16px; margin: 0 0 8px; max-width: 900px; }}
    .finder label {{ font-weight: 700; }}
    .finder input {{
      flex: 1 1 16em; min-width: 0; padding: 11px 14px; border-radius: 12px; font: inherit;
      background: var(--paper); color: var(--ink); border: 1px solid var(--line);
    }}
    .finder output {{ color: var(--ink-soft); font-size: 0.95rem; }}
    .none {{ color: var(--ink-soft); padding: 28px 0; border-top: 1px solid var(--line); }}
    .plugin h2 {{ font-size: 1.45rem; margin: 0 0 6px; max-width: none; }}
    .version {{ font: 400 0.95rem/1 var(--body); color: var(--ink-soft); }}
    .plugin p {{ color: var(--ink-soft); }}
    .columns {{ display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px 32px; margin: 4px 0 14px; }}
    .columns h3 {{ font-size: 1rem; margin: 0 0 4px; }}
    .columns ul {{ margin: 0; padding-left: 1.1em; color: var(--ink-soft); font-size: 0.95rem; }}
    .columns li {{ margin-bottom: 0.1em; }}
    .columns .more {{ list-style: none; margin-left: -1.1em; font-style: italic; }}
    .get {{ font-size: 0.95rem; margin: 0; }}
    .sha {{ font-size: 0.75rem; overflow-wrap: anywhere; color: var(--ink-soft); }}
    @media (max-width: 640px) {{
      .plugin {{ grid-template-columns: 1fr; gap: 12px; }}
      .columns {{ grid-template-columns: 1fr; }}
    }}
  </style>
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
      <p class="updated">Plugins add things keys can do and values their titles can show. Add them in PerritoDeck from Plugins → Look online, or download one here, unzip it and use Install from folder. They're free and open source (MIT), and they run on your computer with your permissions.</p>
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
  <script>
    // The search box filters the list as you type. Without JavaScript it stays hidden and
    // every plugin is listed.
    (() => {{
      const finder = document.querySelector(".finder");
      const box = document.getElementById("find");
      const found = document.getElementById("found");
      const none = document.getElementById("none");
      const plugins = [...document.querySelectorAll(".plugin")];
      const show = () => {{
        const words = box.value.toLowerCase().split(/\\s+/).filter(Boolean);
        let shown = 0;
        for (const p of plugins) {{
          const hit = words.every((w) => p.dataset.words.includes(w));
          p.hidden = !hit;
          if (hit) shown += 1;
        }}
        found.textContent = words.length ? `${{shown}} of ${{plugins.length}}` : `${{plugins.length}} plugins`;
        none.hidden = shown > 0;
      }};
      finder.hidden = false;
      box.addEventListener("input", show);
      show();
    }})();
  </script>
</body>
</html>
"""


if __name__ == "__main__":
    main()
