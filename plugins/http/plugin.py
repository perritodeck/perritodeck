"""HTTP requests from a key. Standard library only."""

import os
import sys
import urllib.error
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()
TIMEOUT = 8


def build_request(settings):
    url = (settings.get("url") or "").strip()
    if not url.startswith(("http://", "https://")):
        raise ValueError("the web address must start with http:// or https://")
    method = (settings.get("method") or "POST").upper()
    body = settings.get("body") or ""
    data = body.encode() if body and method != "GET" else None
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("User-Agent", "PerritoDeck")
    if data is not None:
        stripped = body.lstrip()
        req.add_header("Content-Type", "application/json" if stripped[:1] in "{[" else "text/plain; charset=utf-8")
    for line in (settings.get("headers") or "").splitlines():
        if ":" in line:
            name, value = line.split(":", 1)
            req.add_header(name.strip(), value.strip())
    return req


@plugin.action("request")
def request(settings):
    req = build_request(settings)
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
            plugin.set_value("last-status", str(resp.status))
            return None
    except urllib.error.HTTPError as e:
        plugin.set_value("last-status", str(e.code))
        return f"the server answered {e.code} {e.reason}"
    except urllib.error.URLError as e:
        plugin.set_value("last-status", "failed")
        raise RuntimeError(f"couldn't reach it: {e.reason}") from None


def with_value(settings):
    """Put a slider's position (0-100) wherever the settings say {value}."""
    value = str(settings.get("value", ""))
    return {k: v.replace("{value}", value) if isinstance(v, str) else v for k, v in settings.items()}


@plugin.action("send-value")
def send_value(settings):
    return request(with_value(settings))


if __name__ == "__main__":
    plugin.run()
