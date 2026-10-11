# Ollama

Load, unload and download local AI models in [Ollama](https://ollama.com), see which
ones are loaded and how much graphics memory they use, and ask a model a question from
a key: the answer goes to the clipboard. Plugin id: `dev.perritodeck.ollama`.

It uses Ollama's HTTP API (`/api/ps`, `/api/generate`, `/api/pull`), polling `/api/ps`
every 5 seconds. Python 3.8+ standard library only; works on Windows, macOS and Linux.

## Setup

Nothing to set up when Ollama runs on the same computer as the PerritoDeck host: the
default address is `http://127.0.0.1:11434`. For Ollama on another machine, start it
there with `OLLAMA_HOST=0.0.0.0` and put its address in **Plugins → Ollama → Ollama
address**, e.g. `http://192.168.1.30:11434`. Ollama has no login of its own; only
expose it on a network you trust.

## Actions

| Action | Fields | What it does |
|---|---|---|
| Load a model | Model; keep it loaded for (optional: `30m`, `2h`, `-1` for always) | `POST /api/generate` with no prompt, which loads the model |
| Unload a model | Model | `POST /api/generate` with `keep_alive: 0` |
| Unload all models | | The same for every model `/api/ps` lists. Warns when nothing is loaded. |
| Download a model | Model | `POST /api/pull`; the progress shows in the value `pull_status` |
| Ask a question | Model, prompt, copy the answer to the clipboard (on by default) | `POST /api/generate` with `stream: false`; the answer goes to the clipboard and to `last_answer` |

Model names are Ollama's, e.g. `llama3.2`, `qwen3:8b` or `me/my-model:latest`.

Loading a large model, downloading and answering can take much longer than the 10
seconds a key may wait. These run in the background: the key waits up to 7 seconds,
reports an error if one came by then (e.g. `model 'x' not found`), and otherwise
answers OK while the work carries on. Watch `model_loaded`, `pull_status` or
`last_answer` to see when it's done. Pressing the key again while the same work is
still running flashes a ⚠ ("still answering the last question") instead of starting
it twice.

The clipboard uses `clip` on Windows, `pbcopy` on macOS, and `wl-copy` (Wayland) or
`xclip`/`xsel` (X11) on Linux; install `wl-clipboard` or `xclip` if neither is there.

## Values

| Value | Example title | Shows |
|---|---|---|
| `loaded` | `{plugin:dev.perritodeck.ollama:loaded}` | Loaded models, e.g. `llama3.2, qwen3:8b` (`none` when empty) |
| `loaded_count` | `{plugin:dev.perritodeck.ollama:loaded_count} loaded` | How many models are loaded |
| `vram` | `{plugin:dev.perritodeck.ollama:vram} GB` | Graphics memory loaded models use (`size_vram`), in GB |
| `model_loaded:<model>` | `{plugin:dev.perritodeck.ollama:model_loaded:llama3.2}` | `on` / `off`, for a toggle to follow (`llama3.2` matches `llama3.2:latest`) |
| `last_answer` | `{plugin:dev.perritodeck.ollama:last_answer}` | The last answer on one line, cut to 80 characters; `thinking…` while waiting |
| `pull_status` | `{plugin:dev.perritodeck.ollama:pull_status}` | e.g. `llama3.2 42%`, `llama3.2 ready`, `llama3.2 failed` |

Values show `–` when Ollama can't be reached.

## Tested and untested

Tested with unit tests against a fake Ollama server shaped like the documented API
(`docs/api.md` in the Ollama repository: `/api/ps` with `size_vram`, load and unload
through `/api/generate`, streamed `/api/pull` progress with `total`/`completed`), and
with the clipboard commands mocked.

Not yet tried against a real Ollama, and the clipboard commands haven't been run on
real Windows, macOS or Linux desktops (`clip` is fed UTF-16 with a byte order mark so
accents survive; that is the part most worth checking).
