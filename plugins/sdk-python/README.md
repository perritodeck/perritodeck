# PerritoDeck plugin SDK for Python

One file, standard library only: copy [`perritodeck_plugin.py`](perritodeck_plugin.py)
into your plugin's folder.

```python
from perritodeck_plugin import Plugin

plugin = Plugin()

@plugin.action("say-hello")
def say_hello(settings):
    plugin.log(f"hello, {settings.get('name', 'world')}")
    # return "some text" to flash a warning on the key; raise to flash an error

plugin.run()
```

`perritodeck-plugin.json` next to it:

```json
{
  "id": "com.example.hello",
  "name": "Hello",
  "version": "1.0.0",
  "runtime": "python",
  "main": "plugin.py",
  "actions": [
    { "id": "say-hello", "name": "Say hello",
      "fields": [{ "id": "name", "label": "Name", "type": "text" }] }
  ]
}
```

Live values: declare them under `"values"` in the manifest and publish with
`plugin.set_value("id", "text")`; a key shows one with `{plugin:com.example.hello:id}`.

Keep the plugin's own files in `plugin.data_dir` (a folder the host gives each
plugin; created on first use). `on_settings` and `on_watch` callbacks run in order
on one worker thread, so a slow one never delays actions; stdin and stdout are UTF-8
whatever the locale.

On Windows the host ends every program a plugin started when the plugin stops
(restarted, updated, removed, or PerritoDeck quitting), so players and helpers never
linger. Open browsers and apps meant for the user with `open_for_user(target)` (a web
address, file, folder or app name, opened as a double-click would) or
`start_for_user([program, *args])` instead of `webbrowser`, `os.startfile` or
`subprocess`: those keep running, start in the program's own folder rather than the
plugin's, and get none of the plugin's pipes.

Run the tests with `python3 -m unittest test_sdk.py`.
