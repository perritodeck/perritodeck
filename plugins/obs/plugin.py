"""OBS Studio over obs-websocket 5 (built into OBS 28 and newer). Standard library only.

One WebSocket connection stays open on a background thread and reconnects by itself.
OBS's events keep the live values current; only the statistics and timecodes are
polled. Protocol reference:
https://github.com/obsproject/obs-websocket/blob/master/docs/generated/protocol.md
"""

import base64
import datetime
import hashlib
import itertools
import json
import os
import queue
import re
import socket
import ssl
import struct
import sys
import threading
import time
import traceback
from urllib.parse import urlsplit

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()

DEFAULT_URL = "ws://127.0.0.1:4455"
TIMEOUT = 5.0  # one request
CONNECT_TIMEOUT = 4.0  # TCP connect, WebSocket handshake, Hello and Identify
CONNECT_WAIT = 3.0  # how long an action waits for a connection that is (re)starting
ACTION_DEADLINE = 8.0  # actions with several requests answer within the host's 10 s
STILL_WORKING = "OBS is still working on it…"
BACKOFF_FIRST = 0.5
BACKOFF_MAX = 10.0
MAX_MESSAGE = 8 << 20  # obs-websocket answers are small; refuse anything bigger
RPC_VERSION = 1
UNKNOWN = "–"
WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

# EventSubscription bits. Everything except Vendors, Canvases and the high-volume ones.
SUB_GENERAL, SUB_CONFIG, SUB_SCENES, SUB_INPUTS = 1 << 0, 1 << 1, 1 << 2, 1 << 3
SUB_TRANSITIONS, SUB_FILTERS, SUB_OUTPUTS, SUB_SCENE_ITEMS = 1 << 4, 1 << 5, 1 << 6, 1 << 7
SUB_MEDIA_INPUTS, SUB_UI = 1 << 8, 1 << 10
EVENT_SUBSCRIPTIONS = (
    SUB_GENERAL | SUB_CONFIG | SUB_SCENES | SUB_INPUTS | SUB_TRANSITIONS | SUB_FILTERS
    | SUB_OUTPUTS | SUB_SCENE_ITEMS | SUB_MEDIA_INPUTS | SUB_UI
)

# WebSocketCloseCode
CLOSE_AUTH_FAILED = 4009
CLOSE_UNSUPPORTED_RPC = 4010
CLOSE_SESSION_INVALIDATED = 4011

# RequestStatus codes this plugin explains in plain words.
UNKNOWN_REQUEST_TYPE = 204
OUTPUT_RUNNING, OUTPUT_NOT_RUNNING, OUTPUT_PAUSED, OUTPUT_NOT_PAUSED = 500, 501, 502, 503
OUTPUT_DISABLED, STUDIO_MODE_ACTIVE, STUDIO_MODE_NOT_ACTIVE = 504, 505, 506
RESOURCE_NOT_FOUND = 600
FRIENDLY = {
    UNKNOWN_REQUEST_TYPE: "this version of OBS doesn't support that; update OBS",
    OUTPUT_RUNNING: "it's already running",
    OUTPUT_NOT_RUNNING: "it isn't running",
    OUTPUT_PAUSED: "the recording is already paused",
    OUTPUT_NOT_PAUSED: "the recording isn't paused",
    OUTPUT_DISABLED: "it's turned off in OBS's settings",
    STUDIO_MODE_ACTIVE: "Studio Mode is on",
    STUDIO_MODE_NOT_ACTIVE: "Studio Mode is off",
}

# ---------------------------------------------------------------------------
# WebSocket client (RFC 6455): handshake, masked frames, fragments, ping/pong, close.
# ---------------------------------------------------------------------------

OP_CONT, OP_TEXT, OP_BINARY, OP_CLOSE, OP_PING, OP_PONG = 0x0, 0x1, 0x2, 0x8, 0x9, 0xA


class WebSocketClosed(ConnectionError):
    def __init__(self, code=None, reason=""):
        super().__init__(f"connection closed ({code}{': ' + reason if reason else ''})")
        self.code = code
        self.reason = reason


