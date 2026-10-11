# Plugins

Plugins add actions and live values to PerritoDeck. They're ordinary programs the host
starts and talks to over stdin/stdout (see [PROTOCOL.md](PROTOCOL.md)).

| Folder | What |
|---|---|
| [`sdk-node/`](sdk-node/) | SDK for JavaScript/TypeScript plugins (no dependencies) |
| [`sdk-python/`](sdk-python/) | SDK for Python plugins (standard library only) |
| [`http/`](http/) | First-party: send HTTP requests (webhooks, REST APIs) |
| [`mqtt/`](mqtt/) | First-party: publish MQTT messages |
| [`media/`](media/) | First-party: now playing and media controls (Windows media session, Spotify/Music on macOS, `playerctl` on Linux) |

Everything in this folder is MIT-licensed (see [LICENSE](LICENSE)) so plugin authors
can use the SDKs under any license.

Install a plugin from the desktop app (Plugins → Install from folder…), or copy its
folder into the host's settings folder under `plugins/`.

## Online catalogue

`python3 plugins/build_catalog.py` writes `website/plugins/index.json` and one
reproducible zip per plugin. Publishing that folder at `https://perritodeck.dev/plugins/`
makes the plugins available from the app's Plugins page ("Look online"). The app
refuses a zip whose SHA-256 doesn't match the index.
