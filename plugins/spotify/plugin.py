"""Spotify: control playback and show what's playing, through the Spotify Web API.

Signs in with the Authorization Code flow and PKCE (RFC 7636), using a client ID the
user registers in the Spotify developer dashboard: no client secret is needed. The
sign-in answers on a one-shot web server on 127.0.0.1 (Spotify doesn't accept
"localhost" redirect addresses). Tokens are kept in the plugin's data folder,
readable by the user only, and refreshed automatically. Standard library only.
"""

import base64
import hashlib
import http.server
import json
import os
import re
import secrets
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin, open_for_user  # noqa: E402

plugin = Plugin()
NOTHING = "–"
ACCOUNTS_URL = "https://accounts.spotify.com"
API_URL = "https://api.spotify.com/v1"
DEFAULT_PORT = 47123
TIMEOUT = 6  # per request; the host wants an answer within 10 s
SIGN_IN_WAIT = 300  # seconds the sign-in page may take
POLL_PLAYING = 5
POLL_IDLE = 15  # nothing playing: no need to ask as often
REFRESH_AFTER_ACTION = 0.7
TOKEN_FILE = "tokens.json"
SCOPES = (
    "user-read-playback-state",
    "user-modify-playback-state",
    "user-read-currently-playing",
    "user-library-read",
    "user-library-modify",
    "playlist-modify-public",
    "playlist-modify-private",
)
VALUES = ("track", "artist", "album", "playing", "shuffle", "repeat", "liked", "volume", "progress", "device")
REPEAT_NEXT = {"off": "context", "context": "track", "track": "off"}
REPEAT_NAMES = {"off": "off", "context": "all", "track": "one"}


class SpotifyError(RuntimeError):
    pass


# --- PKCE --------------------------------------------------------------------------


def b64url(raw):
    return base64.urlsafe_b64encode(raw).decode("ascii").rstrip("=")


def make_verifier():
    """A code verifier: 43–128 characters from the unreserved set (RFC 7636 4.1)."""
    return b64url(secrets.token_bytes(64))  # 86 characters


def challenge_for(verifier):
    """The S256 code challenge: BASE64URL(SHA256(ASCII(verifier))) without padding."""
    return b64url(hashlib.sha256(verifier.encode("ascii")).digest())


# --- settings and tokens -------------------------------------------------------------


def client_id():
    cid = ((plugin.settings or {}).get("client_id") or "").strip()
    if not cid:
        raise SpotifyError("set your Spotify app's client ID on the Plugins page")
    return cid


def sign_in_port():
    raw = (plugin.settings or {}).get("port")
    if raw in (None, ""):
        return DEFAULT_PORT
    try:
        port = int(str(raw).strip())
    except ValueError:
        raise SpotifyError(f"the sign-in port must be a number, not {raw!r}") from None
    if not 0 <= port <= 65535:
        raise SpotifyError(f"the sign-in port must be between 1 and 65535 (or 0 for any free port), not {port}")
    return port


_token_lock = threading.RLock()


def token_path():
    return os.path.join(plugin.data_dir, TOKEN_FILE)


def load_tokens():
    try:
        with open(token_path(), encoding="utf-8") as f:
            tokens = json.load(f)
    except (OSError, ValueError):
        return None
    return tokens if isinstance(tokens, dict) and tokens.get("refresh_token") else None


def save_tokens(tokens):
    """Write the tokens readable by the user only (0600 where the system has modes)."""
    path = token_path()
    tmp = path + ".tmp"
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        json.dump(tokens, f)
    try:
        os.chmod(tmp, 0o600)
    except OSError:
        pass
    os.replace(tmp, path)


def forget_tokens():
    try:
        os.remove(token_path())
    except OSError:
        pass


def current_tokens():
    """The saved tokens if they belong to the client ID in the settings."""
    tokens = load_tokens()
    if tokens is None:
        return None
    cid = ((plugin.settings or {}).get("client_id") or "").strip()
    if tokens.get("client_id") != cid:
        return None
    return tokens


