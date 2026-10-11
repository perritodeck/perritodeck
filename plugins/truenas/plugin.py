"""TrueNAS (Community Edition / SCALE): pools, alerts and apps on keys.

TrueNAS 25.04 deprecated the REST API (/api/v2.0) and TrueNAS 26 removes it, so this
plugin speaks the JSON-RPC 2.0 API over a WebSocket (wss://<nas>/api/current), with a
small WebSocket client written on the standard library. It keeps one logged-in
connection open: TrueNAS allows only 20 logins a minute before a 10-minute cooldown.

API keys only travel over TLS (wss://): TrueNAS revokes a key that is sent over plain
HTTP/ws, so this plugin refuses http:// addresses.
"""

import base64
import hashlib
import json
import os
import re
import socket
import ssl
import struct
import sys
import threading
import time
import urllib.parse

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()
NOTHING = "–"
TIMEOUT = 4  # connecting, logging in and quick calls
JOB_WAIT = 6.0  # jobs answer when they finish; an action waits this long, then says "started"
RETRY_SECONDS = 10  # after a failed connection, don't try again sooner
AUTH_RETRY_SECONDS = 120  # after a refused key: TrueNAS rate-limits logins
TICK_SECONDS = 15  # apps every tick, pools and alerts every other tick
REALTIME_PUBLISH_SECONDS = 5
# Tests run a plain ws:// fake server; real use needs wss://.
ALLOW_PLAIN_WS = False

LEVELS = ["INFO", "NOTICE", "WARNING", "ERROR", "CRITICAL", "ALERT", "EMERGENCY"]
POOL_VALUES = ("pool", "pool_ok", "pool_used")
APP_VALUES = ("app", "app_running")
WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


class TrueNASError(RuntimeError):
    pass


class AuthError(TrueNASError):
    pass


# --- a minimal WebSocket client (RFC 6455) ------------------------------------------------


class WebSocket:
    """Text messages only; enough for JSON-RPC. Not thread-safe for reading: one
    reader thread calls recv(); send() may be called from any thread."""

    def __init__(self, url, insecure=False, timeout=TIMEOUT):
        parsed = urllib.parse.urlparse(url)
        secure = parsed.scheme == "wss"
        host = parsed.hostname
        port = parsed.port or (443 if secure else 80)
        path = (parsed.path or "/") + (("?" + parsed.query) if parsed.query else "")
        sock = socket.create_connection((host, port), timeout=timeout)
        try:
            if secure:
                ctx = ssl.create_default_context()
                if insecure:
                    ctx.check_hostname = False
                    ctx.verify_mode = ssl.CERT_NONE
                sock = ctx.wrap_socket(sock, server_hostname=host)
            self.sock = sock
            self._buf = b""
            self._send_lock = threading.Lock()
            self._handshake(host, port, secure, path)
        except BaseException:
            sock.close()
            raise
        sock.settimeout(None)  # the reader blocks; calls have their own timeouts

    def _handshake(self, host, port, secure, path):
        key = base64.b64encode(os.urandom(16)).decode()
        default_port = 443 if secure else 80
        host_header = host if port == default_port else f"{host}:{port}"
        if ":" in host and not host.startswith("["):  # IPv6 literal
            host_header = f"[{host}]" if port == default_port else f"[{host}]:{port}"
        request = (
            f"GET {path} HTTP/1.1\r\n"
            f"Host: {host_header}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n"
            "User-Agent: PerritoDeck\r\n"
            "\r\n"
        )
        self.sock.sendall(request.encode())
        while b"\r\n\r\n" not in self._buf:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise ConnectionError("the server closed the connection during the handshake")
            self._buf += chunk
            if len(self._buf) > 65536:
                raise ConnectionError("the server's handshake answer is too long")
        head, self._buf = self._buf.split(b"\r\n\r\n", 1)
        lines = head.decode("latin-1").split("\r\n")
        status = lines[0].split(" ", 2)
        if len(status) < 2 or status[1] != "101":
            raise ConnectionError(f"the server answered {' '.join(status[1:]) or lines[0]} instead of a WebSocket")
        headers = {}
        for line in lines[1:]:
            name, _, value = line.partition(":")
            headers[name.strip().lower()] = value.strip()
        expected = base64.b64encode(hashlib.sha1((key + WS_GUID).encode()).digest()).decode()
        if headers.get("sec-websocket-accept") != expected:
            raise ConnectionError("the server's WebSocket handshake is invalid")

    def _read(self, n):
        while len(self._buf) < n:
            chunk = self.sock.recv(max(65536, n - len(self._buf)))
            if not chunk:
                raise ConnectionError("connection closed")
            self._buf += chunk
        data, self._buf = self._buf[:n], self._buf[n:]
        return data

    def _frame(self):
        b0, b1 = self._read(2)
        fin, opcode = b0 & 0x80, b0 & 0x0F
        length = b1 & 0x7F
        if length == 126:
            length = struct.unpack("!H", self._read(2))[0]
        elif length == 127:
            length = struct.unpack("!Q", self._read(8))[0]
        mask = self._read(4) if b1 & 0x80 else None
        payload = self._read(length)
        if mask:
            payload = masked(payload, mask)
        return bool(fin), opcode, payload

    def recv(self):
        """The next text message, as str. Answers pings; raises ConnectionError on close."""
        parts = []
        while True:
            fin, opcode, payload = self._frame()
            if opcode == 0x9:  # ping
                self._send_frame(0xA, payload)
                continue
            if opcode == 0xA:  # pong
                continue
            if opcode == 0x8:  # close
                try:
                    self._send_frame(0x8, payload[:2])
                except OSError:
                    pass
                raise ConnectionError("the server closed the connection")
            parts.append(payload)
            if fin:
                return b"".join(parts).decode("utf-8")

    def _send_frame(self, opcode, payload):
        header = bytes([0x80 | opcode])
        n = len(payload)
        if n < 126:
            header += bytes([0x80 | n])
        elif n < 65536:
            header += bytes([0x80 | 126]) + struct.pack("!H", n)
        else:
            header += bytes([0x80 | 127]) + struct.pack("!Q", n)
        key = os.urandom(4)
        with self._send_lock:
            self.sock.sendall(header + key + masked(payload, key))

    def send(self, text):
        self._send_frame(0x1, text.encode("utf-8"))

    def close(self):
        try:
            self._send_frame(0x8, struct.pack("!H", 1000))
        except OSError:
            pass
        try:
            self.sock.close()
        except OSError:
            pass


