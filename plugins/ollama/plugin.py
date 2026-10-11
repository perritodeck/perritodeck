"""Ollama: load, unload, pull and ask local language models.

Uses Ollama's HTTP API (/api/ps, /api/generate, /api/pull). Anything that can take
longer than a key may wait (loading a big model, pulling, answering) runs in the
background: the action answers within a few seconds and live values show how it went.
Standard library only.
"""

import json
import os
import re
import shutil
import subprocess
import sys
import threading
import urllib.error
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()
NOTHING = "–"
DEFAULT_URL = "http://127.0.0.1:11434"
TIMEOUT = 5  # quick calls (/api/ps, unload)
WAIT_FOR_ACTION = 7.0  # how long an action waits for background work before answering
BACKGROUND_TIMEOUT = 600  # loading, pulling and answering can be slow
POLL_SECONDS = 5
ANSWER_LENGTH = 80
# subprocess.CREATE_NO_WINDOW only exists on Windows; the value is fixed.
CREATE_NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0x08000000)

_busy = {}  # "ask" / "pull:<model>" / "load:<model>" -> Thread
_busy_lock = threading.Lock()


class OllamaError(RuntimeError):
    pass


def base_url():
    url = ((plugin.settings or {}).get("url") or "").strip().rstrip("/") or DEFAULT_URL
    if not url.startswith(("http://", "https://")):
        url = "http://" + url
    return url


def call(method, path, body=None, timeout=TIMEOUT):
    url = base_url()
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url + path, data=data, method=method)
    req.add_header("User-Agent", "PerritoDeck")
    if data is not None:
        req.add_header("Content-Type", "application/json")
    try:
        resp = urllib.request.urlopen(req, timeout=timeout)
    except urllib.error.HTTPError as e:
        raise OllamaError(describe_http_error(e)) from None
    except urllib.error.URLError as e:
        raise OllamaError(f"couldn't reach Ollama at {url}: {plain_reason(e.reason)}") from None
    except OSError as e:
        raise OllamaError(f"couldn't reach Ollama at {url}: {plain_reason(e)}") from None
    return resp


def plain_reason(reason):
    """"[Errno 111] Connection refused" -> "connection refused"."""
    if isinstance(reason, OSError) and reason.strerror:
        text = reason.strerror
    else:
        text = re.sub(r"^\[Errno -?\d+\]\s*", "", str(reason))
    if "timed out" in text:
        return "no answer (timed out)"
    return text[:1].lower() + text[1:] if text[:2] != text[:2].upper() else text


def describe_http_error(e):
    message = ""
    try:
        message = json.loads(e.read().decode("utf-8") or "{}").get("error") or ""
    except Exception:
        pass
    return f"Ollama said: {message}" if message else f"Ollama said {e.code} {e.reason}"


def call_json(method, path, body=None, timeout=TIMEOUT):
    resp = call(method, path, body, timeout)
    with resp:
        try:
            raw = resp.read().decode("utf-8")
        except OSError as e:
            raise OllamaError(f"Ollama stopped answering: {e}") from None
    try:
        answer = json.loads(raw or "{}")
    except ValueError:
        raise OllamaError("Ollama sent an answer that isn't JSON; check the address") from None
    if isinstance(answer, dict) and answer.get("error"):
        raise OllamaError(f"Ollama said: {answer['error']}")
    return answer


def model_name(settings):
    model = (settings.get("model") or "").strip()
    if not model:
        raise OllamaError("choose a model, e.g. llama3.2")
    return model


def same_model(a, b):
    """llama3.2 and llama3.2:latest are the same model."""
    def full(name):
        name = (name or "").strip()
        return name if ":" in name.rsplit("/", 1)[-1] else name + ":latest"

    return full(a) == full(b)


def short(name):
    return name[: -len(":latest")] if name.endswith(":latest") else name


def loaded_models():
    return call_json("GET", "/api/ps").get("models") or []


# --- background work ---------------------------------------------------------------


def in_background(key, work, busy_message):
    """Run work() on a thread; wait up to WAIT_FOR_ACTION for it. Raise its error if it
    failed quickly, return busy_message if the same work is still running from before."""
    with _busy_lock:
        running = _busy.get(key)
        if running is not None and running.is_alive():
            return busy_message
        outcome = {}

        def run():
            try:
                work()
            except Exception as e:
                outcome["error"] = e
                plugin.log(f"{key}: {e}", "warn")
            finally:
                with _busy_lock:
                    if _busy.get(key) is threading.current_thread():
                        del _busy[key]
                safe_poll()

        thread = threading.Thread(target=run, daemon=True)
        _busy[key] = thread
        thread.start()
    thread.join(WAIT_FOR_ACTION)
    if "error" in outcome:
        raise outcome["error"]
    return None


# --- actions ---------------------------------------------------------------------------


@plugin.action("load")
def load(settings):
    model = model_name(settings)
    keep = (settings.get("keep_alive") or "").strip()
    body = {"model": model}
    if keep:
        body["keep_alive"] = int(keep) if keep.lstrip("-").isdigit() else keep
    return in_background(
        "load:" + model,
        lambda: call_json("POST", "/api/generate", body, timeout=BACKGROUND_TIMEOUT),
        f"still loading {model}",
    )


def unload_model(model):
    call_json("POST", "/api/generate", {"model": model, "keep_alive": 0}, timeout=TIMEOUT)


@plugin.action("unload")
def unload(settings):
    unload_model(model_name(settings))
    safe_poll()


@plugin.action("unload-all")
def unload_all(settings):
    models = [m.get("model") or m.get("name") for m in loaded_models()]
    if not models:
        return "no model is loaded"
    for model in models:
        unload_model(model)
    safe_poll()
    return None