def mask_bytes(data, key):
    """XOR data with the 4-byte key, repeated (RFC 6455 5.3)."""
    n = len(data)
    if not n:
        return b""
    k = (key * (n // 4 + 1))[:n]
    return (int.from_bytes(data, "big") ^ int.from_bytes(k, "big")).to_bytes(n, "big")


def encode_frame(opcode, payload, fin=True, mask=True):
    head = bytearray([(0x80 if fin else 0) | opcode])
    n = len(payload)
    bit = 0x80 if mask else 0
    if n < 126:
        head.append(bit | n)
    elif n < 65536:
        head.append(bit | 126)
        head += struct.pack("!H", n)
    else:
        head.append(bit | 127)
        head += struct.pack("!Q", n)
    if mask:
        key = os.urandom(4)
        head += key
        payload = mask_bytes(payload, key)
    return bytes(head) + payload


def parse_url(url):
    """'ws://host:port/path' -> (secure, host, port, path). The scheme and port are optional."""
    url = (url or "").strip() or DEFAULT_URL
    if "://" not in url:
        url = "ws://" + url
    parts = urlsplit(url)
    if parts.scheme not in ("ws", "wss"):
        raise ValueError(f"the OBS address must start with ws:// (not {parts.scheme}://)")
    if not parts.hostname:
        raise ValueError(f"the OBS address {url} has no computer name")
    secure = parts.scheme == "wss"
    try:
        port = parts.port or (443 if secure else 4455)
    except ValueError:
        raise ValueError(f"the port in {url} isn't a number") from None
    path = parts.path or "/"
    if parts.query:
        path += "?" + parts.query
    return secure, parts.hostname, port, path


class WebSocket:
    def __init__(self, sock):
        self.sock = sock
        self._buf = bytearray()
        self._send_lock = threading.Lock()
        self.protocol = None

    @classmethod
    def connect(cls, url, timeout=CONNECT_TIMEOUT, insecure=False, protocol="obswebsocket.json"):
        secure, host, port, path = parse_url(url)
        sock = socket.create_connection((host, port), timeout=timeout)
        try:
            sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            sock.setsockopt(socket.SOL_SOCKET, socket.SO_KEEPALIVE, 1)
            if secure:
                ctx = ssl.create_default_context()
                if insecure:
                    ctx.check_hostname = False
                    ctx.verify_mode = ssl.CERT_NONE
                sock = ctx.wrap_socket(sock, server_hostname=host)
            ws = cls(sock)
            ws._handshake(host, port, path, protocol, secure)
            return ws
        except BaseException:
            sock.close()
            raise

    def _handshake(self, host, port, path, protocol, secure):
        key = base64.b64encode(os.urandom(16)).decode()
        name = f"[{host}]" if ":" in host else host
        if port != (443 if secure else 80):
            name += f":{port}"
        request = (
            f"GET {path} HTTP/1.1\r\n"
            f"Host: {name}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n"
            f"Sec-WebSocket-Protocol: {protocol}\r\n"
            "User-Agent: PerritoDeck\r\n\r\n"
        )
        self.sock.sendall(request.encode())
        while b"\r\n\r\n" not in self._buf:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise ConnectionError("the server closed the connection during the WebSocket handshake")
            self._buf += chunk
            if len(self._buf) > 65536:
                raise ConnectionError("that isn't a WebSocket server (the answer was too long)")
        end = self._buf.index(b"\r\n\r\n")
        head = bytes(self._buf[:end]).decode("iso-8859-1")
        del self._buf[: end + 4]
        lines = head.split("\r\n")
        status = lines[0].split(" ", 2)
        if len(status) < 2 or status[1] != "101":
            raise ConnectionError(f"that isn't a WebSocket server (it answered: {lines[0][:80]})")
        headers = {}
        for line in lines[1:]:
            name_, _, value = line.partition(":")
            headers[name_.strip().lower()] = value.strip()
        expected = base64.b64encode(hashlib.sha1((key + WS_GUID).encode()).digest()).decode()
        if headers.get("sec-websocket-accept") != expected:
            raise ConnectionError("the WebSocket handshake failed (bad Sec-WebSocket-Accept)")
        self.protocol = headers.get("sec-websocket-protocol")

    def settimeout(self, seconds):
        self.sock.settimeout(seconds)

    def _read(self, n):
        while len(self._buf) < n:
            chunk = self.sock.recv(max(65536, n - len(self._buf)))
            if not chunk:
                raise WebSocketClosed(1006, "connection lost")
            self._buf += chunk
        out = bytes(self._buf[:n])
        del self._buf[:n]
        return out

    def _read_frame(self):
        b0, b1 = self._read(2)
        fin, opcode = bool(b0 & 0x80), b0 & 0x0F
        if b0 & 0x70:
            raise WebSocketClosed(1002, "the server set reserved frame bits")
        if b1 & 0x80:
            raise WebSocketClosed(1002, "the server sent a masked frame")  # RFC 6455 5.1
        n = b1 & 0x7F
        if opcode & 0x8:  # control frames: never fragmented, at most 125 bytes (5.5)
            if not fin:
                raise WebSocketClosed(1002, "a fragmented control frame")
            if n > 125:
                raise WebSocketClosed(1002, "a control frame longer than 125 bytes")
        if n == 126:
            (n,) = struct.unpack("!H", self._read(2))
        elif n == 127:
            (n,) = struct.unpack("!Q", self._read(8))
        if n > MAX_MESSAGE:
            raise WebSocketClosed(1009, "message too big")
        return fin, opcode, self._read(n)

    def send_frame(self, opcode, payload, fin=True):
        frame = encode_frame(opcode, payload, fin=fin)
        with self._send_lock:
            self.sock.sendall(frame)

    def send_text(self, text, fragment=None):
        """Send a text message, split into frames of `fragment` bytes if given."""
        data = text.encode("utf-8")
        if not fragment or len(data) <= fragment:
            self.send_frame(OP_TEXT, data)
            return
        chunks = [data[i : i + fragment] for i in range(0, len(data), fragment)]
        frames = b"".join(
            encode_frame(OP_TEXT if i == 0 else OP_CONT, c, fin=i == len(chunks) - 1) for i, c in enumerate(chunks)
        )
        with self._send_lock:  # keep the fragments together
            self.sock.sendall(frames)

    def recv(self):
        """The next text message. Answers pings; raises WebSocketClosed on close."""
        parts, kind, size = [], None, 0
        while True:
            fin, opcode, payload = self._read_frame()
            if opcode == OP_PING:
                self.send_frame(OP_PONG, payload)
                continue
            if opcode == OP_PONG:
                continue
            if opcode == OP_CLOSE:
                code = struct.unpack("!H", payload[:2])[0] if len(payload) >= 2 else 1005
                reason = payload[2:].decode("utf-8", "replace")
                try:
                    self.send_frame(OP_CLOSE, payload[:2])
                except OSError:
                    pass
                raise WebSocketClosed(code, reason)
            if opcode in (OP_TEXT, OP_BINARY):
                if kind is not None:
                    raise WebSocketClosed(1002, "a new message started inside a fragmented one")
                kind, parts = opcode, [payload]
            elif opcode == OP_CONT:
                if kind is None:
                    raise WebSocketClosed(1002, "a continuation frame came with no message")
                parts.append(payload)
            else:
                raise WebSocketClosed(1002, f"unknown frame type {opcode}")
            size += len(payload)
            if size > MAX_MESSAGE:
                raise WebSocketClosed(1009, "message too big")
            if fin:
                return b"".join(parts).decode("utf-8")

    def close(self, code=1000):
        try:
            self.send_frame(OP_CLOSE, struct.pack("!H", code))
        except OSError:
            pass
        self.abort()

    def abort(self):
        """Drop the connection now; a thread blocked in recv() wakes with an error."""
        try:
            self.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        try:
            self.sock.close()
        except OSError:
            pass


# ---------------------------------------------------------------------------
# obs-websocket 5 client
# ---------------------------------------------------------------------------


def auth_string(password, salt, challenge):
    """base64(sha256(base64(sha256(password + salt)) + challenge))"""
    secret = base64.b64encode(hashlib.sha256((password + salt).encode()).digest())
    return base64.b64encode(hashlib.sha256(secret + challenge.encode()).digest()).decode()


class ObsError(RuntimeError):
    """OBS answered a request with a failure."""

    def __init__(self, request_type, code, comment=None):
        self.request_type, self.code, self.comment = request_type, code, comment
        if comment:
            text = f"OBS: {comment}"
        elif code in FRIENDLY:
            text = f"OBS: {FRIENDLY[code]}"
        else:
            text = f"OBS refused {request_type} (error {code})"
        super().__init__(text)


class NotConnected(RuntimeError):
    pass


class AuthError(ConnectionError):
    pass


SETTINGS_HINT = "(in OBS: Tools → WebSocket Server Settings)"


def describe_connect_error(url, e):
    if isinstance(e, socket.timeout):
        return f"couldn't reach OBS at {url}: no answer. Is OBS open, with its WebSocket server on {SETTINGS_HINT}?"
    if isinstance(e, ConnectionRefusedError):
        return (
            f"couldn't reach OBS at {url}: connection refused. Is OBS open, "
            f"with its WebSocket server on {SETTINGS_HINT}?"
        )
    if isinstance(e, socket.gaierror):
        return f"couldn't find the computer in {url}: {e.strerror or e}"
    if isinstance(e, ssl.SSLError):
        return f"couldn't make a secure connection to {url}: {getattr(e, 'reason', None) or e}"
    reason = getattr(e, "strerror", None) or str(e) or type(e).__name__
    return f"couldn't reach OBS at {url}: {reason}"


class _Slot:
    def __init__(self):
        self.done = threading.Event()
        self.response = None
        self.error = None


class Obs:
    """One persistent obs-websocket connection with automatic reconnection."""

    def __init__(self, on_event=None, on_connect=None, on_disconnect=None, log=None):
        self.on_event = on_event
        self.on_connect = on_connect
        self.on_disconnect = on_disconnect
        self.log = log or (lambda message, level="info": None)
        self._lock = threading.Lock()
        self._wake = threading.Event()
        self._up = threading.Event()
        self._config = None
        self._gen = 0
        self._ws = None
        self._pending = {}
        self._ids = itertools.count(1)
        self._thread = None
        self._stopped = False
        self._exiting = False
        self.last_error = "connecting to OBS…"
        self.hello = {}

    # -- settings and lifecycle --

    def configure(self, url, password="", insecure=False):
        url = (url or "").strip() or DEFAULT_URL
        if "://" not in url:
            url = "ws://" + url
        config = (url, password or "", bool(insecure))
        with self._lock:
            if config == self._config:
                return
            self._config = config
            self._gen += 1
            ws = self._ws
            self.last_error = "connecting to OBS…"
        if ws is not None:
            ws.close()
        self._wake.set()
        self._start()

    @property
    def url(self):
        return self._config[0] if self._config else DEFAULT_URL

    @property
    def connected(self):
        return self._ws is not None

    def _start(self):
        with self._lock:
            if self._thread is None and not self._stopped:
                self._thread = threading.Thread(target=self._run, name="obs-connection", daemon=True)
                self._thread.start()

    def stop(self):
        self._stopped = True
        ws = self._ws
        if ws is not None:
            ws.close()
        self._wake.set()

    def _run(self):
        delay = BACKOFF_FIRST
        while not self._stopped:
            with self._lock:
                config, gen = self._config, self._gen
            self._wake.clear()
            outcome = self._session(config, gen)
            if self._stopped:
                break
            if outcome == "connected":
                delay = BACKOFF_FIRST
            elif outcome == "stale":
                continue
            elif outcome == "kicked":
                self._wake.wait()  # OBS asked us not to reconnect by ourselves
                continue
            if self._wake.wait(delay):
                continue  # settings changed or an action wants OBS now
            delay = min(delay * 2, BACKOFF_MAX)

    def _set_error(self, gen, message):
        with self._lock:
            if gen != self._gen or message == self.last_error:
                return
            self.last_error = message
        self.log(message, "warn")  # only when it changes, so retries don't flood the log

    def _session(self, config, gen):
        url, password, insecure = config
        try:
            ws = WebSocket.connect(url, timeout=CONNECT_TIMEOUT, insecure=insecure)
        except ValueError as e:
            self._set_error(gen, str(e))
            return "error"
        except (OSError, ConnectionError) as e:
            self._set_error(gen, describe_connect_error(url, e))
            return "error"
        try:
            self._identify(ws, password)
        except AuthError as e:
            ws.abort()
            self._set_error(gen, str(e))
            return "error"
        except (OSError, ConnectionError, ValueError) as e:
            ws.abort()
            if isinstance(e, socket.timeout):
                message = f"OBS at {url} didn't greet PerritoDeck; it needs OBS 28 or newer (obs-websocket 5)"
            else:
                message = describe_connect_error(url, e)
            self._set_error(gen, message)
            return "error"
        with self._lock:
            if gen != self._gen or self._stopped:
                ws.close()
                return "stale"
            self._ws = ws
            self._exiting = False
            self.last_error = ""
        self._up.set()
        self.log(f"connected to OBS {self.hello.get('obsStudioVersion', '?')} at {url}")
        if self.on_connect:
            # Requests need this thread to read their answers, so sync elsewhere.
            threading.Thread(target=self._safe, args=(self.on_connect,), daemon=True).start()
        closed = None
        try:
            while True:
                text = ws.recv()
                try:
                    message = json.loads(text)
                except ValueError:
                    continue
                self._dispatch(message)
        except WebSocketClosed as e:
            closed = e.code
        except (OSError, ValueError):
            pass
        finally:
            with self._lock:
                if self._ws is ws:
                    self._ws = None
                    self._up.clear()
                pending, self._pending = self._pending, {}
            ws.abort()
            for slot in pending.values():
                slot.error = "lost the connection to OBS"
                slot.done.set()
            if self.on_disconnect:
                self._safe(self.on_disconnect)
        if closed == CLOSE_SESSION_INVALIDATED:
            self._set_error(gen, "OBS disconnected PerritoDeck (from its session list); press a key to reconnect")
            return "kicked"
        if self._exiting:
            self._set_error(gen, "OBS was closed")
        else:
            self._set_error(gen, f"lost the connection to OBS at {url}; reconnecting")
        return "connected"

    def _safe(self, fn, *args):
        try:
            fn(*args)
        except Exception:
            traceback.print_exc(file=sys.stderr)

    def _identify(self, ws, password):
        ws.settimeout(CONNECT_TIMEOUT)
        hello = json.loads(ws.recv())
        if hello.get("op") != 0:
            raise ConnectionError("that isn't obs-websocket 5 (it didn't say Hello)")
        data = hello.get("d") or {}
        identify = {"rpcVersion": RPC_VERSION, "eventSubscriptions": EVENT_SUBSCRIPTIONS}
        auth = data.get("authentication")
        if auth:
            if not password:
                raise AuthError(f"OBS needs a password: enter it in this plugin's settings {SETTINGS_HINT}")
            identify["authentication"] = auth_string(password, auth.get("salt", ""), auth.get("challenge", ""))
        ws.send_text(json.dumps({"op": 1, "d": identify}))
        try:
            answer = json.loads(ws.recv())
        except WebSocketClosed as e:
            if e.code == CLOSE_AUTH_FAILED:
                raise AuthError(f"OBS rejected the password: check it in this plugin's settings {SETTINGS_HINT}") from None
            if e.code == CLOSE_UNSUPPORTED_RPC:
                raise ConnectionError("this OBS speaks a different obs-websocket version; update OBS") from None
            raise ConnectionError(f"OBS closed the connection ({e.code}{': ' + e.reason if e.reason else ''})") from None
        if answer.get("op") != 2:
            raise ConnectionError("OBS didn't accept the connection")
        self.hello = data
        ws.settimeout(None)

    def _dispatch(self, message):
        op, d = message.get("op"), message.get("d") or {}
        if op in (7, 9):
            with self._lock:
                slot = self._pending.pop(d.get("requestId"), None)
            if slot is not None:
                slot.response = d
                slot.done.set()
        elif op == 5:
            event = d.get("eventType")
            if event == "ExitStarted":
                self._exiting = True
            if self.on_event:
                try:
                    self.on_event(event, d.get("eventData") or {})
                except Exception:
                    traceback.print_exc(file=sys.stderr)

    # -- requests --

    def _connection(self, wait):
        ws = self._ws
        if ws is not None:
            return ws
        if wait > 0:  # an action wants OBS now: skip the reconnect backoff
            self._wake.set()
            self._start()
            self._up.wait(wait)
        ws = self._ws
        if ws is None:
            raise NotConnected(self.last_error or "OBS isn't connected")
        return ws

    def _exchange(self, request_id, message, timeout, wait, drop_on_timeout):
        ws = self._connection(wait)
        slot = _Slot()
        with self._lock:
            self._pending[request_id] = slot
        try:
            ws.send_text(json.dumps(message))
        except OSError:
            with self._lock:
                self._pending.pop(request_id, None)
            ws.abort()
            raise NotConnected("lost the connection to OBS") from None
        if not slot.done.wait(timeout):
            with self._lock:
                self._pending.pop(request_id, None)
            if drop_on_timeout:
                self.log("OBS stopped answering; reconnecting", "warn")
                ws.abort()
            raise TimeoutError("OBS didn't answer in time")
        if slot.error:
            raise NotConnected(slot.error)
        return slot.response

    def call(self, request_type, data=None, timeout=TIMEOUT, wait=None, drop_on_timeout=False):
        """Send one request; return its responseData ({} if none) or raise ObsError."""
        request_id = f"pd-{next(self._ids)}"
        d = {"requestType": request_type, "requestId": request_id}
        if data is not None:
            d["requestData"] = data
        response = self._exchange(
            request_id, {"op": 6, "d": d}, timeout, CONNECT_WAIT if wait is None else wait, drop_on_timeout
        )
        status = response.get("requestStatus") or {}
        if not status.get("result"):
            raise ObsError(request_type, status.get("code"), status.get("comment"))
        return response.get("responseData") or {}

    def batch(self, requests, timeout=TIMEOUT, wait=None, drop_on_timeout=False):
        """Send [(requestType, requestData), …] as one RequestBatch. Returns a list of
        responseData dicts, or ObsError instances for the requests that failed."""
        if not requests:
            return []
        request_id = f"pd-{next(self._ids)}"
        items = [
            {"requestType": t, "requestId": str(i), "requestData": data or {}} for i, (t, data) in enumerate(requests)
        ]
        message = {"op": 8, "d": {"requestId": request_id, "haltOnFailure": False, "requests": items}}
        response = self._exchange(request_id, message, timeout, 0 if wait is None else wait, drop_on_timeout)
        results = [ObsError(t, None, "no answer") for t, _ in requests]
        for position, r in enumerate(response.get("results") or []):
            try:
                index = int(r.get("requestId", position))
            except (TypeError, ValueError):
                index = position
            if not 0 <= index < len(results):
                continue
            status = r.get("requestStatus") or {}
            if status.get("result"):
                results[index] = r.get("responseData") or {}
            else:
                results[index] = ObsError(requests[index][0], status.get("code"), status.get("comment"))
        return results


# ---------------------------------------------------------------------------
# Live values
# ---------------------------------------------------------------------------


def slider_to_mul(value):
    """0–100 slider -> OBS volume multiplier, perceptual: (v/100)^2 (like the host)."""
    v = min(max(float(value), 0.0), 100.0)
    return round((v / 100.0) ** 2, 6)


def mul_to_slider(mul):
    return int(round(min(max(float(mul), 0.0), 1.0) ** 0.5 * 100))


def onoff(flag):
    if flag is None:
        return UNKNOWN
    return "on" if flag else "off"


def number(x, decimals=1):
    if x is None:
        return UNKNOWN
    text = f"{float(x):.{decimals}f}"
    if "." in text:
        text = text.rstrip("0").rstrip(".")
    return "0" if text in ("-0", "") else text


def timecode(tc):
    """'01:02:03.456' -> '01:02:03'."""
    return (tc or "00:00:00").split(".")[0]


def split_pair(arg):
    """'Scene/Source' (or 'Scene|Source' when a name has a '/') -> ('Scene', 'Source')."""
    sep = "|" if "|" in arg else "/"
    first, _, second = arg.partition(sep)
    return first.strip(), second.strip()


FIXED_VALUES = [
    "connected", "scene", "preview_scene", "studio_mode", "streaming", "recording", "record_paused",
    "replay_buffer", "virtualcam", "transition", "profile", "scene_collection", "stream_time",
    "record_time", "fps", "cpu", "memory", "dropped_frames", "bitrate",
]
ARG_VALUES = [
    "scene_active", "scene_preview", "muted", "volume", "volume_db", "item_visible", "filter_enabled",
    "input_text", "media_playing",
]


class Live:
    """What we know about OBS, kept current by events, published as plugin values."""

    def __init__(self, obs, publish, watched, log=None, clock=time.monotonic):
        self.obs = obs
        self.publish_raw = publish
        self.watched = watched
        self.log = log or (lambda message, level="info": None)
        self.clock = clock
        self.lock = threading.RLock()
        self.published = {}
        self.jobs = queue.Queue()
        self.queued = set()
        self.transition_ended = threading.Event()
        self.worker = threading.Thread(target=self._work, name="obs-sync", daemon=True)
        self.worker.start()
        self.reset()

    def reset(self):
        with self.lock:
            self.s = {k: None for k in ("program", "preview", "studio", "streaming", "recording", "paused",
                                         "replay", "virtualcam", "transition", "profile", "collection",
                                         "stream_time", "record_time", "fps", "cpu", "memory", "dropped",
                                         "bitrate")}
            self.available = None
            self.muted, self.volume, self.texts, self.media = {}, {}, {}, {}
            self.items, self.filters = {}, {}  # arg -> bool/None
            self.item_keys, self.filter_keys = {}, {}  # (scene, item id) / (source, filter) -> {arg}
            self.last_bytes = None
            self.changing = False

    # -- background jobs (anything that sends requests runs here, never on the reader) --

    def schedule(self, name, fn):
        with self.lock:
            if name in self.queued:
                return
            self.queued.add(name)
        self.jobs.put((name, fn))

    def _work(self):
        while True:
            name, fn = self.jobs.get()
            with self.lock:
                self.queued.discard(name)
            try:
                fn()
            except (NotConnected, TimeoutError, ObsError) as e:
                self.log(f"couldn't update from OBS: {e}", "info")
            except Exception:
                traceback.print_exc(file=sys.stderr)
            finally:
                self.jobs.task_done()

    # -- publishing --

    def _put(self, full_id, value):
        value = str(value)
        if self.published.get(full_id) == value:
            return
        self.published[full_id] = value
        self.publish_raw(full_id, value)

    def fixed_values(self):
        s, up = self.s, self.obs.connected
        if not up:
            return {k: ("off" if k == "connected" else UNKNOWN) for k in FIXED_VALUES}
        studio = s["studio"]
        return {
            "connected": "on",
            "scene": s["program"] or UNKNOWN,
            "preview_scene": (s["preview"] or UNKNOWN) if studio else UNKNOWN,
            "studio_mode": onoff(studio),
            "streaming": onoff(s["streaming"]),
            "recording": onoff(s["recording"]),
            "record_paused": onoff(s["paused"]),
            "replay_buffer": onoff(s["replay"]),
            "virtualcam": onoff(s["virtualcam"]),
            "transition": s["transition"] or UNKNOWN,
            "profile": s["profile"] or UNKNOWN,
            "scene_collection": s["collection"] or UNKNOWN,
            "stream_time": s["stream_time"] or UNKNOWN,
            "record_time": s["record_time"] or UNKNOWN,
            "fps": number(s["fps"]),
            "cpu": number(s["cpu"]),
            "memory": number(s["memory"], 0),
            "dropped_frames": number(s["dropped"]),
            "bitrate": number(s["bitrate"], 0),
        }

    def arg_value(self, value_id, arg):
        if not self.obs.connected:
            return UNKNOWN
        s = self.s
        if value_id == "scene_active":
            return UNKNOWN if s["program"] is None else onoff(s["program"] == arg)
        if value_id == "scene_preview":
            if s["studio"] is False:
                return "off"
            return UNKNOWN if s["preview"] is None else onoff(s["preview"] == arg)
        if value_id == "muted":
            return onoff(self.muted.get(arg))
        if value_id in ("volume", "volume_db"):
            vol = self.volume.get(arg)
            if vol is None:
                return UNKNOWN
            if value_id == "volume":
                return str(mul_to_slider(vol[0]))
            db = vol[1]
            return "-inf" if db is None or db <= -100 else number(db)
        if value_id == "item_visible":
            return onoff(self.items.get(arg))
        if value_id == "filter_enabled":
            return onoff(self.filters.get(arg))
        if value_id == "input_text":
            text = self.texts.get(arg)
            return UNKNOWN if text is None else text
        if value_id == "media_playing":
            return onoff(self.media.get(arg))
        return UNKNOWN

    def publish_all(self):
        with self.lock:
            for key, value in self.fixed_values().items():
                self._put(key, value)
            for value_id in ARG_VALUES:
                for arg in self.watched(value_id):
                    self._put(f"{value_id}:{arg}", self.arg_value(value_id, arg))

    # -- connection --

    def connected(self):
        self.reset()
        self.publish_all()
        self.schedule("sync", self.sync)

    def disconnected(self):
        self.reset()
        self.publish_all()

    def watch_changed(self):
        self.publish_all()
        if self.obs.connected:
            self.schedule("args-missing", lambda: self.refresh_args(missing_only=True))

    def sync(self):
        """Read the whole state once after connecting (one RequestBatch)."""
        names = ["GetVersion", "GetSceneList", "GetStudioModeEnabled", "GetStreamStatus", "GetRecordStatus",
                 "GetReplayBufferStatus", "GetVirtualCamStatus", "GetCurrentSceneTransition",
                 "GetProfileList", "GetSceneCollectionList"]
        r = dict(zip(names, self.obs.batch([(n, {}) for n in names])))

        def ok(name):
            value = r.get(name)
            return value if isinstance(value, dict) else {}

        with self.lock:
            s = self.s
            if ok("GetVersion"):
                self.available = set(ok("GetVersion").get("availableRequests") or [])
            scenes = ok("GetSceneList")
            s["program"] = scenes.get("currentProgramSceneName")
            s["preview"] = scenes.get("currentPreviewSceneName")
            s["studio"] = ok("GetStudioModeEnabled").get("studioModeEnabled")
            stream, record = ok("GetStreamStatus"), ok("GetRecordStatus")
            s["streaming"] = stream.get("outputActive")
            s["recording"] = record.get("outputActive")
            s["paused"] = record.get("outputPaused")
            s["stream_time"] = timecode(stream.get("outputTimecode")) if stream else None
            s["record_time"] = timecode(record.get("outputTimecode")) if record else None
            replay = r.get("GetReplayBufferStatus")
            s["replay"] = replay.get("outputActive") if isinstance(replay, dict) else False
            s["virtualcam"] = ok("GetVirtualCamStatus").get("outputActive")
            s["transition"] = ok("GetCurrentSceneTransition").get("transitionName")
            s["profile"] = ok("GetProfileList").get("currentProfileName")
            s["collection"] = ok("GetSceneCollectionList").get("currentSceneCollectionName")
            if s["streaming"] is False:
                s["bitrate"] = 0
            self.publish_all()
        self.refresh_args(missing_only=False)

    def refresh_args(self, missing_only):
        """Fetch what watched values with an argument need (two RequestBatches at most)."""
        if self.changing or not self.obs.connected:
            return
        with self.lock:
            requests, keys = [], []

            def want(cache, arg):
                return not missing_only or arg not in cache

            inputs = set()
            for arg in self.watched("muted"):
                if want(self.muted, arg):
                    requests.append(("GetInputMute", {"inputName": arg}))
                    keys.append(("muted", arg))
            for arg in self.watched("volume") + self.watched("volume_db"):
                if want(self.volume, arg) and arg not in inputs:
                    inputs.add(arg)
                    requests.append(("GetInputVolume", {"inputName": arg}))
                    keys.append(("volume", arg))
            for arg in self.watched("input_text"):
                if want(self.texts, arg):
                    requests.append(("GetInputSettings", {"inputName": arg}))
                    keys.append(("text", arg))
            for arg in self.watched("media_playing"):
                if want(self.media, arg):
                    requests.append(("GetMediaInputStatus", {"inputName": arg}))
                    keys.append(("media", arg))
            for arg in self.watched("filter_enabled"):
                if want(self.filters, arg):
                    source, name = split_pair(arg)
                    requests.append(("GetSourceFilter", {"sourceName": source, "filterName": name}))
                    keys.append(("filter", arg))
            for arg in self.watched("item_visible"):
                if want(self.items, arg):
                    scene, source = split_pair(arg)
                    requests.append(("GetSceneItemId", {"sceneName": scene, "sourceName": source}))
                    keys.append(("item", arg))
        if not requests:
            return
        results = self.obs.batch(requests)
        second, second_keys = [], []
        with self.lock:
            for (kind, arg), result in zip(keys, results):
                good = isinstance(result, dict)
                if kind == "muted":
                    self.muted[arg] = result.get("inputMuted") if good else None
                elif kind == "volume":
                    self.volume[arg] = (result.get("inputVolumeMul", 0), result.get("inputVolumeDb")) if good else None
                elif kind == "text":
                    self.texts[arg] = str((result.get("inputSettings") or {}).get("text", "")) if good else None
                elif kind == "media":
                    self.media[arg] = (result.get("mediaState") == "OBS_MEDIA_STATE_PLAYING") if good else None
                elif kind == "filter":
                    self.filters[arg] = result.get("filterEnabled") if good else None
                    self.filter_keys.setdefault(split_pair(arg), set()).add(arg)
                elif kind == "item":
                    self.items[arg] = None
                    if good and result.get("sceneItemId") is not None:
                        scene, _ = split_pair(arg)
                        item_id = result["sceneItemId"]
                        for args in self.item_keys.values():
                            args.discard(arg)
                        self.item_keys.setdefault((scene, item_id), set()).add(arg)
                        second.append(("GetSceneItemEnabled", {"sceneName": scene, "sceneItemId": item_id}))
                        second_keys.append(arg)
            self.publish_all()
        if second:
            results = self.obs.batch(second)
            with self.lock:
                for arg, result in zip(second_keys, results):
                    self.items[arg] = result.get("sceneItemEnabled") if isinstance(result, dict) else None
                self.publish_all()

    def structure_changed(self):
        self.schedule("args-all", lambda: self.refresh_args(missing_only=False))

    # -- events (called on the connection's reader thread: no requests here) --

    def on_event(self, event, d):
        with self.lock:
            s = self.s
            if event == "CurrentProgramSceneChanged":
                s["program"] = d.get("sceneName")
            elif event == "CurrentPreviewSceneChanged":
                s["preview"] = d.get("sceneName")
            elif event == "StudioModeStateChanged":
                s["studio"] = bool(d.get("studioModeEnabled"))
                if not s["studio"]:
                    s["preview"] = None
                else:
                    self.schedule("preview", self._fetch_preview)
            elif event == "StreamStateChanged":
                s["streaming"] = bool(d.get("outputActive"))
                if not s["streaming"]:
                    s["stream_time"], s["bitrate"], self.last_bytes = "00:00:00", 0, None
            elif event == "RecordStateChanged":
                state = d.get("outputState")
                s["recording"] = bool(d.get("outputActive"))
                if state == "OBS_WEBSOCKET_OUTPUT_PAUSED":
                    s["paused"] = True
                elif state in ("OBS_WEBSOCKET_OUTPUT_RESUMED", "OBS_WEBSOCKET_OUTPUT_STARTED",
                               "OBS_WEBSOCKET_OUTPUT_STOPPED"):
                    s["paused"] = False
                if not s["recording"]:
                    s["record_time"], s["paused"] = "00:00:00", False
            elif event == "ReplayBufferStateChanged":
                s["replay"] = bool(d.get("outputActive"))
            elif event == "VirtualcamStateChanged":
                s["virtualcam"] = bool(d.get("outputActive"))
            elif event == "CurrentSceneTransitionChanged":
                s["transition"] = d.get("transitionName")
            elif event == "SceneTransitionEnded":
                self.transition_ended.set()
            elif event == "CurrentProfileChanged":
                s["profile"] = d.get("profileName")
            elif event == "CurrentSceneCollectionChanging":
                self.changing = True
            elif event == "CurrentSceneCollectionChanged":
                self.changing = False
                s["collection"] = d.get("sceneCollectionName")
                self.schedule("sync", self.sync)
            elif event == "InputMuteStateChanged":
                self.muted[d.get("inputName")] = bool(d.get("inputMuted"))
            elif event == "InputVolumeChanged":
                self.volume[d.get("inputName")] = (d.get("inputVolumeMul", 0), d.get("inputVolumeDb"))
            elif event == "InputSettingsChanged":
                name, settings = d.get("inputName"), d.get("inputSettings") or {}
                if name in self.texts:
                    if "text" in settings:
                        self.texts[name] = str(settings["text"])
                    else:
                        self.texts.pop(name, None)
                        self.schedule("args-missing", lambda: self.refresh_args(missing_only=True))
            elif event == "SceneItemEnableStateChanged":
                for arg in self.item_keys.get((d.get("sceneName"), d.get("sceneItemId")), ()):
                    self.items[arg] = bool(d.get("sceneItemEnabled"))
            elif event == "SourceFilterEnableStateChanged":
                for arg in self.filter_keys.get((d.get("sourceName"), d.get("filterName")), ()):
                    self.filters[arg] = bool(d.get("filterEnabled"))
            elif event == "MediaInputPlaybackStarted":
                self.media[d.get("inputName")] = True
            elif event == "MediaInputPlaybackEnded":
                self.media[d.get("inputName")] = False
            elif event == "MediaInputActionTriggered":
                if d.get("inputName") in self.media:
                    self.media.pop(d.get("inputName"), None)
                    self.schedule("args-missing", lambda: self.refresh_args(missing_only=True))
            elif event in ("InputNameChanged", "InputRemoved"):
                gone = d.get("oldInputName") or d.get("inputName")
                for cache in (self.muted, self.volume, self.texts, self.media):
                    cache.pop(gone, None)
                self.structure_changed()
            elif event in ("SceneItemCreated", "SceneItemRemoved", "SceneNameChanged", "SceneRemoved",
                           "SceneCreated", "InputCreated",
                           "SourceFilterCreated", "SourceFilterRemoved", "SourceFilterNameChanged"):
                self.structure_changed()
            else:
                return
            self.publish_all()

    def _fetch_preview(self):
        try:
            data = self.obs.call("GetCurrentPreviewScene", wait=0)
        except ObsError:
            return
        with self.lock:
            self.s["preview"] = data.get("sceneName") or data.get("currentPreviewSceneName")
            self.publish_all()

    # -- statistics and timecodes (polled: OBS has no events for them) --

    def poll(self):
        if not self.obs.connected or self.changing:
            return
        stats, stream, record = self.obs.batch(
            [("GetStats", {}), ("GetStreamStatus", {}), ("GetRecordStatus", {})], drop_on_timeout=True
        )
        now = self.clock()
        with self.lock:
            s = self.s
            if isinstance(stats, dict):
                s["fps"] = stats.get("activeFps")
                s["cpu"] = stats.get("cpuUsage")
                s["memory"] = stats.get("memoryUsage")
                skipped = (stats.get("renderSkippedFrames") or 0) + (stats.get("outputSkippedFrames") or 0)
                total = (stats.get("renderTotalFrames") or 0) + (stats.get("outputTotalFrames") or 0)
                s["dropped"] = 100.0 * skipped / total if total else 0
            if isinstance(stream, dict):
                active = bool(stream.get("outputActive"))
                s["streaming"] = active
                s["stream_time"] = timecode(stream.get("outputTimecode")) if active else "00:00:00"
                sent = stream.get("outputBytes") or 0
                if not active:
                    s["bitrate"], self.last_bytes = 0, None
                else:
                    last = self.last_bytes
                    if last and sent >= last[0] and now > last[1]:
                        s["bitrate"] = (sent - last[0]) * 8 / 1000.0 / (now - last[1])
                    elif s["bitrate"] is None:
                        s["bitrate"] = 0
                    self.last_bytes = (sent, now)
            if isinstance(record, dict):
                active = bool(record.get("outputActive"))
                s["recording"] = active
                s["paused"] = bool(record.get("outputPaused")) if active else False
                s["record_time"] = timecode(record.get("outputTimecode")) if active else "00:00:00"
            self.publish_all()


# ---------------------------------------------------------------------------
# Plugin wiring
# ---------------------------------------------------------------------------

client = Obs(log=plugin.log)
live = Live(client, lambda full_id, value: plugin.set_value(full_id, value), plugin.watched, log=plugin.log)
client.on_event = live.on_event
client.on_connect = live.connected
client.on_disconnect = live.disconnected
last_response = {"value": UNKNOWN}


@plugin.on_settings
def configure(settings):
    client.configure(settings.get("url"), settings.get("password") or "", bool(settings.get("insecure")))


@plugin.on_start
def started():
    live.publish_all()
    plugin.set_value("last_response", last_response["value"])


@plugin.on_watch
def watching(values):
    live.watch_changed()


@plugin.every(2)
def poll():
    try:
        live.poll()
    except (NotConnected, TimeoutError, ObsError):
        pass


# -- helpers for actions --


def call(request_type, data=None, timeout=TIMEOUT):
    try:
        return client.call(request_type, data, timeout=timeout)
    except NotConnected as e:
        raise RuntimeError(str(e)) from None
    except TimeoutError:
        raise RuntimeError("OBS didn't answer in time") from None


def answer_in_time(work, still_working, name, deadline=None):
    """Run work() on a thread and wait up to ACTION_DEADLINE for it: the host wants an
    answer within 10 s. Done in time: its result (or its error). Not done: the
    still_working warning (a string, or a function that makes one) while work()
    finishes in the background; an error it hits then goes to the log."""
    box = {}
    lock = threading.Lock()

    def run():
        try:
            result, error = work(), None
        except Exception as e:
            result, error = None, e
        with lock:
            box.update(done=True, result=result, error=error)
            late = box.get("late")
        if late and error is not None:
            plugin.log(f"{name}: {error}", "warn")

    thread = threading.Thread(target=run, daemon=True)
    thread.start()
    thread.join(ACTION_DEADLINE if deadline is None else deadline)
    with lock:
        if not box.get("done"):
            box["late"] = True
            return still_working() if callable(still_working) else still_working
    if box["error"] is not None:
        raise box["error"]
    return box["result"]


def need(settings, key, what):
    value = str(settings.get(key) or "").strip()
    if not value:
        raise ValueError(f"enter the {what}")
    return value


def choice(settings, key, allowed, default):
    value = str(settings.get(key) or default).strip().lower()
    if value not in allowed:
        raise ValueError(f"unknown choice “{value}”")
    return value


def optional_number(settings, key, what, low=None, high=None):
    raw = settings.get(key)
    if raw is None or str(raw).strip() == "":
        return None
    try:
        value = float(raw)
    except (TypeError, ValueError):
        raise ValueError(f"the {what} must be a number") from None
    if (low is not None and value < low) or (high is not None and value > high):
        raise ValueError(f"the {what} must be between {number(low, 0)} and {number(high, 0)}")
    return value


def current_scene():
    scene = live.s["program"] if client.connected else None
    if scene:
        return scene
    data = call("GetCurrentProgramScene")
    return data.get("sceneName") or data.get("currentProgramSceneName")


def studio_enabled():
    studio = live.s["studio"] if client.connected else None
    if studio is None:
        studio = bool(call("GetStudioModeEnabled").get("studioModeEnabled"))
    return studio


def supported(request_type):
    """False when OBS says it doesn't have the request; None when we don't know."""
    available = live.available
    return None if available is None else request_type in available


def output_action(mode, requests, doing):
    """Run start/stop/toggle (or more) for an output; already-running and not-running
    are warnings, not errors."""
    try:
        call(requests[mode])
    except ObsError as e:
        if e.code == OUTPUT_RUNNING:
            return f"OBS is already {doing}"
        if e.code == OUTPUT_NOT_RUNNING:
            return f"OBS isn't {doing}"
        if e.code == OUTPUT_PAUSED:
            return "the recording is already paused"
        if e.code == OUTPUT_NOT_PAUSED:
            return "the recording isn't paused"
        if e.code == OUTPUT_DISABLED:
            raise RuntimeError(f"that's turned off in OBS's settings ({e.comment or 'output disabled'})") from None
        raise
    return None


# -- actions --


@plugin.action("scene")
def switch_scene(settings):
    scene = need(settings, "scene", "scene name")
    where = choice(settings, "where", ("auto", "program", "preview"), "auto")
    preview = where == "preview" or (where == "auto" and studio_enabled())
    call("SetCurrentPreviewScene" if preview else "SetCurrentProgramScene", {"sceneName": scene})


@plugin.action("studio-mode")
def studio_mode(settings):
    mode = choice(settings, "mode", ("toggle", "on", "off"), "toggle")
    enable = (not studio_enabled()) if mode == "toggle" else mode == "on"
    call("SetStudioModeEnabled", {"studioModeEnabled": enable})


def _restore_transition(previous, duration_ms):
    live.transition_ended.wait(min(duration_ms / 1000.0 + 2.0, 22.0))
    try:
        if previous.get("transitionName"):
            client.call("SetCurrentSceneTransition", {"transitionName": previous["transitionName"]})
        if previous.get("transitionDuration") and not previous.get("transitionFixed"):
            client.call("SetCurrentSceneTransitionDuration", {"transitionDuration": previous["transitionDuration"]})
    except (NotConnected, TimeoutError, ObsError) as e:
        plugin.log(f"couldn't put the transition back: {e}", "warn")


@plugin.action("transition")
def transition_now(settings):
    name = str(settings.get("transition") or "").strip()
    duration = optional_number(settings, "duration", "duration", 50, 20000)
    return answer_in_time(lambda: _transition(name, duration), STILL_WORKING, "transition")


def _transition(name, duration):
    previous = None
    if name or duration:
        previous = call("GetCurrentSceneTransition")
        if name:
            call("SetCurrentSceneTransition", {"transitionName": name})
        if duration:
            call("SetCurrentSceneTransitionDuration", {"transitionDuration": int(duration)})
    live.transition_ended.clear()
    try:
        call("TriggerStudioModeTransition")
    except ObsError as e:
        if previous:
            _restore_transition(previous, -2000)  # put it back right away
        if e.code == STUDIO_MODE_NOT_ACTIVE:
            return "Studio Mode is off; there's nothing to transition"
        raise
    if previous:
        wait = duration or previous.get("transitionDuration") or 1000
        threading.Thread(target=_restore_transition, args=(previous, wait), daemon=True).start()
    return None


@plugin.action("set-transition")
def set_transition(settings):
    name = need(settings, "transition", "transition name")
    duration = optional_number(settings, "duration", "duration", 50, 20000)
    call("SetCurrentSceneTransition", {"transitionName": name})
    if duration:
        call("SetCurrentSceneTransitionDuration", {"transitionDuration": int(duration)})


@plugin.action("stream")
def stream(settings):
    mode = choice(settings, "mode", ("toggle", "start", "stop"), "toggle")
    return output_action(mode, {"toggle": "ToggleStream", "start": "StartStream", "stop": "StopStream"}, "streaming")


@plugin.action("record")
def record(settings):
    modes = {
        "toggle": "ToggleRecord", "start": "StartRecord", "stop": "StopRecord",
        "pause-toggle": "ToggleRecordPause", "pause": "PauseRecord", "resume": "ResumeRecord",
    }
    mode = choice(settings, "mode", tuple(modes), "toggle")
    return output_action(mode, modes, "recording")


def _newer_request(request_type, what, data=None):
    if supported(request_type) is False:
        return f"{what} needs OBS 30.2 or newer"
    try:
        call(request_type, data)
    except ObsError as e:
        if e.code == UNKNOWN_REQUEST_TYPE:
            return f"{what} needs OBS 30.2 or newer"
        if e.code == OUTPUT_NOT_RUNNING:
            return "OBS isn't recording"
        raise
    return None


@plugin.action("record-split")
def record_split(settings):
    return _newer_request("SplitRecordFile", "splitting the recording")


@plugin.action("record-chapter")
def record_chapter(settings):
    name = str(settings.get("name") or "").strip()
    return _newer_request("CreateRecordChapter", "chapter markers", {"chapterName": name} if name else {})


@plugin.action("replay-buffer")
def replay_buffer(settings):
    modes = {"toggle": "ToggleReplayBuffer", "start": "StartReplayBuffer", "stop": "StopReplayBuffer",
             "save": "SaveReplayBuffer"}
    mode = choice(settings, "mode", tuple(modes), "toggle")
    try:
        return output_action(mode, modes, "running the replay buffer")
    except RuntimeError as e:
        if "turned off" in str(e):
            raise RuntimeError("the replay buffer is turned off in OBS (Settings → Output → Replay Buffer)") from None
        raise


@plugin.action("virtualcam")
def virtualcam(settings):
    modes = {"toggle": "ToggleVirtualCam", "start": "StartVirtualCam", "stop": "StopVirtualCam"}
    mode = choice(settings, "mode", tuple(modes), "toggle")
    return output_action(mode, modes, "running the virtual camera")


@plugin.action("mute")
def mute(settings):
    source = need(settings, "input", "audio source")
    mode = choice(settings, "mode", ("toggle", "mute", "unmute"), "toggle")
    if mode == "toggle":
        call("ToggleInputMute", {"inputName": source})
    else:
        call("SetInputMute", {"inputName": source, "inputMuted": mode == "mute"})


@plugin.action("volume")
def volume(settings):
    source = need(settings, "input", "audio source")
    try:
        value = float(settings.get("value", 0))
    except (TypeError, ValueError):
        raise ValueError("the slider sent no position") from None
    call("SetInputVolume", {"inputName": source, "inputVolumeMul": slider_to_mul(value)})


@plugin.action("volume-step")
def volume_step(settings):
    source = need(settings, "input", "audio source")
    step = optional_number(settings, "step", "step", -100, 100)
    if step is None:
        step = 10
    known = live.volume.get(source) if client.connected else None
    mul = known[0] if known else call("GetInputVolume", {"inputName": source}).get("inputVolumeMul", 0)
    target = min(max(mul_to_slider(mul) + step, 0), 100)
    call("SetInputVolume", {"inputName": source, "inputVolumeMul": slider_to_mul(target)})


def _scene_item(scene, source):
    try:
        return call("GetSceneItemId", {"sceneName": scene, "sourceName": source})["sceneItemId"]
    except ObsError as e:
        if e.code == RESOURCE_NOT_FOUND:
            raise RuntimeError(f"OBS has no source “{source}” in the scene “{scene}”") from None
        raise


@plugin.action("source-visibility")
def source_visibility(settings):
    source = need(settings, "source", "source name")
    mode = choice(settings, "mode", ("toggle", "show", "hide"), "toggle")
    return answer_in_time(
        lambda: _source_visibility(settings, source, mode), STILL_WORKING, f"showing or hiding {source}"
    )


def _source_visibility(settings, source, mode):
    scene = str(settings.get("scene") or "").strip() or current_scene()
    item = _scene_item(scene, source)
    if mode == "toggle":
        enabled = call("GetSceneItemEnabled", {"sceneName": scene, "sceneItemId": item}).get("sceneItemEnabled")
        show = not enabled
    else:
        show = mode == "show"
    call("SetSceneItemEnabled", {"sceneName": scene, "sceneItemId": item, "sceneItemEnabled": show})


@plugin.action("filter")
def source_filter(settings):
    source = need(settings, "source", "source or scene name")
    name = need(settings, "filter", "filter name")
    mode = choice(settings, "mode", ("toggle", "on", "off"), "toggle")
    if mode == "toggle":
        enable = not call("GetSourceFilter", {"sourceName": source, "filterName": name}).get("filterEnabled")
    else:
        enable = mode == "on"
    call("SetSourceFilterEnabled", {"sourceName": source, "filterName": name, "filterEnabled": enable})


def pictures_folder():
    """The user's Pictures folder on this computer."""
    home = os.path.expanduser("~")
    if sys.platform == "win32":
        try:
            import ctypes
            from ctypes import wintypes

            class GUID(ctypes.Structure):
                _fields_ = [("Data1", wintypes.DWORD), ("Data2", wintypes.WORD), ("Data3", wintypes.WORD),
                            ("Data4", ctypes.c_ubyte * 8)]

            folder_id = GUID(0x33E28130, 0x4E1E, 0x4676,
                             (ctypes.c_ubyte * 8)(0x83, 0x5A, 0x98, 0x39, 0x5C, 0x3B, 0xC3, 0xBB))
            path = ctypes.c_wchar_p()
            if ctypes.windll.shell32.SHGetKnownFolderPath(ctypes.byref(folder_id), 0, None, ctypes.byref(path)) == 0:
                try:
                    if path.value:
                        return path.value
                finally:
                    ctypes.windll.ole32.CoTaskMemFree(path)
        except Exception:
            pass
        return os.path.join(os.environ.get("USERPROFILE") or home, "Pictures")
    if sys.platform != "darwin":
        config = os.environ.get("XDG_CONFIG_HOME") or os.path.join(home, ".config")
        try:
            with open(os.path.join(config, "user-dirs.dirs"), encoding="utf-8") as f:
                for line in f:
                    m = re.match(r'\s*XDG_PICTURES_DIR\s*=\s*"(.*)"\s*$', line)
                    if m:
                        path = m.group(1).replace("$HOME", home)
                        if os.path.isabs(path) and path.rstrip("/") != home:
                            return path
        except OSError:
            pass
    return os.path.join(home, "Pictures")


def obs_is_local():
    try:
        host = parse_url(client.url)[1]
    except ValueError:
        return True
    return host in ("127.0.0.1", "localhost", "::1") or host.startswith("127.")


def safe_file_name(name):
    cleaned = re.sub(r'[\\/:*?"<>|\x00-\x1f]+', "_", name).strip(" .")
    return cleaned[:80] or "OBS"


@plugin.action("screenshot")
def screenshot(settings):
    source = str(settings.get("source") or "").strip() or current_scene()
    image_format = choice(settings, "format", ("png", "jpg"), "png")
    folder = str(settings.get("folder") or "").strip()
    if not folder:
        if not obs_is_local():
            raise ValueError("OBS runs on another computer: enter a folder on that computer")
        folder = pictures_folder()
    folder = os.path.expanduser(folder)
    if obs_is_local():
        try:
            os.makedirs(folder, exist_ok=True)
        except OSError:
            pass
    stamp = datetime.datetime.now().strftime("%Y-%m-%d %H-%M-%S")
    path = os.path.join(folder, f"{safe_file_name(source)} {stamp}.{image_format}")
    n = 2
    while obs_is_local() and os.path.exists(path):
        path = os.path.join(folder, f"{safe_file_name(source)} {stamp} ({n}).{image_format}")
        n += 1
    call("SaveSourceScreenshot", {"sourceName": source, "imageFormat": image_format, "imageFilePath": path})


@plugin.action("hotkey")
def hotkey(settings):
    call("TriggerHotkeyByName", {"hotkeyName": need(settings, "hotkey", "hotkey name")})


MEDIA = {
    "play": "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_PLAY",
    "pause": "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_PAUSE",
    "restart": "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_RESTART",
    "stop": "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_STOP",
    "next": "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_NEXT",
    "previous": "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_PREVIOUS",
}


@plugin.action("media")
def media(settings):
    source = need(settings, "input", "media source")
    command = choice(settings, "command", ("play-pause",) + tuple(MEDIA), "play-pause")
    if command == "play-pause":
        state = call("GetMediaInputStatus", {"inputName": source}).get("mediaState")
        if state == "OBS_MEDIA_STATE_PLAYING":
            command = "pause"
        elif state == "OBS_MEDIA_STATE_ENDED":
            command = "restart"
        else:
            command = "play"
    call("TriggerMediaInputAction", {"inputName": source, "mediaAction": MEDIA[command]})


@plugin.action("browser-refresh")
def browser_refresh(settings):
    source = need(settings, "input", "browser source")
    call("PressInputPropertiesButton", {"inputName": source, "propertyName": "refreshnocache"})


@plugin.action("set-text")
def set_text(settings):
    source = need(settings, "input", "text source")
    text = str(settings.get("text") or "").replace("\\n", "\n")
    call("SetInputSettings", {"inputName": source, "inputSettings": {"text": text}})


@plugin.action("projector")
def projector(settings):
    what = choice(settings, "what", ("program", "preview", "multiview", "source"), "program")
    screen = optional_number(settings, "monitor", "screen number", 1, 32)
    data = {}
    if screen is not None:
        data["monitorIndex"] = int(screen) - 1
    if what == "source":
        data["sourceName"] = need(settings, "source", "source or scene name")
        call("OpenSourceProjector", data)
    else:
        data["videoMixType"] = "OBS_WEBSOCKET_VIDEO_MIX_TYPE_" + what.upper()
        call("OpenVideoMixProjector", data)


@plugin.action("profile")
def profile(settings):
    call("SetCurrentProfile", {"profileName": need(settings, "profile", "profile name")})


@plugin.action("scene-collection")
def scene_collection(settings):
    name = need(settings, "collection", "scene collection name")
    # Switching can take OBS a while with big collections: answer the key in time and
    # let the request finish (OBS's event updates the value when it's done).
    def switch():
        call("SetCurrentSceneCollection", {"sceneCollectionName": name}, timeout=20)

    return answer_in_time(
        switch,
        f"OBS is still switching to “{name}”…",
        f"switching to the scene collection {name}",
    )


@plugin.action("caption")
def caption(settings):
    text = need(settings, "text", "caption text")
    try:
        call("SendStreamCaption", {"captionText": text})
    except ObsError as e:
        if e.code == OUTPUT_NOT_RUNNING:
            return "OBS isn't streaming"
        raise
    return None


def _send_request(request_type, data_text):
    request_type = request_type.strip()
    if not re.match(r"^[A-Za-z]+$", request_type):
        raise ValueError("enter a request type such as GetVersion")
    data = None
    if data_text.strip():
        try:
            data = json.loads(data_text)
        except ValueError as e:
            raise ValueError(f"the data isn't valid JSON: {e}") from None
        if not isinstance(data, dict):
            raise ValueError("the data must be a JSON object, like {\"sceneName\": \"Gaming\"}")
    response = call(request_type, data)
    text = json.dumps(response, ensure_ascii=False, separators=(",", ":")) if response else "ok"
    if len(text) > 500:
        text = text[:499] + "…"
    last_response["value"] = text
    plugin.set_value("last_response", text)


@plugin.action("request")
def send_request(settings):
    _send_request(str(settings.get("request") or ""), str(settings.get("data") or ""))


@plugin.action("request-value")
def send_request_value(settings):
    try:
        value = min(max(float(settings.get("value", 0)), 0.0), 100.0)
    except (TypeError, ValueError):
        raise ValueError("the slider sent no position") from None
    data = str(settings.get("data") or "")
    data = data.replace("{value}", number(value, 2)).replace("{mul}", repr(slider_to_mul(value)))
    _send_request(str(settings.get("request") or ""), data)


if __name__ == "__main__":
    plugin.run()
