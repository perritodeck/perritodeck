# Phase 5 — Plugins

**Goal:** let anyone add actions and live values without changing PerritoDeck, in the
languages people already use, without plugins being able to break the host.

## Design

- A plugin is a **separate program** (Node.js, Python, or any executable) that the host
  starts and talks to over stdin/stdout, one JSON object per line
  ([plugins/PROTOCOL.md](../../plugins/PROTOCOL.md)). A crash or hang can't take the
  host down; a plugin that keeps crashing is stopped and shown as such.
- A folder with `perritodeck-plugin.json` declares the plugin's actions (with settings
  fields the editor turns into a form) and live values.
- Keys use them with `action = { type = "plugin", plugin = "…", action = "…", settings = {…} }`
  and titles with `{plugin:<id>:<value>}`.
- SDKs: [Python](../../plugins/sdk-python) (one file, standard library) and
  [Node.js](../../plugins/sdk-node) (no dependencies, TypeScript types). MIT licensed.

## Status (2026-10-11)

Done:
- Plugin runtime in the host (`perritodeck-plugins`): start, restart with backoff, give
  up after repeated crashes, 10 s answer timeout, missing-runtime detection.
- SDKs for Python and Node.js, with tests.
- First-party plugins, bundled with the desktop app: **HTTP requests**, **MQTT**
  (publish; MQTT 3.1.1 implemented directly), **Now playing** (Windows media session through PowerShell, Spotify or Music on macOS through osascript, playerctl on Linux; the Windows and macOS parts are untested on real machines).
  OBS and Home Assistant stay built in.
- Desktop app: Plugins page (status, restart, remove, add bundled ones, install from a
  folder), and plugin actions with generated settings forms in the editor.
- Tests: runtime tests with real plugin processes, SDK and plugin tests
  (`plugins/test_all.py`), and the real-app test installs the bundled HTTP plugin.

Not done:
- A plugin catalogue online (download and update plugins from the app).
- Plugins driving sliders, and plugins drawing their own key images.
- A Spotify plugin using its Web API.
- Signing or sandboxing plugins. For now the app warns that plugins run with the
  user's permissions.
