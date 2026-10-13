# Plugin protocol, version 1

A plugin is a program the PerritoDeck host starts and talks to over **stdin/stdout,
one JSON object per line** (UTF-8, `\n`-terminated). Anything a plugin writes to
stderr goes to the host's log. The SDKs in this folder implement all of this; you
only need this page to write a plugin in another language.

## Files

A plugin is a folder containing `perritodeck-plugin.json`:

```json
{
  "id": "com.example.weather",
  "name": "Weather",
  "version": "1.0.0",
  "description": "Shows the weather on a key.",
  "author": "Example",
  "runtime": "python",
  "main": "plugin.py",
  "actions": [
    {
      "id": "refresh",
      "name": "Refresh the weather",
      "fields": [
        { "id": "city", "label": "City", "type": "text", "placeholder": "Lisbon" }
      ]
    }
  ],
  "values": [
    { "id": "temperature", "name": "Temperature" }
  ]
}
```

- `id`: reverse-DNS style, lowercase letters, digits, `.`, `-`, `_`.
- `runtime`: `node` (runs `node <main>`), `python` (runs Python 3 with `<main>`), or
  `exec` (runs `<main>` itself; for compiled plugins).
- `actions`: what keys can do with this plugin. `fields` describe the settings the
  desktop editor asks for; `type` is `text`, `number`, `checkbox` or `select` (with
  `options: [{ "value", "label" }]`). With `"slider": true` the action belongs on a
  slider instead of a key: each `run` carries the slider's position as
  `settings.value` (0–100), and while someone drags, the host sends only the latest
  position once the previous `run` has answered.
- `values`: live values the plugin publishes; a key title shows one with
  `{plugin:<plugin id>:<value id>}`, a toggle can `follow` it, and a slider can
  `follow` one that is a number from 0 to 100 so it sits where the plugin says.
  A value with `"arg": "<label>"` takes an argument saying what to show, e.g.
  `{ "id": "up", "name": "Host is up", "arg": "Host or address" }`: a title uses
  `{plugin:<plugin id>:up:192.168.1.10}`. The host tells the plugin which arguments
  profiles use (`watch`), and the plugin publishes `value` with id `up:192.168.1.10`.
  Arguments may contain `:` (e.g. `nas:443`); the value id is everything before the
  first `:`.
- `settings`: fields for the plugin as a whole (a server address, an API token), same
  shape as action fields plus the type `password` (shown masked). People fill them in
  on the Plugins page; the host stores them in its settings folder
  (`plugin-settings/<id>.json`, readable by the user only) and sends them in `hello`
  and in `settings` whenever they change.
- `icon` (optional): a square picture of the plugin, as a path inside its folder
  (`.svg`, `.png` or `.webp`, at least 128 × 128). The online catalogue shows it; the
  host doesn't use it yet.

Installed plugins live in the host's settings folder under `plugins/<id>/`.

## Environment

The host starts each plugin with its folder as the working directory and these
environment variables set:

| Variable | Value | Why |
|---|---|---|
| `PERRITODECK_PLUGIN_DATA` | A folder for this plugin only, in the host's settings folder; the host creates it. | Keep the plugin's own files here (counters, state, caches), not next to the code: the plugin's folder is replaced on update. The SDKs expose it as `plugin.data_dir` (Python) and `plugin.dataDir` (Node) and fall back to a per-user folder when the plugin runs outside the host. |
| `PYTHONUTF8` | `1` | Python plugins use UTF-8 for files and pipes whatever the system locale. |
| `PYTHONIOENCODING` | `utf-8` | The same for stdin/stdout on older Pythons. |

The pipes are always UTF-8, so a plugin in another language should read and write
UTF-8 explicitly rather than rely on the locale (cp1252 on many Windows machines).
On Windows the host starts plugins without a console window (`CREATE_NO_WINDOW`), so a
plugin must not expect one.

## Messages

Host → plugin:

| `event` | Fields | Meaning |
|---|---|---|
| `hello` | `protocol` (1), `host` (version), `settings` (object), `features` (array of strings) | First line after start. `features` lists what this host can do beyond the messages below; today that's `open` (see `open`). Older hosts leave it out. |
| `settings` | `settings` (object) | The plugin's settings changed on the Plugins page. |
| `watch` | `values` (array of strings) | The argument values profiles use right now, e.g. `["up:nas", "up:10.0.0.1"]`; sent when the list changes and again after each start. Publish only these, and stop polling the rest. |
| `run` | `id` (number), `action`, `settings` (object), `device` | A key was pressed (or a slider moved). Answer with `result` and the same `id` within 10 seconds. |
| `opened` | `id`, `ok` (bool), `message?` | How an `open` with that `id` went; `message` says why it failed. |
| `shutdown` | | Exit now (the host also closes stdin). |

Plugin → host:

| `event` | Fields | Meaning |
|---|---|---|
| `ready` | | Send once after `hello`; actions are only sent after this. |
| `result` | `id`, `ok` (bool), `message?` | How a `run` went. `ok: false` flashes ⚠ with the message; `ok: true` with a message flashes ⚠ too (a warning). |
| `value` | `id` (value id), `value` (string) | Publish a live value. |
| `log` | `level` (`info`, `warn`, `error`), `message` | Write to the host's log. |
| `open` | `target`, or `program` and `args?` (array of strings); `id?` | Ask the host to open something for the person: a web address, file, folder or app link (`target`, opened as double-clicking would), or a program. The host opens it the way its Open and Launch keys do, so it keeps running after the plugin stops. With an `id`, the host answers with `opened`. Only for hosts whose `hello` lists `open` in `features`. |

Unknown events are ignored in both directions, so either side can add some later.

### Opening things for the person

On Windows the host runs each plugin in a job object, so that a plugin's own helpers
end with it. That means a browser or app the plugin starts itself closes when the plugin
stops or restarts. Use `open` instead. The Node SDK has `plugin.openForUser(target)` and
`plugin.startForUser(program, args)`, which use it when the host can and otherwise start
the program themselves, as before. The Python SDK's `open_for_user` and
`start_for_user` leave the job by themselves and work with any host.
If a plugin exits, the host restarts it with a growing delay, up to 5 times in a row.