def masked(data, key):
    n = len(data)
    if not n:
        return b""
    k = (key * (n // 4 + 1))[:n]
    return (int.from_bytes(data, "big") ^ int.from_bytes(k, "big")).to_bytes(n, "big")


# --- JSON-RPC over the WebSocket ---------------------------------------------------------------


class Connection:
    def __init__(self, ws, on_event=None):
        self.ws = ws
        self.on_event = on_event
        self.alive = True
        self._next_id = 0
        self._pending = {}
        self._lock = threading.Lock()
        threading.Thread(target=self._reader, daemon=True).start()

    def _reader(self):
        try:
            while True:
                message = json.loads(self.ws.recv())
                if not isinstance(message, dict):
                    continue
                if "method" in message:
                    if message.get("method") == "collection_update" and self.on_event:
                        try:
                            self.on_event(message.get("params") or {})
                        except Exception as e:
                            plugin.log(f"event failed: {e}", "warn")
                    continue
                with self._lock:
                    waiter = self._pending.pop(message.get("id"), None)
                if waiter is not None:
                    waiter[1] = message
                    waiter[0].set()
        except Exception:
            pass
        self.close()

    def call(self, method, params=(), timeout=TIMEOUT, job=False):
        """Call a method; return its result. With job=True, return None if the job is
        still running after `timeout` (TrueNAS answers a job when it finishes)."""
        if not self.alive:
            raise TrueNASError("the connection to TrueNAS was lost")
        with self._lock:
            self._next_id += 1
            call_id = self._next_id
            waiter = [threading.Event(), None]
            self._pending[call_id] = waiter
        request = {"jsonrpc": "2.0", "id": call_id, "method": method, "params": list(params)}
        try:
            self.ws.send(json.dumps(request))
        except OSError as e:
            self.close()
            raise TrueNASError(f"the connection to TrueNAS was lost: {e}") from None
        if not waiter[0].wait(timeout):
            if job:
                return None  # still running; its answer will be dropped when it comes
            with self._lock:
                self._pending.pop(call_id, None)
            self.close()  # a quick call that hangs means a dead connection
            raise TrueNASError(f"TrueNAS didn't answer {method} in time")
        message = waiter[1]
        if message is None:
            raise TrueNASError("the connection to TrueNAS was lost")
        if "error" in message:
            raise TrueNASError(describe_rpc_error(method, message["error"]))
        return message.get("result")

    def close(self):
        if not self.alive:
            return
        self.alive = False
        self.ws.close()
        with self._lock:
            waiters, self._pending = list(self._pending.values()), {}
        for waiter in waiters:
            waiter[0].set()  # their message stays None: "connection lost"


def describe_rpc_error(method, error):
    if not isinstance(error, dict):
        return f"TrueNAS couldn't do {method}: {error}"
    data = error.get("data") if isinstance(error.get("data"), dict) else {}
    extra = data.get("extra")
    if isinstance(extra, list) and extra and all(isinstance(e, (list, tuple)) and len(e) >= 2 for e in extra):
        reason = "; ".join(str(e[1]) for e in extra)
    else:
        reason = data.get("reason") or error.get("message") or "unknown error"
    reason = str(reason).strip().splitlines()[0] if str(reason).strip() else "unknown error"
    if data.get("errname") == "ENOTAUTHENTICATED":
        return "TrueNAS says the connection isn't logged in"
    if data.get("errname") == "EACCES" or "not authorized" in reason.lower():
        return f"the API key's user isn't allowed to do that ({reason})"
    return f"TrueNAS couldn't do {method}: {reason}"


# --- the connection everyone shares ----------------------------------------------------------------

_state_lock = threading.Lock()
_conn = {"conn": None, "key": None, "failed_at": 0.0, "error": None, "retry": RETRY_SECONDS}


def config():
    s = plugin.settings or {}
    url = (s.get("url") or "").strip().rstrip("/")
    if not url:
        raise TrueNASError("set the TrueNAS address on the Plugins page")
    if "://" not in url:
        url = "https://" + url
    parsed = urllib.parse.urlparse(url)
    if parsed.scheme in ("http", "ws") and not ALLOW_PLAIN_WS:
        raise TrueNASError(
            "use an https:// address: TrueNAS revokes an API key that is sent without encryption"
        )
    if parsed.scheme not in ("https", "wss", "http", "ws") or not parsed.hostname:
        raise TrueNASError(f"{url} isn't a web address")
    scheme = "wss" if parsed.scheme in ("https", "wss") else "ws"
    path = parsed.path.rstrip("/")
    if not path.startswith("/api/"):
        path = "/api/current"
    ws_url = f"{scheme}://{parsed.netloc}{path}"
    key = (s.get("api_key") or "").strip()
    if not key:
        raise TrueNASError("set an API key on the Plugins page")
    username = (s.get("username") or "").strip()
    return ws_url, username, key, bool(s.get("insecure"))


def plain_reason(reason):
    """"[Errno 111] Connection refused" -> "connection refused"."""
    if isinstance(reason, OSError) and reason.strerror:
        text = reason.strerror
    else:
        text = re.sub(r"^\[Errno -?\d+\]\s*", "", str(reason))
    if "timed out" in text:
        return "no answer (timed out)"
    return text[:1].lower() + text[1:] if text[:2] != text[:2].upper() else text


def connect(ws_url, username, key, insecure):
    display = ws_url.replace("wss://", "https://", 1).replace("ws://", "http://", 1)
    try:
        ws = WebSocket(ws_url, insecure=insecure, timeout=TIMEOUT)
    except ssl.SSLCertVerificationError:
        raise TrueNASError(
            f"couldn't check the certificate of {display}; install a trusted one or turn on \"Skip certificate check\""
        ) from None
    except (OSError, ConnectionError) as e:
        raise TrueNASError(f"couldn't reach TrueNAS at {display}: {plain_reason(e)}") from None
    conn = Connection(ws, on_event=on_event)
    try:
        login(conn, username, key)
    except Exception:
        conn.close()
        raise
    try:
        # Don't wait long: the CPU numbers aren't worth delaying a key press.
        conn.call("core.subscribe", ["reporting.realtime"], timeout=2, job=True)
    except TrueNASError as e:
        plugin.log(f"no live CPU numbers: {e}", "warn")
    return conn


def login(conn, username, key):
    if username:
        # TrueNAS 25.04+: auth.login_ex with the API_KEY_PLAIN mechanism (over TLS).
        try:
            answer = conn.call("auth.login_ex", [{"mechanism": "API_KEY_PLAIN", "username": username, "api_key": key}])
        except TrueNASError as e:
            raise AuthError(f"TrueNAS refused the login: {e}") from None
        kind = (answer or {}).get("response_type") if isinstance(answer, dict) else None
        if kind == "SUCCESS":
            return
        reasons = {
            "AUTH_ERR": "TrueNAS refused the API key: check the key and the user name",
            "EXPIRED": "the API key has expired; make a new one in TrueNAS",
            "DENIED": "that user isn't allowed to use the API",
            "REDIRECT": "log in to the active controller's address instead",
        }
        raise AuthError(reasons.get(kind, f"TrueNAS answered the login with {kind}"))
    try:
        ok = conn.call("auth.login_with_api_key", [key])
    except TrueNASError as e:
        raise AuthError(f"TrueNAS refused the login: {e}") from None
    if ok is not True:
        raise AuthError("TrueNAS refused the API key (on TrueNAS 25.04 and later, also fill in the user name)")


def client():
    """The shared logged-in connection; connects (at most every RETRY_SECONDS) if needed."""
    settings_key = config()
    with _state_lock:
        conn = _conn["conn"]
        if conn is not None and conn.alive and _conn["key"] == settings_key:
            return conn
        if conn is not None:
            conn.close()
            _conn["conn"] = None
        if _conn["key"] == settings_key and _conn["error"] and time.monotonic() - _conn["failed_at"] < _conn["retry"]:
            raise TrueNASError(_conn["error"])
        try:
            conn = connect(*settings_key)
        except TrueNASError as e:
            _conn.update(key=settings_key, failed_at=time.monotonic(), error=str(e),
                         retry=AUTH_RETRY_SECONDS if isinstance(e, AuthError) else RETRY_SECONDS)
            raise
        _conn.update(conn=conn, key=settings_key, error=None, failed_at=0.0)
        return conn


def disconnect():
    with _state_lock:
        if _conn["conn"] is not None:
            _conn["conn"].close()
        _conn.update(conn=None, key=None, error=None, failed_at=0.0)


def rpc(method, *params, timeout=TIMEOUT, job=False):
    return client().call(method, params, timeout=timeout, job=job)


# --- actions --------------------------------------------------------------------------------------


def app_name(settings):
    name = (settings.get("app") or "").strip()
    if not name:
        raise TrueNASError("enter the app's name, as TrueNAS shows it (e.g. jellyfin)")
    return name


def app_state(name):
    found = rpc("app.query", [["name", "=", name]])
    if not found:
        raise TrueNASError(f"there's no app called {name}")
    return (found[0].get("state") or "").upper()


def refresh_apps_soon():
    timer = threading.Timer(2.0, lambda: safe_poll(apps=True, pools=False))
    timer.daemon = True
    timer.start()


@plugin.action("start-app")
def start_app(settings):
    name = app_name(settings)
    rpc("app.start", name, timeout=JOB_WAIT, job=True)
    refresh_apps_soon()


@plugin.action("stop-app")
def stop_app(settings):
    name = app_name(settings)
    rpc("app.stop", name, timeout=JOB_WAIT, job=True)
    refresh_apps_soon()


@plugin.action("toggle-app")
def toggle_app(settings):
    name = app_name(settings)
    state = app_state(name)
    method = "app.stop" if state in ("RUNNING", "DEPLOYING") else "app.start"
    rpc(method, name, timeout=JOB_WAIT, job=True)
    refresh_apps_soon()


@plugin.action("restart-app")
def restart_app(settings):
    """Stop, then start: app.redeploy would also pull newer images."""
    name = app_name(settings)
    conn = client()
    outcome = {}

    def work():
        try:
            conn.call("app.stop", [name], timeout=300, job=True)
            conn.call("app.start", [name], timeout=300, job=True)
        except Exception as e:
            outcome["error"] = e
            plugin.log(f"restarting {name}: {e}", "warn")
        refresh_apps_soon()

    thread = threading.Thread(target=work, daemon=True)
    thread.start()
    thread.join(JOB_WAIT)
    if "error" in outcome:
        raise outcome["error"]


@plugin.action("scrub")
def scrub(settings):
    pool = (settings.get("pool") or "").strip()
    if not pool:
        raise TrueNASError("enter the pool's name (e.g. tank)")
    rpc("pool.scrub.scrub", pool, "START", timeout=JOB_WAIT, job=True)


@plugin.action("dismiss-alerts")
def dismiss_alerts(settings):
    active = [a for a in rpc("alert.list") or [] if not a.get("dismissed")]
    if not active:
        return "there are no alerts to dismiss"
    for alert in active:
        rpc("alert.dismiss", alert["uuid"])
    safe_poll(apps=False, pools=True)
    return None


TASK_METHODS = {"cloudsync": "cloudsync.sync", "replication": "replication.run"}


@plugin.action("run-task")
def run_task(settings):
    kind = settings.get("kind") or "cloudsync"
    method = TASK_METHODS.get(kind)
    if method is None:
        raise TrueNASError(f"unknown task type {kind}")
    task_id = str(settings.get("task_id") or "").strip()
    if not task_id.isdigit():
        raise TrueNASError("enter the task's number (its id in TrueNAS)")
    rpc(method, int(task_id), timeout=JOB_WAIT, job=True)


# --- live values ---------------------------------------------------------------------------------------

_realtime = {"cpu": None, "temp": None, "published_at": 0.0}


def number(x):
    try:
        return None if x is None else float(x)
    except (TypeError, ValueError):
        return None


def on_event(params):
    if params.get("collection") != "reporting.realtime":
        return
    cpu = ((params.get("fields") or {}).get("cpu") or {})
    total = cpu.get("cpu") if isinstance(cpu.get("cpu"), dict) else cpu.get("average") or {}
    usage, temp = number(total.get("usage")), number(total.get("temp"))
    _realtime["cpu"], _realtime["temp"] = usage, temp
    now = time.monotonic()
    if now - _realtime["published_at"] >= REALTIME_PUBLISH_SECONDS:
        _realtime["published_at"] = now
        plugin.set_value("cpu", NOTHING if usage is None else str(round(usage)))
        plugin.set_value("temp", NOTHING if temp is None else str(round(temp)))


def pool_values(pool):
    if pool is None:
        return dict.fromkeys(POOL_VALUES, NOTHING)
    size, used = number(pool.get("size")), number(pool.get("allocated"))
    return {
        "pool": pool.get("status") or NOTHING,
        "pool_ok": "on" if pool.get("healthy") and pool.get("status") == "ONLINE" else "off",
        "pool_used": str(round(used * 100 / size)) if size and used is not None else NOTHING,
    }


def highest_level(alerts):
    best = -1
    for a in alerts:
        level = (a.get("level") or "").upper()
        best = max(best, LEVELS.index(level) if level in LEVELS else 0)
    return LEVELS[best] if best >= 0 else "none"


def poll_pools_and_alerts():
    alerts = [a for a in rpc("alert.list") or [] if not a.get("dismissed")]
    plugin.set_value("alerts", str(len(alerts)))
    plugin.set_value("alert_level", highest_level(alerts))
    watched = {v: plugin.watched(v) for v in POOL_VALUES}
    if any(watched.values()):
        pools = {p.get("name"): p for p in rpc("pool.query") or []}
        for value_id, args in watched.items():
            for arg in args:
                plugin.set_value(value_id, pool_values(pools.get(arg.strip()))[value_id], arg=arg)


def poll_apps():
    watched = {v: plugin.watched(v) for v in APP_VALUES}
    names = sorted({a.strip() for args in watched.values() for a in args})
    if not names:
        return
    apps = {a.get("name"): a for a in rpc("app.query", [["name", "in", names]]) or []}
    for value_id, args in watched.items():
        for arg in args:
            app = apps.get(arg.strip())
            state = (app.get("state") or "").lower() if app else ""
            if value_id == "app":
                plugin.set_value("app", state or NOTHING, arg=arg)
            else:
                plugin.set_value("app_running", ("on" if state == "running" else "off") if app else NOTHING, arg=arg)


def publish_unknown(apps=True, pools=True):
    ids = (APP_VALUES if apps else ()) + (POOL_VALUES if pools else ())
    for value_id in ids:
        for arg in plugin.watched(value_id):
            plugin.set_value(value_id, NOTHING, arg=arg)
    if pools:
        for value_id in ("alerts", "alert_level", "cpu", "temp"):
            plugin.set_value(value_id, NOTHING)


_poll_lock = threading.Lock()
_ticks = {"n": 0}
_last_error = {"text": None}


def safe_poll(apps=True, pools=True):
    with _poll_lock:
        try:
            if pools:
                poll_pools_and_alerts()
            if apps:
                poll_apps()
            _last_error["text"] = None
        except TrueNASError as e:
            if _last_error["text"] != str(e):  # say it once, not every 15 s
                plugin.log(str(e), "warn")
                _last_error["text"] = str(e)
            publish_unknown(apps, pools)
        except Exception as e:
            plugin.log(f"poll failed: {e}", "warn")


@plugin.every(TICK_SECONDS)
def tick():
    _ticks["n"] += 1
    safe_poll(apps=True, pools=_ticks["n"] % 2 == 1)


@plugin.on_settings
def settings_changed(settings):
    disconnect()


@plugin.on_watch
def watch_changed(values):
    threading.Thread(target=safe_poll, daemon=True).start()


if __name__ == "__main__":
    plugin.run()
