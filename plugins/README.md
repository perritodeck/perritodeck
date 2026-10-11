# Plugins

Plugins add actions and live values to PerritoDeck. They're ordinary programs the host
starts and talks to over stdin/stdout (see [PROTOCOL.md](PROTOCOL.md)).

| Folder | What |
|---|---|
| [`sdk-node/`](sdk-node/) | SDK for JavaScript/TypeScript plugins (no dependencies) |
| [`sdk-python/`](sdk-python/) | SDK for Python plugins (standard library only) |
| [`http/`](http/) | First-party: send HTTP requests (webhooks, REST APIs) |
| [`mqtt/`](mqtt/) | First-party: publish MQTT messages |
| [`media/`](media/) | First-party: now playing and media controls (Linux, via `playerctl`) |

Everything in this folder is MIT-licensed (see [LICENSE](LICENSE)) so plugin authors
can use the SDKs under any license.

Install a plugin from the desktop app (Plugins → Install from folder…), or copy its
folder into the host's settings folder under `plugins/`.
