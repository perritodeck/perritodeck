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

Run the tests with `python3 -m unittest test_sdk.py`.
