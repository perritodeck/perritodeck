"""Is it up? Checks homelab services from the computer running PerritoDeck.

Targets (the value's argument):
  https://nas.lan:5001/   HTTP GET; up when the answer is 2xx or 3xx (redirects aren't followed)
  nas.lan:22              TCP connection to that port
  [fd00::5]:22            the same, for an IPv6 address
  nas.lan                 TCP port 443, then port 80 if 443 doesn't answer

No ping (ICMP): it needs administrator rights on most systems. Standard library only.
"""

import concurrent.futures
import http.client
import os
import socket
import ssl
import sys
import threading
import time
import urllib.error
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()

TIMEOUT = 3
DEFAULT_INTERVAL = 10
VALUES = ("up", "ms", "status")
UNKNOWN = "–"

_pool = concurrent.futures.ThreadPoolExecutor(max_workers=16)
_lock = threading.Lock()
_results = {}  # target -> {"up": "on", "ms": "12 ms", "status": "200"}
_last_poll = 0.0
_check_soon = threading.Event()


class Result:
    def __init__(self, up, status, ms=None):
        self.up, self.status, self.ms = up, str(status), ms

    def values(self):
        return {
            "up": "on" if self.up else "off",
            "ms": f"{round(self.ms)} ms" if self.up and self.ms is not None else UNKNOWN,
            "status": self.status,
        }


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None  # a 3xx answer is the result


def split_host_port(target):
    """'nas:22' -> ('nas', 22); '[fd00::5]:22' -> ('fd00::5', 22); 'nas' -> ('nas', None)."""
    if target.startswith("["):
        host, _, rest = target[1:].partition("]")
        port = rest[1:] if rest.startswith(":") else ""
    elif target.count(":") == 1:
        host, _, port = target.partition(":")
    else:
        host, port = target, ""  # a bare name, or a bare IPv6 address
    if port:
        if not port.isdigit() or not 0 < int(port) < 65536:
            raise ValueError(f"“{port}” isn't a port number")
        return host, int(port)
    return host, None


def check_tcp(host, port):
    start = time.monotonic()
    try:
        with socket.create_connection((host, port), timeout=TIMEOUT):
            return Result(True, "up", (time.monotonic() - start) * 1000)
    except OSError:
        return Result(False, "down")


def check_http(url):
    context = None
    if url.startswith("https://") and plugin.settings.get("insecure"):
        context = ssl.create_default_context()
        context.check_hostname = False
        context.verify_mode = ssl.CERT_NONE
    opener = urllib.request.build_opener(_NoRedirect, urllib.request.HTTPSHandler(context=context))
    request = urllib.request.Request(url, headers={"User-Agent": "PerritoDeck uptime"})
    start = time.monotonic()
    try:
        with opener.open(request, timeout=TIMEOUT) as resp:
            code = resp.status
    except urllib.error.HTTPError as e:
        code = e.code
        e.close()
    except (urllib.error.URLError, http.client.HTTPException, OSError, ValueError) as e:
        reason = getattr(e, "reason", e)
        if isinstance(reason, ssl.SSLCertVerificationError):
            plugin.log(f"{url}: certificate not trusted (turn on “Skip certificate check” for self-signed ones)", "warn")
        return Result(False, "down")
    ms = (time.monotonic() - start) * 1000
    return Result(200 <= code < 400, code, ms)


def check(target):
    """Check one target; never raises."""
    target = target.strip()
    try:
        if target.lower().startswith(("http://", "https://")):
            return check_http(target)
        host, port = split_host_port(target)
        if not host:
            raise ValueError("no host name")
        if port is not None:
            return check_tcp(host, port)
        result = check_tcp(host, 443)
        return result if result.up else check_tcp(host, 80)
    except Exception as e:
        plugin.log(f"can't check “{target}”: {e}", "warn")
        return Result(False, "down")


def watched_targets():
    targets = set()
    for value in VALUES:
        targets.update(plugin.watched(value))
    return sorted(targets)


def publish(target, result):
    new = result.values()
    with _lock:
        old = _results.get(target, {})
        _results[target] = new
    for value in VALUES:
        if old.get(value) != new[value] and target in plugin.watched(value):
            plugin.set_value(value, new[value], arg=target)


def check_all(targets):
    """Check targets at the same time and publish; returns {target: Result}."""
    futures = {t: _pool.submit(check, t) for t in targets}
    results = {t: f.result() for t, f in futures.items()}
    for target, result in results.items():
        publish(target, result)
    return results


def interval():
    try:
        return max(2.0, float(plugin.settings.get("interval") or DEFAULT_INTERVAL))
    except (TypeError, ValueError):
        return float(DEFAULT_INTERVAL)


@plugin.every(1)
def poll(now=None):
    global _last_poll
    now = time.monotonic() if now is None else now
    if not (_check_soon.is_set() or now - _last_poll >= interval()):
        return
    _check_soon.clear()
    _last_poll = now
    targets = watched_targets()
    if targets:
        check_all(targets)


@plugin.on_watch
def watching(_values):
    targets = set(watched_targets())
    with _lock:
        for target in [t for t in _results if t not in targets]:
            del _results[target]
        known = dict(_results)
    # Show what we already know for newly watched values; check new targets soon.
    for value in VALUES:
        for target in plugin.watched(value):
            if target in known:
                plugin.set_value(value, known[target][value], arg=target)
    if any(t not in known for t in targets):
        _check_soon.set()


@plugin.on_settings
def configure(_settings):
    _check_soon.set()


@plugin.on_start
def started():
    with _lock:
        _results.clear()
    _check_soon.set()


@plugin.action("check")
def check_now(settings):
    target = str(settings.get("target") or "").strip()
    targets = [target] if target else watched_targets()
    if not targets:
        return "nothing to check: no key shows a target yet"
    results = check_all(targets)
    down = [t if r.status == "down" else f"{t} ({r.status})" for t, r in results.items() if not r.up]
    if down:
        return f"{', '.join(down)} {'is' if len(down) == 1 else 'are'} down"
    return None


if __name__ == "__main__":
    plugin.run()
