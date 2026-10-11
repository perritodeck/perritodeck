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
  `options: [{ "value", "label" }]`).
- `values`: live values the plugin publishes; a key title shows one with
  `{plugin:<plugin id>:<value id>}`, and a toggle can `follow` it.

Installed plugins live in the host's settings folder under `plugins/<id>/`.

## Messages

Host → plugin:

| `event` | Fields | Meaning |
|---|---|---|
| `hello` | `protocol` (1), `host` (version) | First line after start. |
| `run` | `id` (number), `action`, `settings` (object), `device` | A key was pressed. Answer with `result` and the same `id` within 10 seconds. |
| `shutdown` | | Exit now (the host also closes stdin). |

Plugin → host:

| `event` | Fields | Meaning |
|---|---|---|
| `ready` | | Send once after `hello`; actions are only sent after this. |
| `result` | `id`, `ok` (bool), `message?` | How a `run` went. `ok: false` flashes ⚠ with the message; `ok: true` with a message flashes ⚠ too (a warning). |
| `value` | `id` (value id), `value` (string) | Publish a live value. |
| `log` | `level` (`info`, `warn`, `error`), `message` | Write to the host's log. |

Unknown events are ignored in both directions, so either side can add some later.
If a plugin exits, the host restarts it with a growing delay, up to 5 times in a row.
