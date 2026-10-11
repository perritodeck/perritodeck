"""Proxmox VE: start, stop and watch virtual machines and containers.

Talks to the Proxmox VE REST API (/api2/json) with an API token. Keys only need the
guest's id (vmid): the plugin finds its node and type (qemu or lxc) in
/cluster/resources, which is also the one call each poll makes. Standard library only.
"""

import json
import os
import re
import ssl
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()
NOTHING = "–"
TIMEOUT = 4  # seconds per request; an action makes at most two, well inside 10 s
POLL_SECONDS = 5
IDLE_POLL_SECONDS = 30  # only guests_running is shown: poll less often
REFRESH_AFTER_ACTION = 2.0

GUEST_VALUES = ("status", "running", "cpu", "mem", "uptime", "name")
NODE_VALUES = ("node_cpu", "node_mem", "node_status")

_lock = threading.Lock()
_cache = {"resources": None, "at": 0.0}
_last_poll = {"at": 0.0}


class ProxmoxError(RuntimeError):
    pass


# --- talking to Proxmox ------------------------------------------------------


def config():
    s = plugin.settings or {}
    url = (s.get("url") or "").strip().rstrip("/")
    if not url:
        raise ProxmoxError("set the Proxmox address on the Plugins page")
    if not url.startswith(("https://", "http://")):
        url = "https://" + url
    if url.endswith("/api2/json"):
        url = url[: -len("/api2/json")]
    token_id = (s.get("token_id") or "").strip()
    secret = (s.get("token_secret") or "").strip()
    if not token_id or not secret:
        raise ProxmoxError("set the API token id and secret on the Plugins page")
    if "!" not in token_id or "@" not in token_id:
        raise ProxmoxError("the token id looks like user@realm!name, e.g. root@pam!perritodeck")
    return url, token_id, secret, bool(s.get("insecure"))


def tls_context(insecure):
    if not insecure:
        return ssl.create_default_context()
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    return ctx


def api(method, path, params=None):
    """Call /api2/json<path>; return the "data" member of the answer."""
    url, token_id, secret, insecure = config()
    full = f"{url}/api2/json{path}"
    data = None
    if params and method == "GET":
        full += "?" + urllib.parse.urlencode(params)
    elif method != "GET":
        data = urllib.parse.urlencode(params or {}).encode()
    req = urllib.request.Request(full, data=data, method=method)
    req.add_header("Authorization", f"PVEAPIToken={token_id}={secret}")
    req.add_header("Accept", "application/json")
    req.add_header("User-Agent", "PerritoDeck")
    if data is not None:
        req.add_header("Content-Type", "application/x-www-form-urlencoded")
    context = tls_context(insecure) if full.startswith("https://") else None
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT, context=context) as resp:
            body = resp.read()
    except urllib.error.HTTPError as e:
        raise ProxmoxError(describe_http_error(e)) from None
    except urllib.error.URLError as e:
        raise ProxmoxError(describe_url_error(url, e.reason)) from None
    except (OSError, ssl.SSLError) as e:  # timeouts and resets outside URLError
        raise ProxmoxError(describe_url_error(url, e)) from None
    try:
        return json.loads(body.decode("utf-8") or "{}").get("data")
    except (ValueError, AttributeError):
        raise ProxmoxError("Proxmox sent an answer that isn't JSON; check the address") from None


def describe_http_error(e):
    # Proxmox puts its error text in the HTTP reason phrase (and sometimes "message").
    reason = (e.reason or "").strip()
    try:
        body = json.loads(e.read().decode("utf-8") or "{}")
        if isinstance(body, dict) and body.get("message"):
            reason = str(body["message"]).strip()
    except Exception:
        pass
    if e.code == 401:
        return "Proxmox refused the token: check the token id and secret"
    if e.code == 403:
        return f"the token isn't allowed to do that ({reason})" if reason else "the token isn't allowed to do that"
    return f"Proxmox said {e.code}: {reason}" if reason else f"Proxmox said {e.code}"


def describe_url_error(url, reason):
    text = str(reason)
    if isinstance(reason, ssl.SSLCertVerificationError) or "CERTIFICATE_VERIFY_FAILED" in text:
        return (
            f"couldn't check the certificate of {url}; Proxmox uses a self-signed one, "
            "so turn on \"Skip certificate check\" or install a trusted certificate"
        )
    return f"couldn't reach Proxmox at {url}: {plain_reason(reason)}"