def token_request(form):
    """POST to the accounts token endpoint; returns its JSON or raises SpotifyError."""
    data = urllib.parse.urlencode(form).encode()
    req = urllib.request.Request(f"{ACCOUNTS_URL}/api/token", data=data, method="POST")
    req.add_header("Content-Type", "application/x-www-form-urlencoded")
    req.add_header("Accept", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
            return json.loads(resp.read().decode("utf-8") or "{}")
    except urllib.error.HTTPError as e:
        try:
            body = json.loads(e.read().decode("utf-8") or "{}")
        except ValueError:
            body = {}
        code = body.get("error") if isinstance(body, dict) else None
        text = body.get("error_description") if isinstance(body, dict) else None
        raise SpotifyError(f"Spotify refused the sign-in ({code or e.code}{': ' + text if text else ''})") from None
    except (urllib.error.URLError, OSError) as e:
        raise SpotifyError(f"couldn't reach Spotify: {getattr(e, 'reason', e)}") from None
    except ValueError:
        raise SpotifyError("Spotify sent a sign-in answer that isn't JSON") from None


def store_token_answer(answer, cid, old=None):
    if not answer.get("access_token"):
        raise SpotifyError("Spotify didn't send an access token")
    tokens = {
        "client_id": cid,
        "access_token": answer["access_token"],
        # A refresh may or may not hand out a new refresh token; keep the old one if not.
        "refresh_token": answer.get("refresh_token") or (old or {}).get("refresh_token"),
        "expires_at": time.time() + int(answer.get("expires_in") or 3600),
        "scope": answer.get("scope", (old or {}).get("scope", "")),
    }
    save_tokens(tokens)
    return tokens


def refresh(tokens):
    with _token_lock:
        latest = current_tokens()
        if latest and latest.get("access_token") != tokens.get("access_token"):
            return latest  # another thread refreshed it already
        cid = client_id()
        try:
            answer = token_request({
                "grant_type": "refresh_token",
                "refresh_token": tokens["refresh_token"],
                "client_id": cid,
            })
        except SpotifyError as e:
            if "invalid_grant" in str(e):
                forget_tokens()
                raise SpotifyError("Spotify ended the sign-in; press \"Sign in to Spotify\" again") from None
            raise
        plugin.log("Spotify sign-in refreshed")
        return store_token_answer(answer, cid, tokens)


def access_token(force_refresh=False):
    with _token_lock:
        client_id()
        tokens = current_tokens()
        if tokens is None:
            raise SpotifyError("not signed in: press \"Sign in to Spotify\" first")
        if force_refresh or time.time() > float(tokens.get("expires_at") or 0) - 60:
            tokens = refresh(tokens)
        return tokens["access_token"]


# --- sign-in -------------------------------------------------------------------------


class SignInServer(http.server.HTTPServer):
    # HTTPServer sets SO_REUSEADDR. On Windows that lets this server bind a port another
    # socket is still listening on, and which of the two gets the browser is then anyone's
    # guess. Windows doesn't need it to reuse a port in TIME_WAIT either; the standard
    # library's socket.create_server leaves it off there for the same reasons.
    allow_reuse_address = os.name != "nt"


class SignIn:
    """One sign-in attempt: a one-shot web server on 127.0.0.1 waiting for Spotify to
    send the browser back with a code, then the code exchange."""

    def __init__(self, cid, port):
        self.cid = cid
        self.verifier = make_verifier()
        self.state = b64url(secrets.token_bytes(24))
        self.done = threading.Event()
        self.error = None
        sign_in = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                sign_in.answer(self)

            def log_message(self, *args):
                pass

        try:
            self.server = SignInServer(("127.0.0.1", port), Handler)
        except OSError as e:
            raise SpotifyError(
                f"couldn't listen on 127.0.0.1:{port} for the sign-in ({e.strerror or e}); "
                "pick another sign-in port") from None
        self.server.timeout = 1
        self.port = self.server.server_port
        self.redirect_uri = f"http://127.0.0.1:{self.port}/callback"
        self.thread = threading.Thread(target=self.serve, name="spotify-sign-in", daemon=True)

    def authorize_url(self):
        return f"{ACCOUNTS_URL}/authorize?" + urllib.parse.urlencode({
            "response_type": "code",
            "client_id": self.cid,
            "scope": " ".join(SCOPES),
            "redirect_uri": self.redirect_uri,
            "state": self.state,
            "code_challenge_method": "S256",
            "code_challenge": challenge_for(self.verifier),
        })

    def start(self):
        self.thread.start()

    def serve(self):
        deadline = time.monotonic() + SIGN_IN_WAIT
        try:
            while not self.done.is_set() and time.monotonic() < deadline:
                self.server.handle_request()
            if not self.done.is_set():
                self.finish("the sign-in took too long; press \"Sign in to Spotify\" again")
        finally:
            self.server.server_close()

    def finish(self, error=None):
        self.error = error
        if error:
            plugin.log(f"Spotify sign-in: {error}", "warn")
        else:
            plugin.log("signed in to Spotify")
        self.done.set()

    def answer(self, request):
        url = urllib.parse.urlparse(request.path)
        if url.path != "/callback":
            return self.page(request, 404, "Not found.")
        query = dict(urllib.parse.parse_qsl(url.query))
        if query.get("state") != self.state:
            return self.page(request, 400, "This sign-in link doesn't match. Start again from PerritoDeck.")
        if query.get("error"):
            self.page(request, 200, "Spotify sign-in was cancelled. You can close this tab.")
            return self.finish(f"Spotify said {query['error']}")
        code = query.get("code")
        if not code:
            return self.page(request, 400, "Spotify didn't send a code. Start again from PerritoDeck.")
        try:
            answer = token_request({
                "grant_type": "authorization_code",
                "code": code,
                "redirect_uri": self.redirect_uri,
                "client_id": self.cid,
                "code_verifier": self.verifier,
            })
            with _token_lock:
                store_token_answer(answer, self.cid)
        except SpotifyError as e:
            self.page(request, 200, f"Sign-in failed: {e}. You can close this tab.")
            return self.finish(str(e))
        self.page(request, 200, "Signed in to Spotify. You can close this tab and go back to PerritoDeck.")
        self.finish()
        poll_soon()

    @staticmethod
    def page(request, status, text):
        body = (
            "<!doctype html><meta charset=utf-8><title>PerritoDeck</title>"
            f"<p style=\"font:16px sans-serif;margin:2em\">{text}</p>"
        ).encode("utf-8")
        request.send_response(status)
        request.send_header("Content-Type", "text/html; charset=utf-8")
        request.send_header("Content-Length", str(len(body)))
        request.end_headers()
        request.wfile.write(body)


_sign_in = {"current": None}
# Actions run on threads of their own: two quick presses mustn't both replace the attempt.
_sign_in_lock = threading.Lock()


@plugin.action("sign-in")
def sign_in(settings):
    cid = client_id()
    with _sign_in_lock:
        old = _sign_in["current"]
        if old is not None:
            if not old.done.is_set():
                old.finish("replaced by a new sign-in")
            # It stops listening within a second (server.timeout); the new attempt needs the port.
            old.thread.join(2)
        attempt = SignIn(cid, sign_in_port())
        attempt.start()
        _sign_in["current"] = attempt
    url = attempt.authorize_url()
    try:
        open_for_user(url)  # a browser it starts must outlive a plugin restart
    except OSError:
        plugin.log(f"open this address to sign in to Spotify: {url}", "warn")
        return "couldn't open a web browser; the sign-in address is in the log"
    return "finish signing in in your web browser"


@plugin.action("sign-out")
def sign_out(settings):
    with _token_lock:
        forget_tokens()
    _state.update(player=None, liked={})
    publish_all(dict.fromkeys(VALUES, NOTHING))
    return None


# --- API -----------------------------------------------------------------------------

_backoff = {"until": 0.0}


def wait_left():
    return max(0.0, _backoff["until"] - time.monotonic())


def api(method, path, params=None, body=None):
    """Call the Web API; returns its JSON (None for an empty answer)."""
    left = wait_left()
    if left > 0:
        raise SpotifyError(f"Spotify asked to slow down; try again in {int(left) + 1} s")
    full = API_URL + path
    if params:
        full += "?" + urllib.parse.urlencode(params)
    for attempt in (0, 1):
        token = access_token(force_refresh=attempt == 1)
        # Spotify answers 411 to a PUT/POST without a length, so always send a body.
        data = json.dumps(body).encode() if body is not None else (b"" if method in ("PUT", "POST") else None)
        req = urllib.request.Request(full, data=data, method=method)
        req.add_header("Authorization", f"Bearer {token}")
        req.add_header("Accept", "application/json")
        if body is not None:
            req.add_header("Content-Type", "application/json")
        try:
            with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
                raw = resp.read()
        except urllib.error.HTTPError as e:
            if e.code == 401 and attempt == 0:
                continue  # expired early or revoked: refresh once and try again
            raise SpotifyError(describe_http_error(e)) from None
        except (urllib.error.URLError, OSError) as e:
            raise SpotifyError(f"couldn't reach Spotify: {getattr(e, 'reason', e)}") from None
        if not raw.strip():
            return None
        try:
            return json.loads(raw.decode("utf-8"))
        except ValueError:
            return None  # some player commands answer with a plain text body
    return None


def describe_http_error(e):
    reason = message = ""
    try:
        body = json.loads(e.read().decode("utf-8") or "{}")
        err = body.get("error") if isinstance(body, dict) else None
        if isinstance(err, dict):
            reason = err.get("reason") or ""
            message = err.get("message") or ""
        elif isinstance(err, str):
            message = body.get("error_description") or err
    except Exception:
        pass
    if e.code == 429:
        try:
            wait = max(1, int(e.headers.get("Retry-After") or 5))
        except ValueError:
            wait = 5
        _backoff["until"] = time.monotonic() + wait
        return f"Spotify asked to slow down; try again in {wait} s"
    if e.code == 401:
        return "Spotify refused the sign-in; press \"Sign in to Spotify\" again"
    if reason == "PREMIUM_REQUIRED" or (e.code == 403 and "premium" in message.lower()):
        return "controlling playback needs Spotify Premium"
    if reason == "NO_ACTIVE_DEVICE" or (e.code == 404 and "device" in message.lower()):
        return "no Spotify device is active: start playing in a Spotify app first"
    if e.code == 403 and "scope" in message.lower():
        return "the sign-in doesn't allow that; press \"Sign in to Spotify\" again"
    return f"Spotify said {e.code}: {message}" if message else f"Spotify said {e.code} {e.reason}"


def player():
    return api("GET", "/me/player", {"additional_types": "track,episode"})


def need_player():
    state = player()
    if not state or not state.get("device"):
        raise SpotifyError("no Spotify device is active: start playing in a Spotify app first")
    return state


def current_uri():
    state = player() or {}
    item = state.get("item") or {}
    if not item.get("uri"):
        raise SpotifyError("nothing is playing")
    return item["uri"], item


def poll_soon():
    timer = threading.Timer(REFRESH_AFTER_ACTION, safe_poll)
    timer.daemon = True
    timer.start()


def after(result=None):
    poll_soon()
    return result


# --- references: spotify:playlist:… or https://open.spotify.com/playlist/… ----------------

KINDS = ("track", "episode", "album", "playlist", "artist", "show")
_ID = re.compile(r"^[A-Za-z0-9]{8,40}$")


def parse_ref(text, default_kind=None):
    """('playlist', '37i9…') from a Spotify URI, an open.spotify.com link or (with a
    default kind) a bare ID."""
    text = (text or "").strip()
    if not text:
        raise SpotifyError("enter a Spotify link or URI")
    if text.startswith("spotify:"):
        parts = text.split(":")
        # spotify:playlist:ID, or the older spotify:user:NAME:playlist:ID
        if len(parts) >= 3 and parts[-2] in KINDS and _ID.match(parts[-1]):
            return parts[-2], parts[-1]
    elif "://" in text or text.startswith("open.spotify.com"):
        url = urllib.parse.urlparse(text if "://" in text else "https://" + text)
        parts = [p for p in url.path.split("/") if p]
        for i, part in enumerate(parts[:-1]):
            if part in KINDS and _ID.match(parts[i + 1]):
                return part, parts[i + 1]
    elif default_kind and _ID.match(text):
        return default_kind, text
    raise SpotifyError(f"that isn't a Spotify link or URI: {text}")


# --- actions -------------------------------------------------------------------------


@plugin.action("play-pause")
def play_pause(settings):
    state = need_player()
    api("PUT", "/me/player/pause" if state.get("is_playing") else "/me/player/play")
    _state["player"] = dict(state, is_playing=not state.get("is_playing"))
    return after()


@plugin.action("next")
def next_track(settings):
    api("POST", "/me/player/next")
    return after()


@plugin.action("previous")
def previous_track(settings):
    api("POST", "/me/player/previous")
    return after()


def wanted(settings, current):
    """'toggle' (the default), 'on' or 'off' → the new state."""
    mode = (settings.get("mode") or "toggle").strip().lower()
    if mode == "on":
        return True
    if mode == "off":
        return False
    return not current


@plugin.action("shuffle")
def shuffle(settings):
    state = need_player()
    on = wanted(settings, bool(state.get("shuffle_state")))
    api("PUT", "/me/player/shuffle", {"state": "true" if on else "false"})
    return after()


@plugin.action("repeat")
def repeat(settings):
    state = need_player()
    mode = (settings.get("mode") or "cycle").strip().lower()
    new = REPEAT_NEXT.get(state.get("repeat_state") or "off", "off") if mode == "cycle" else mode
    if new not in REPEAT_NEXT:
        raise SpotifyError(f"repeat is off, context or track, not {mode}")
    api("PUT", "/me/player/repeat", {"state": new})
    return after()


def set_volume(percent):
    percent = max(0, min(100, int(round(percent))))
    api("PUT", "/me/player/volume", {"volume_percent": percent})
    return percent


def number(value, what, default=None):
    if value in (None, ""):
        if default is None:
            raise SpotifyError(f"enter {what}")
        return default
    try:
        return float(value)
    except (TypeError, ValueError):
        raise SpotifyError(f"{what} must be a number, not {value!r}") from None


@plugin.action("volume")
def volume(settings):
    set_volume(number(settings.get("value"), "the volume"))
    return after()


def step_volume(settings, sign):
    state = need_player()
    device = state.get("device") or {}
    current = device.get("volume_percent")
    if current is None or device.get("supports_volume") is False:
        raise SpotifyError(f"Spotify can't change the volume of {device.get('name') or 'this device'}")
    set_volume(current + sign * abs(number(settings.get("step"), "the step", 10)))
    return after()


@plugin.action("volume-up")
def volume_up(settings):
    return step_volume(settings, +1)


@plugin.action("volume-down")
def volume_down(settings):
    return step_volume(settings, -1)


def is_liked(uri):
    answer = api("GET", "/me/library/contains", {"uris": uri})
    return bool(answer and answer[0])


@plugin.action("like")
def like(settings):
    uri, item = current_uri()
    liked = is_liked(uri)
    want = wanted(settings, liked)
    if want != liked:
        api("PUT" if want else "DELETE", "/me/library", {"uris": uri})
    _state["liked"] = {uri: want}
    return after()


def find_device(name):
    name = (name or "").strip()
    devices = (api("GET", "/me/player/devices") or {}).get("devices") or []
    lower = name.lower()
    for test in (lambda d: d.lower() == lower, lambda d: lower in d.lower()):
        for device in devices:
            if device.get("id") and test(device.get("name") or ""):
                return device
    names = ", ".join(d.get("name") or "?" for d in devices) or "none (open Spotify on it first)"
    raise SpotifyError(f"no Spotify device called {name!r}; devices: {names}")


@plugin.action("play")
def play(settings):
    kind, item_id = parse_ref(settings.get("uri"))
    uri = f"spotify:{kind}:{item_id}"
    body = {"uris": [uri]} if kind in ("track", "episode") else {"context_uri": uri}
    params = None
    if (settings.get("device") or "").strip():
        params = {"device_id": find_device(settings["device"])["id"]}
    api("PUT", "/me/player/play", params, body)
    return after()


@plugin.action("add-to-playlist")
def add_to_playlist(settings):
    kind, playlist_id = parse_ref(settings.get("playlist"), default_kind="playlist")
    if kind != "playlist":
        raise SpotifyError("that's a link to a " + kind + ", not a playlist")
    uri, item = current_uri()
    api("POST", f"/playlists/{playlist_id}/items", body={"uris": [uri]})
    return None


@plugin.action("transfer")
def transfer(settings):
    if not (settings.get("device") or "").strip():
        raise SpotifyError("enter the device's name, as Spotify shows it")
    device = find_device(settings.get("device"))
    body = {"device_ids": [device["id"]]}
    if settings.get("play", True):
        body["play"] = True
    api("PUT", "/me/player", body=body)
    return after()


# --- live values ---------------------------------------------------------------------

_state = {"player": None, "liked": {}, "next": 0.0}
_published = {}
_publish_lock = threading.Lock()


def clock(ms):
    seconds = max(0, int((ms or 0) // 1000))
    hours, rest = divmod(seconds, 3600)
    return f"{hours}:{rest // 60:02d}:{rest % 60:02d}" if hours else f"{rest // 60}:{rest % 60:02d}"


def values_for(state, liked):
    if not state or (not state.get("item") and not state.get("device")):
        return dict(dict.fromkeys(VALUES, NOTHING), playing="off")
    item = state.get("item") or {}
    device = state.get("device") or {}
    if item.get("type") == "episode":
        artist = (item.get("show") or {}).get("name") or (item.get("show") or {}).get("publisher")
        album = (item.get("show") or {}).get("name")
    else:
        artist = ", ".join(a.get("name") for a in item.get("artists") or [] if a.get("name"))
        album = (item.get("album") or {}).get("name")
    vol = device.get("volume_percent")
    return {
        "track": item.get("name") or NOTHING,
        "artist": artist or NOTHING,
        "album": album or NOTHING,
        "playing": "on" if state.get("is_playing") else "off",
        "shuffle": "on" if state.get("shuffle_state") else "off",
        "repeat": REPEAT_NAMES.get(state.get("repeat_state"), NOTHING),
        "liked": NOTHING if liked is None else ("on" if liked else "off"),
        "volume": NOTHING if vol is None else str(int(vol)),
        "progress": f"{clock(state.get('progress_ms'))} / {clock(item.get('duration_ms'))}" if item else NOTHING,
        "device": device.get("name") or NOTHING,
    }


def publish_all(values):
    with _publish_lock:
        for value_id in VALUES:
            value = values.get(value_id, NOTHING)
            if _published.get(value_id) != value:
                _published[value_id] = value
                plugin.set_value(value_id, value)


def poll():
    """Ask Spotify what's playing and publish what changed. Returns seconds to the next poll."""
    if current_tokens() is None or not (plugin.settings or {}).get("client_id"):
        publish_all(dict.fromkeys(VALUES, NOTHING))
        _state["player"] = None
        return POLL_IDLE
    state = player()
    _state["player"] = state
    liked = None
    uri = ((state or {}).get("item") or {}).get("uri")
    if uri:
        cache = _state["liked"]
        if uri not in cache:
            try:
                cache = {uri: is_liked(uri)}  # one track at a time is enough
            except SpotifyError as e:
                report("liked", str(e))
                cache = {}
            _state["liked"] = cache
        liked = cache.get(uri)
    publish_all(values_for(state, liked))
    return POLL_PLAYING if state and state.get("is_playing") else POLL_IDLE


_reported = {}


def report(key, error):
    """Log a repeating problem once, and once more when it goes away."""
    if _reported.get(key) == error:
        return
    if error is None:
        _reported.pop(key, None)
        plugin.log(f"{key}: working again")
    else:
        _reported[key] = error
        plugin.log(f"{key}: {error}", "warn")


_poll_lock = threading.Lock()


def safe_poll():
    if not _poll_lock.acquire(blocking=False):
        return
    try:
        if wait_left() > 0:
            return
        try:
            delay = poll()
        except SpotifyError as e:
            report("poll", str(e))
            if "slow down" not in str(e):
                publish_all(dict.fromkeys(VALUES, NOTHING))
            delay = POLL_IDLE
        else:
            report("poll", None)
        _state["next"] = time.monotonic() + max(delay, wait_left())
    finally:
        _poll_lock.release()


@plugin.every(1)
def tick():
    if time.monotonic() >= _state["next"] and wait_left() <= 0:
        safe_poll()


@plugin.on_settings
def settings_changed(settings):
    _state["next"] = 0.0
    _state["liked"] = {}


if __name__ == "__main__":
    plugin.run()