def pull_progress(model, line):
    try:
        event = json.loads(line)
    except ValueError:
        return
    if event.get("error"):
        raise OllamaError(f"Ollama said: {event['error']}")
    status = event.get("status") or ""
    total, done = event.get("total"), event.get("completed")
    if total and done is not None:
        plugin.set_value("pull_status", f"{short(model)} {int(done * 100 / total)}%")
    elif status == "success":
        plugin.set_value("pull_status", f"{short(model)} ready")
    elif status:
        plugin.set_value("pull_status", f"{short(model)}: {status}")


@plugin.action("pull")
def pull(settings):
    model = model_name(settings)

    def work():
        plugin.set_value("pull_status", f"{short(model)}: starting")
        try:
            resp = call("POST", "/api/pull", {"model": model, "stream": True}, timeout=BACKGROUND_TIMEOUT)
            with resp:
                for raw in resp:
                    line = raw.decode("utf-8", "replace").strip()
                    if line:
                        pull_progress(model, line)
        except Exception as e:
            plugin.set_value("pull_status", f"{short(model)} failed")
            if isinstance(e, OllamaError):
                raise
            raise OllamaError(f"pulling {model} stopped: {e}") from None

    return in_background("pull:" + model, work, f"already pulling {model}")


def one_line(text, limit=ANSWER_LENGTH):
    text = " ".join((text or "").split())
    return text if len(text) <= limit else text[: limit - 1].rstrip() + "…"


def clipboard_command():
    """The command that puts stdin on the clipboard, and how to encode the text."""
    if sys.platform == "win32":
        # clip.exe reads UTF-16 when the text starts with a byte order mark.
        return ["clip"], "utf-16"
    if sys.platform == "darwin":
        return ["pbcopy"], "utf-8"
    if os.environ.get("WAYLAND_DISPLAY") and shutil.which("wl-copy"):
        return ["wl-copy"], "utf-8"
    if shutil.which("xclip"):
        return ["xclip", "-selection", "clipboard"], "utf-8"
    if shutil.which("xsel"):
        return ["xsel", "--clipboard", "--input"], "utf-8"
    if shutil.which("wl-copy"):
        return ["wl-copy"], "utf-8"
    return None, None


def copy_to_clipboard(text):
    command, encoding = clipboard_command()
    if command is None:
        raise OllamaError("no clipboard tool found (install wl-clipboard or xclip)")
    env = dict(os.environ)
    if sys.platform == "darwin":
        env.setdefault("LANG", "en_US.UTF-8")  # pbcopy reads UTF-8 only with a UTF-8 locale
    # xclip and wl-copy stay in the background to serve the clipboard: don't wait on
    # their output pipes.
    subprocess.run(
        command,
        input=text.encode(encoding),
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        env=env,
        timeout=5,
        check=True,
        creationflags=CREATE_NO_WINDOW if sys.platform == "win32" else 0,
    )


@plugin.action("ask")
def ask(settings):
    model = model_name(settings)
    prompt = (settings.get("prompt") or "").strip()
    if not prompt:
        raise OllamaError("write a question or prompt")
    copy = settings.get("copy", True) not in (False, "false", 0)

    def work():
        plugin.set_value("last_answer", "thinking…")
        try:
            answer = call_json(
                "POST",
                "/api/generate",
                {"model": model, "prompt": prompt, "stream": False},
                timeout=BACKGROUND_TIMEOUT,
            )
        except Exception:
            plugin.set_value("last_answer", "no answer")
            raise
        text = (answer.get("response") or "").strip()
        plugin.set_value("last_answer", one_line(text) or NOTHING)
        if copy and text:
            try:
                copy_to_clipboard(text)
            except Exception as e:
                plugin.log(f"couldn't copy the answer: {e}", "warn")

    return in_background("ask", work, "still answering the last question")


# --- live values -------------------------------------------------------------------------


def publish(models):
    names = [short(m.get("name") or m.get("model") or "") for m in models]
    plugin.set_value("loaded", ", ".join(n for n in names if n) or "none")
    plugin.set_value("loaded_count", str(len(models)))
    vram = sum(int(m.get("size_vram") or 0) for m in models)
    plugin.set_value("vram", f"{vram / 1e9:.1f}")
    for wanted in plugin.watched("model_loaded"):
        on = any(same_model(wanted, m.get("model") or m.get("name")) for m in models)
        plugin.set_value("model_loaded", "on" if on else "off", arg=wanted)


def publish_unknown():
    for value_id in ("loaded", "loaded_count", "vram"):
        plugin.set_value(value_id, NOTHING)
    for wanted in plugin.watched("model_loaded"):
        plugin.set_value("model_loaded", NOTHING, arg=wanted)


_poll_lock = threading.Lock()
_last_error = {"text": None}


def safe_poll():
    if not _poll_lock.acquire(blocking=False):
        return
    try:
        try:
            models = loaded_models()
        except OllamaError as e:
            if _last_error["text"] != str(e):  # don't fill the log every 5 s
                plugin.log(str(e), "warn")
                _last_error["text"] = str(e)
            publish_unknown()
            return
        _last_error["text"] = None
        publish(models)
    except Exception as e:
        plugin.log(f"poll failed: {e}", "warn")
    finally:
        _poll_lock.release()


@plugin.every(POLL_SECONDS)
def tick():
    safe_poll()


@plugin.on_start
def started():
    plugin.set_value("last_answer", NOTHING)
    plugin.set_value("pull_status", NOTHING)


@plugin.on_watch
def watch_changed(values):
    threading.Thread(target=safe_poll, daemon=True).start()


if __name__ == "__main__":
    plugin.run()