def plain_reason(reason):
    """"[Errno 111] Connection refused" -> "connection refused"."""
    if isinstance(reason, OSError) and reason.strerror:
        text = reason.strerror
    else:
        text = re.sub(r"^\[Errno -?\d+\]\s*", "", str(reason))
    if "timed out" in text:
        return "no answer (timed out)"
    return text[:1].lower() + text[1:] if text[:2] != text[:2].upper() else text


def resources(max_age=0.0):
    """Every node, VM and container, from one /cluster/resources call (cached)."""
    with _lock:
        if _cache["resources"] is not None and time.monotonic() - _cache["at"] <= max_age:
            return _cache["resources"]
    items = api("GET", "/cluster/resources") or []
    with _lock:
        _cache["resources"] = items
        _cache["at"] = time.monotonic()
    return items


def forget():
    with _lock:
        _cache["resources"] = None


def parse_vmid(text):
    text = str(text or "").strip()
    if not text.isdigit():
        raise ProxmoxError("enter the VM or container id (a number like 101)")
    return int(text)


def guest_in(items, vmid):
    for r in items or []:
        if r.get("type") in ("qemu", "lxc") and str(r.get("vmid")) == str(vmid):
            return r
    return None


def find_guest(vmid):
    guest = guest_in(resources(max_age=POLL_SECONDS), vmid)
    if guest is None:
        raise ProxmoxError(f"there's no VM or container {vmid} (or the token can't see it: it needs VM.Audit)")
    return guest


def guest_path(guest):
    return f"/nodes/{urllib.parse.quote(str(guest['node']))}/{guest['type']}/{guest['vmid']}"


# --- actions -------------------------------------------------------------------


def refresh_soon():
    forget()
    timer = threading.Timer(REFRESH_AFTER_ACTION, safe_poll)
    timer.daemon = True
    timer.start()


def power(settings, command):
    vmid = parse_vmid(settings.get("vmid"))
    guest = find_guest(vmid)
    kind = "VM" if guest["type"] == "qemu" else "container"
    if command in ("suspend", "resume") and guest["type"] != "qemu":
        raise ProxmoxError("pause and resume only work for VMs, not containers")
    if command == "toggle":
        command = "shutdown" if guest.get("status") == "running" else "start"
    status = guest.get("status")
    if command == "start" and status == "running":
        return f"{kind} {vmid} is already running"
    if command in ("shutdown", "stop", "reboot", "suspend") and status == "stopped":
        return f"{kind} {vmid} is already stopped"
    api("POST", f"{guest_path(guest)}/status/{command}")
    refresh_soon()
    return None


@plugin.action("start")
def start(settings):
    return power(settings, "start")


@plugin.action("shutdown")
def shutdown(settings):
    return power(settings, "shutdown")


@plugin.action("stop")
def stop(settings):
    return power(settings, "stop")


@plugin.action("reboot")
def reboot(settings):
    return power(settings, "reboot")


@plugin.action("suspend")
def suspend(settings):
    return power(settings, "suspend")


@plugin.action("resume")
def resume(settings):
    return power(settings, "resume")


@plugin.action("toggle")
def toggle(settings):
    return power(settings, "toggle")


SNAPSHOT_NAME = re.compile(r"^[A-Za-z][A-Za-z0-9_\-]{1,39}$")


@plugin.action("snapshot")
def snapshot(settings):
    vmid = parse_vmid(settings.get("vmid"))
    name = (settings.get("name") or "").strip()
    if not name:
        name = time.strftime("deck_%Y%m%d_%H%M%S")
    if not SNAPSHOT_NAME.match(name):
        raise ProxmoxError("snapshot names start with a letter and use only letters, digits, - and _")
    guest = find_guest(vmid)
    params = {"snapname": name, "description": "Taken from PerritoDeck"}
    api("POST", f"{guest_path(guest)}/snapshot", params)
    return None


# --- live values ---------------------------------------------------------------


def percent(used, total):
    try:
        if not total:
            return NOTHING
        return str(max(0, min(100, round(float(used) * 100 / float(total)))))
    except (TypeError, ValueError):
        return NOTHING


def cpu_percent(entry):
    try:
        return str(max(0, min(100, round(float(entry.get("cpu") or 0) * 100))))
    except (TypeError, ValueError):
        return NOTHING


