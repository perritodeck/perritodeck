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
| [`obs/`](obs/) | First-party: OBS Studio 28+ over obs-websocket 5: scenes, Studio Mode, outputs, audio, sources, filters, screenshots, live stats |
| [`timer/`](timer/) | First-party: countdowns, stopwatches and Pomodoro |
| [`counter/`](counter/) | First-party: named counters, optionally written to text files for OBS |
| [`uptime/`](uptime/) | First-party: is a host or web address up, and how fast |
| [`wol/`](wol/) | First-party: Wake-on-LAN |
| [`weather/`](weather/) | First-party: weather from Open-Meteo (no account) |
| [`ollama/`](ollama/) | First-party: load, unload, pull and ask local Ollama models |
| [`proxmox/`](proxmox/) | First-party: Proxmox VE guests: power, snapshots, status |
| [`truenas/`](truenas/) | First-party: TrueNAS pools, alerts, apps, scrubs, tasks (JSON-RPC API) |
| [`gitlab/`](gitlab/) | First-party: GitLab pipelines, merge requests, issues |
| [`soundboard/`](soundboard/) | First-party: play and stop sound files |
| [`audio/`](audio/) | First-party: mic mute, volume, output/input device switching, per-app volume |
| [`windows/`](windows/) | First-party: focus, launch, minimize, maximize, close and quit apps; show the desktop |
| [`screenshot/`](screenshot/) | First-party: screenshots of the screen, an area or a window |
| [`clipboard/`](clipboard/) | First-party: paste text snippets, clipboard slots |
| [`sysmon/`](sysmon/) | First-party: CPU, memory, disk, network, GPU, temperatures, battery |
| [`clock/`](clock/) | First-party: world clocks, countdowns to a date, days until |
| [`hue/`](hue/) | First-party: Philips Hue lights, rooms and scenes (local bridge API) |
| [`keylight/`](keylight/) | First-party: Elgato Key Light and Light Strip (local API) |
| [`spotify/`](spotify/) | First-party: Spotify playback, likes, playlists, devices (Web API) |
| [`twitch/`](twitch/) | First-party: Twitch stream, chat, moderation, ads, channel points, polls, predictions, live events |

Everything in this folder is MIT-licensed (see [LICENSE](LICENSE)) so plugin authors
can use the SDKs under any license.

Install a plugin from the desktop app (Plugins → Install from folder…), or copy its
folder into the host's settings folder under `plugins/`.

## Online catalogue

`python3 plugins/build_catalog.py` writes `website/plugins/index.json`, a browsable
`index.html` and one reproducible zip per plugin. Publishing that folder at `https://perritodeck.dev/plugins/`
makes the plugins available from the app's Plugins page ("Look online"). The app
refuses a zip whose SHA-256 doesn't match the index.