def human_uptime(seconds):
    try:
        seconds = int(seconds)
    except (TypeError, ValueError):
        return NOTHING
    if seconds <= 0:
        return NOTHING
    days, rest = divmod(seconds, 86400)
    hours, rest = divmod(rest, 3600)
    minutes = rest // 60
    if days:
        return f"{days}d {hours}h"
    if hours:
        return f"{hours}h {minutes}m"
    return f"{minutes}m"


def guest_status(guest):
    """running, stopped or paused. /cluster/resources says "running" for a paused VM,
    so a running VM someone watches status for gets one status/current call."""
    status = guest.get("status") or NOTHING
    if guest.get("type") == "qemu" and status == "running":
        try:
            current = api("GET", f"{guest_path(guest)}/status/current") or {}
            if current.get("qmpstatus") in ("paused", "suspended"):
                return "paused"
        except ProxmoxError:
            pass
    return status


def guest_value(value_id, guest):
    if guest is None:
        return NOTHING
    running = guest.get("status") == "running"
    if value_id == "status":
        return guest_status(guest)
    if value_id == "running":
        return "on" if running else "off"
    if value_id == "cpu":
        return cpu_percent(guest) if running else "0"
    if value_id == "mem":
        return percent(guest.get("mem"), guest.get("maxmem")) if running else "0"
    if value_id == "uptime":
        return human_uptime(guest.get("uptime")) if running else NOTHING
    if value_id == "name":
        return guest.get("name") or NOTHING
    return NOTHING


def node_in(items, name):
    for r in items or []:
        if r.get("type") == "node" and r.get("node") == name:
            return r
    return None


def node_value(value_id, node):
    if node is None:
        return NOTHING
    online = node.get("status") == "online"
    if value_id == "node_status":
        return node.get("status") or NOTHING
    if not online:
        return NOTHING
    if value_id == "node_cpu":
        return cpu_percent(node) if "cpu" in node else NOTHING  # without Sys.Audit no stats
    if value_id == "node_mem":
        return percent(node.get("mem"), node.get("maxmem"))
    return NOTHING


def running_count(items):
    return sum(
        1
        for r in items or []
        if r.get("type") in ("qemu", "lxc") and r.get("status") == "running" and not r.get("template")
    )


def watched_any():
    return any(plugin.watched(v) for v in GUEST_VALUES + NODE_VALUES)


def configured():
    try:
        config()
        return True
    except ProxmoxError:
        return False


def poll():
    if not configured():
        publish_unknown()
        return
    try:
        items = resources()
    except ProxmoxError as e:
        plugin.log(str(e), "warn")
        publish_unknown()
        return
    for value_id in GUEST_VALUES:
        for arg in plugin.watched(value_id):
            plugin.set_value(value_id, guest_value(value_id, guest_in(items, arg.strip())), arg=arg)
    for value_id in NODE_VALUES:
        for arg in plugin.watched(value_id):
            plugin.set_value(value_id, node_value(value_id, node_in(items, arg.strip())), arg=arg)
    plugin.set_value("guests_running", str(running_count(items)))


def publish_unknown():
    for value_id in GUEST_VALUES + NODE_VALUES:
        for arg in plugin.watched(value_id):
            plugin.set_value(value_id, NOTHING, arg=arg)
    plugin.set_value("guests_running", NOTHING)


_poll_lock = threading.Lock()


def safe_poll():
    if not _poll_lock.acquire(blocking=False):
        return  # a poll is already running
    try:
        _last_poll["at"] = time.monotonic()
        poll()
    except Exception as e:  # never let the timer die
        plugin.log(f"poll failed: {e}", "warn")
    finally:
        _poll_lock.release()


@plugin.every(POLL_SECONDS)
def tick():
    # Watched VM/node values: every 5 s. Only the guest count: every 30 s.
    if watched_any() or time.monotonic() - _last_poll["at"] >= IDLE_POLL_SECONDS - 0.5 or not _last_poll["at"]:
        safe_poll()


@plugin.on_settings
def settings_changed(settings):
    forget()
    _last_poll["at"] = 0.0


@plugin.on_watch
def watch_changed(values):
    threading.Thread(target=safe_poll, daemon=True).start()


if __name__ == "__main__":
    plugin.run()
