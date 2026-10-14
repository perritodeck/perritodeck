"""Twitch: run a stream from the deck. Markers, ads, clips, chat, title/category/tags,
moderation, raids, shoutouts, channel points, polls and predictions, VIPs and mods;
live values from Helix polling and from EventSub over WebSocket (follows, subs, cheers,
raids, redemptions, hype trains, ad breaks, going live).

Uses the Twitch Helix API and EventSub WebSocket transport. Signs in with the Device
Code Grant flow as a public client: the user registers their own app (client type
"Public") and pastes its client ID; no client secret is needed. Which permissions the
sign-in asks for depends on the feature groups turned on in the plugin's settings.
Tokens are kept in the plugin's data folder, readable by the user only, refreshed
automatically, validated at start and hourly as Twitch requires, and revoked at
Twitch on sign-out. Standard library only.
"""

import base64
import hashlib
import json
import os
import random
import re
import select
import socket
import ssl
import struct
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timezone

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin, open_for_user  # noqa: E402

plugin = Plugin()
NOTHING = "–"
ID_URL = "https://id.twitch.tv"
HELIX_URL = "https://api.twitch.tv/helix"
EVENTSUB_URL = "wss://eventsub.wss.twitch.tv/ws?keepalive_timeout_seconds=30"
ACTIVATE_URL = "https://www.twitch.tv/activate"
TIMEOUT = 6
VALIDATE_SECONDS = 3600  # Twitch requires apps to validate their tokens at start and hourly
VALIDATE_RETRY = 60  # seconds before checking again when Twitch couldn't be reached
MIN_DEVICE_POLL = 1.0
TOKEN_FILE = "tokens.json"
STATE_FILE = "state.json"
DEVICE_GRANT = "urn:ietf:params:oauth:grant-type:device_code"
RECENT_SECONDS = 600  # a finished poll or prediction stays on keys this long
EVENTSUB_BACKOFF_FIRST = 2.0
EVENTSUB_BACKOFF_MAX = 60.0
EVENTSUB_READ_SECONDS = 5.0  # how often the session checks what keys want
EVENTSUB_RETRY_FIRST = 15.0  # a subscription Twitch refused is tried again after this,
EVENTSUB_RETRY_MAX = 600.0  # doubling up to this (and at once when a working session is lost)
WS_CONNECT_TIMEOUT = 8.0
WS_READ_TIMEOUT = 10.0  # for the rest of a frame that has started to arrive
KEEPALIVE_GRACE = 5.0  # extra seconds past the keepalive timeout before reconnecting
WS_POLL = 1.0  # seconds between checks while the EventSub connection is quiet
WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
MAX_MESSAGE = 1 << 20

# --- permissions -------------------------------------------------------------------------

# The base scopes are always asked for; each group can be turned off in the settings.
BASE_SCOPES = (
    "channel:manage:broadcast",  # markers, title, category, tags, labels
    "clips:edit",
    "user:write:chat",
    "user:manage:chat_color",
)
GROUPS = {
    "ads": ("Ads", (
        "channel:edit:commercial",
        "channel:read:ads",
        "channel:manage:ads",
    )),
    "moderation": ("Moderation", (
        "moderator:manage:chat_settings",
        "moderator:manage:shield_mode",
        "moderator:manage:banned_users",
        "moderator:manage:chat_messages",
        "moderator:manage:warnings",
        "moderator:manage:announcements",
        "moderator:manage:shoutouts",
        "moderator:manage:automod_settings",  # only sets the level: no read scope needed
        "moderator:read:chatters",
        "channel:manage:raids",
        "channel:manage:vips",
        "channel:manage:moderators",
    )),
    "points": ("Channel points", (
        "channel:manage:redemptions",
    )),
    "polls": ("Polls and predictions", (
        "channel:manage:polls",
        "channel:manage:predictions",
    )),
    "events": ("Events and totals", (
        "moderator:read:followers",
        "channel:read:subscriptions",
        "bits:read",
        "channel:read:hype_train",
        "channel:read:redemptions",
    )),
}
SCOPE_GROUP = {scope: group for group, (_, scopes) in GROUPS.items() for scope in scopes}


def group_on(group):
    value = (plugin.settings or {}).get("group_" + group)
    return True if value is None or value == "" else bool(value)


def wanted_scopes():
    scopes = list(BASE_SCOPES)
    for group, (_, group_scopes) in GROUPS.items():
        if group_on(group):
            scopes += [s for s in group_scopes if s not in scopes]
    return scopes


class TwitchError(RuntimeError):
    def __init__(self, message, status=None):
        super().__init__(message)
        self.status = status  # the HTTP status of Helix's error answer, when there was one


SIGN_IN_AGAIN = "press \"Sign in to Twitch\" again"


def missing_scope_message(scope):
    group = SCOPE_GROUP.get(scope)
    if group and not group_on(group):
        return f"sign in again with {GROUPS[group][0]} turned on in the plugin's settings"
    return f"the sign-in doesn't allow that ({scope}); {SIGN_IN_AGAIN}"


def need(*scopes):
    """Raise a clear message unless the sign-in has one of these scopes."""
    tokens = access_token()
    have = set(tokens.get("scope") or [])
    if not have.intersection(scopes):
        raise TwitchError(missing_scope_message(scopes[0]))
    return tokens


def has(*scopes):
    tokens = current_tokens() or {}
    return bool(set(tokens.get("scope") or []).intersection(scopes))


# --- settings and tokens -------------------------------------------------------------


def client_id():
    cid = ((plugin.settings or {}).get("client_id") or "").strip()
    if not cid:
        raise TwitchError("set your Twitch app's client ID on the Plugins page")
    return cid


_token_lock = threading.RLock()


def private_write(name, data):
    """Write a JSON file in the data folder readable by the user only (0600 where the
    system has modes), atomically."""
    path = os.path.join(plugin.data_dir, name)
    tmp = path + ".tmp"
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        json.dump(data, f)
    try:
        os.chmod(tmp, 0o600)
    except OSError:
        pass
    replace_file(tmp, path)


def replace_file(tmp, path, attempts=100):
    """os.replace, retried briefly: on Windows it fails ("Access is denied") while another
    thread or program has the file open, and losing a save right after a refresh would
    lose Twitch's new single-use refresh token."""
    for attempt in range(attempts):
        try:
            os.replace(tmp, path)
            return
        except PermissionError:
            if attempt == attempts - 1:
                raise
            time.sleep(0.02)


def read_json(name):
    try:
        with open(os.path.join(plugin.data_dir, name), encoding="utf-8") as f:
            data = json.load(f)
    except (OSError, ValueError):
        return None
    return data if isinstance(data, dict) else None


def token_path():
    return os.path.join(plugin.data_dir, TOKEN_FILE)


def load_tokens():
    tokens = read_json(TOKEN_FILE)
    return tokens if tokens and tokens.get("access_token") else None


def save_tokens(tokens):
    private_write(TOKEN_FILE, tokens)


def forget_tokens():
    try:
        os.remove(token_path())
    except OSError:
        pass
    _users.clear()


def current_tokens():
    tokens = load_tokens()
    cid = ((plugin.settings or {}).get("client_id") or "").strip()
    if tokens is None or tokens.get("client_id") != cid:
        return None
    return tokens


def id_request(path, form=None, token=None):
    """A request to id.twitch.tv: POST a form, or GET with an OAuth header. Returns
    (status, JSON) for 2xx and 4xx answers; raises TwitchError when unreachable."""
    data = urllib.parse.urlencode(form).encode() if form is not None else None
    req = urllib.request.Request(ID_URL + path, data=data, method="POST" if form is not None else "GET")
    if form is not None:
        req.add_header("Content-Type", "application/x-www-form-urlencoded")
    if token:
        req.add_header("Authorization", f"OAuth {token}")
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
            status, raw = resp.status, resp.read()
    except urllib.error.HTTPError as e:
        status, raw = e.code, error_body(e)
    except (urllib.error.URLError, OSError) as e:
        raise TwitchError(f"couldn't reach Twitch: {getattr(e, 'reason', e)}") from None
    try:
        body = json.loads(raw.decode("utf-8") or "{}")
    except ValueError:
        body = {}
    return status, body if isinstance(body, dict) else {}


def store_token_answer(answer, cid, old=None, user=None):
    if not answer.get("access_token"):
        raise TwitchError("Twitch didn't send an access token")
    old = old or {}
    tokens = {
        "client_id": cid,
        "access_token": answer["access_token"],
        # Refresh tokens are single use: always keep the newest.
        "refresh_token": answer.get("refresh_token") or old.get("refresh_token"),
        "expires_at": time.time() + int(answer.get("expires_in") or 3600),
        "scope": answer.get("scope") or old.get("scope") or [],
        "user_id": (user or {}).get("id") or old.get("user_id"),
        "login": (user or {}).get("login") or old.get("login"),
        "validated_at": time.time(),
    }
    save_tokens(tokens)
    return tokens


def refresh(tokens):
    with _token_lock:
        latest = current_tokens()
        if latest and latest.get("access_token") != tokens.get("access_token"):
            return latest  # another thread refreshed it already
        cid = client_id()
        if not tokens.get("refresh_token"):
            forget_tokens()
            raise TwitchError("Twitch ended the sign-in; " + SIGN_IN_AGAIN)
        status, answer = id_request("/oauth2/token", {
            "client_id": cid,
            "grant_type": "refresh_token",
            "refresh_token": tokens["refresh_token"],
        })
        if status in (400, 401):
            forget_tokens()
            raise TwitchError("Twitch ended the sign-in; " + SIGN_IN_AGAIN)
        if status != 200:
            raise TwitchError(f"Twitch couldn't refresh the sign-in ({status}: {answer.get('message') or ''})")
        plugin.log("Twitch sign-in refreshed")
        return store_token_answer(answer, cid, tokens)


def access_token(force_refresh=False):
    with _token_lock:
        client_id()
        tokens = current_tokens()
        if tokens is None:
            raise TwitchError("not signed in: press \"Sign in to Twitch\" first")
        if force_refresh or time.time() > float(tokens.get("expires_at") or 0) - 60:
            tokens = refresh(tokens)
        return tokens


def validate():
    """GET /oauth2/validate, as Twitch requires of every app at start and hourly. A token
    that fails is refreshed once (or the sign-in is dropped when that fails too, and
    the error says to sign in again). Returns the tokens."""
    tokens = access_token()
    status, answer = id_request("/oauth2/validate", token=tokens["access_token"])
    if status == 401:
        return access_token(force_refresh=True)
    if status != 200:
        raise TwitchError(f"Twitch couldn't check the sign-in ({status})")
    if isinstance(answer.get("scopes"), list):
        tokens["scope"] = answer["scopes"]
    if answer.get("login"):
        tokens["login"] = answer["login"]
    tokens["validated_at"] = time.time()
    with _token_lock:
        latest = current_tokens()
        if latest and latest.get("access_token") == tokens["access_token"]:
            save_tokens(tokens)
    return tokens


_check = {"next": 0.0, "told": False}  # monotonic time of the next validation; 0 = at start


def check_sign_in():
    """Validate the token when the plugin starts and every hour after, whether or not
    keys show Twitch values (Twitch requires it of every app). Says once which account
    is signed in, so a deck on the wrong channel is obvious in the log. Returns False
    once the sign-in is gone."""
    now = time.monotonic()
    if now < _check["next"]:
        return True
    _check["next"] = now + VALIDATE_RETRY  # until a check gets through
    try:
        tokens = validate()
    except TwitchError as e:
        report("sign-in", str(e))
        if current_tokens() is None:
            events.wake()  # closes the EventSub session
            return False
        return True  # Twitch couldn't be reached: keep the sign-in, try again later
    _check["next"] = now + VALIDATE_SECONDS
    report("sign-in", None)
    if not _check["told"]:
        _check["told"] = True
        plugin.log(f"signed in to Twitch as {tokens.get('login') or 'an unknown account'}")
    return True


# --- sign-in (device code grant) -------------------------------------------------------


def twitch_page(url):
    """Whether an address that came from Twitch is safe to open in the user's browser:
    https on twitch.tv or one of its subdomains, with nothing in it that a browser
    could read differently from Python (a backslash, spaces, a user name)."""
    if not isinstance(url, str) or not url or any(c.isspace() or c == "\\" or ord(c) < 32 or ord(c) == 127
                                                  for c in url):
        return False
    try:
        parts = urllib.parse.urlsplit(url)
        port = parts.port
    except ValueError:
        return False
    host = (parts.hostname or "").lower()
    return (parts.scheme == "https" and "@" not in parts.netloc and port in (None, 443)
            and (host == "twitch.tv" or host.endswith(".twitch.tv")))


class DeviceSignIn:
    def __init__(self, cid, answer, scopes):
        self.cid = cid
        self.scopes = scopes
        self.device_code = answer["device_code"]
        self.user_code = answer.get("user_code") or ""
        given = answer.get("verification_uri")
        self.uri = given if twitch_page(given) else ACTIVATE_URL
        self.odd_uri = given if given and given != self.uri else None
        interval = answer.get("interval")
        self.interval = float(5 if interval is None else interval)
        self.deadline = time.monotonic() + float(answer.get("expires_in") or 1800)
        self.done = threading.Event()
        self.cancelled = threading.Event()
        self.why_cancelled = ""
        self.error = None
        self.login = None  # the account that approved, once it has
        self.thread = threading.Thread(target=self.wait, name="twitch-sign-in", daemon=True)

    def cancel(self, why):
        self.why_cancelled = why
        self.cancelled.set()

    def wait(self):
        try:
            self.finish(self.poll())
        except TwitchError as e:
            self.finish(str(e))

    def poll(self):
        while time.monotonic() < self.deadline:
            if self.cancelled.wait(max(MIN_DEVICE_POLL, self.interval)):
                return self.why_cancelled
            status, answer = id_request("/oauth2/token", {
                "client_id": self.cid,
                "scopes": " ".join(self.scopes),
                "device_code": self.device_code,
                "grant_type": DEVICE_GRANT,
            })
            if status == 200:
                return self.approved(answer)
            message = str(answer.get("message") or answer.get("error") or status)
            if message == "authorization_pending":
                continue
            if message == "slow_down":
                self.interval += 5
                continue
            if "invalid device code" in message.lower() or "expired" in message.lower():
                return "the sign-in code expired; " + SIGN_IN_AGAIN
            return f"Twitch refused the sign-in ({message})"
        return "the sign-in code expired; " + SIGN_IN_AGAIN

    def approved(self, answer):
        """Save the sign-in Twitch approved, unless it was cancelled meanwhile: then it's
        ended at Twitch instead. Returns None once saved, else why it wasn't."""
        token = answer.get("access_token")
        if not token:
            raise TwitchError("Twitch didn't send an access token")
        try:
            user = None if self.cancelled.is_set() else me(token, self.cid)
        except TwitchError:
            self.end(token)  # no use here, so it mustn't go on working at Twitch
            raise
        if not answer.get("scope"):
            answer["scope"] = list(self.scopes)
        with _token_lock:
            # sign_out() cancels before it takes this lock, and keeps it while it ends the
            # sign-in: checked here, a sign-out pressed while the approval was on its way
            # (or while me() asked which account it is) always wins.
            if not self.cancelled.is_set():
                store_token_answer(answer, self.cid, user=user)
                _users.clear()
                self.login = user.get("login") or "an unknown account"
                # Name the account: approving in a browser signed in to another channel is
                # easy. Logged under the lock, so a sign-out right after is logged after it.
                plugin.log(f"signed in to Twitch as {self.login}")
                return None
        self.end(token)
        return self.why_cancelled

    def end(self, token):
        """End a token Twitch gave that won't be saved."""
        problem = revoke(self.cid, token)
        if problem:
            plugin.log(f"couldn't end an unused Twitch sign-in at Twitch ({problem}); to end it, remove the app "
                       "under Settings, Connections on twitch.tv", "warn")

    def finish(self, error=None):
        self.error = error
        if error:
            plugin.log(f"Twitch sign-in: {error}", "warn")
        else:
            _check.update(next=time.monotonic() + VALIDATE_SECONDS, told=True)  # a new token is valid
            reschedule()
            events.wake()
        self.done.set()


def me(token, cid):
    req = urllib.request.Request(HELIX_URL + "/users")
    req.add_header("Authorization", f"Bearer {token}")
    req.add_header("Client-Id", cid)
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
            data = json.loads(resp.read().decode("utf-8")).get("data") or []
    except (urllib.error.URLError, OSError, ValueError) as e:
        raise TwitchError(f"couldn't read your Twitch account: {getattr(e, 'reason', e)}") from None
    if not data:
        raise TwitchError("Twitch didn't say which account signed in")
    return data[0]


_sign_in = {"current": None}


@plugin.action("sign-in")
def sign_in(settings):
    cid = client_id()
    old = _sign_in["current"]
    if old is not None and not old.done.is_set():
        old.cancel("replaced by a new sign-in")
    scopes = wanted_scopes()
    status, answer = id_request("/oauth2/device", {"client_id": cid, "scopes": " ".join(scopes)})
    if status != 200 or not answer.get("device_code"):
        message = answer.get("message") or status
        if status == 400 and "client" in str(message).lower():
            raise TwitchError(f"Twitch doesn't know that client ID ({message}); check it, and that the app is \"Public\"")
        raise TwitchError(f"Twitch refused the sign-in ({message})")
    attempt = DeviceSignIn(cid, answer, scopes)
    _sign_in["current"] = attempt
    attempt.thread.start()
    if attempt.odd_uri:
        plugin.log(f"Twitch sent a sign-in page that isn't https on twitch.tv ({attempt.odd_uri[:200]!r}); "
                   f"using {ACTIVATE_URL} instead", "warn")
    shown = "twitch.tv/activate" if attempt.uri.startswith(ACTIVATE_URL) else attempt.uri
    plugin.log(f"to sign in to Twitch, open {attempt.uri} and enter the code {attempt.user_code}")
    try:
        open_for_user(attempt.uri)  # a browser it starts must outlive a plugin restart
    except OSError:
        pass  # the address is in the log and on the key
    return f"go to {shown} and enter {attempt.user_code}"


def end_at_twitch(tokens):
    """Revoke the sign-in at Twitch (POST /oauth2/revoke), so the token stops working
    everywhere and not only here. Returns None when it's ended (or Twitch had ended
    it already), else what went wrong."""
    if time.time() > float(tokens.get("expires_at") or 0) - 60 and current_tokens() is not None:
        # An expired token can't be revoked, and the refresh token behind it would
        # live on: refresh first to have one that can.
        try:
            tokens = refresh(tokens)
        except TwitchError as e:
            return None if load_tokens() is None else str(e)  # gone: Twitch had ended it
    return revoke(tokens.get("client_id"), tokens["access_token"])


def revoke(cid, token):
    """POST /oauth2/revoke for an access token. Returns None when it's ended (or Twitch
    had ended it already), else what went wrong."""
    try:
        status, answer = id_request("/oauth2/revoke", {"client_id": cid or "", "token": token})
    except TwitchError as e:
        return str(e)
    message = str(answer.get("message") or "")
    if status == 200 or (status == 400 and "invalid token" in message.lower()):
        return None
    return f"Twitch said {status}: {message}" if message else f"Twitch said {status}"


@plugin.action("sign-out")
def sign_out(settings):
    attempt = _sign_in["current"]
    if attempt is not None and not attempt.done.is_set():
        # It mustn't sign back in once approved: cancelled before taking the lock, which
        # approved() checks under.
        attempt.cancel("cancelled by signing out")
    with _token_lock:
        tokens = load_tokens()
        problem = end_at_twitch(tokens) if tokens else None
        forget_tokens()  # whatever Twitch said
    events.wake()
    publish_unknown()
    if problem:
        plugin.log(f"signed out of Twitch on this computer, but couldn't end the sign-in at Twitch ({problem}); "
                   "to end it there too, remove the app under Settings, Connections on twitch.tv", "warn")
        return "signed out here, but Twitch couldn't end the sign-in; see the log"
    plugin.log("signed out of Twitch")
    return None


# --- Helix -------------------------------------------------------------------------------

_limit = {"until": 0.0}


def wait_left():
    return max(0.0, _limit["until"] - time.time())


def note_rate_limit(headers, status):
    """Twitch sends Ratelimit-Remaining and Ratelimit-Reset (Unix seconds) with every
    answer. Stop before the bucket is empty, and after a 429 until it refills."""
    if headers is None:
        return
    try:
        remaining = int(headers.get("Ratelimit-Remaining"))
        reset = float(headers.get("Ratelimit-Reset"))
    except (TypeError, ValueError):
        remaining, reset = None, None
    if status == 429 and (remaining is None or remaining <= 2):
        # The bucket is empty. (Some endpoints also answer 429 for their own limits,
        # e.g. no ad snoozes left, with tokens still in the bucket: those don't wait.)
        _limit["until"] = reset if reset and reset > time.time() else time.time() + 5
    elif remaining is not None and remaining <= 2 and reset:
        _limit["until"] = reset


def api(method, path, params=None, body=None):
    """Call Helix; returns the JSON answer (None for 204)."""
    left = wait_left()
    if left > 0:
        raise TwitchError(f"Twitch asked to slow down; try again in {int(left) + 1} s")
    full = HELIX_URL + path
    if params:
        full += "?" + urllib.parse.urlencode(params, doseq=True)
    cid = client_id()
    for attempt in (0, 1):
        tokens = access_token(force_refresh=attempt == 1)
        data = json.dumps(body).encode() if body is not None else (b"" if method in ("POST", "PUT", "PATCH") else None)
        req = urllib.request.Request(full, data=data, method=method)
        req.add_header("Authorization", f"Bearer {tokens['access_token']}")
        req.add_header("Client-Id", cid)
        if body is not None:
            req.add_header("Content-Type", "application/json")
        try:
            with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
                note_rate_limit(resp.headers, resp.status)
                raw = resp.read()
        except urllib.error.HTTPError as e:
            note_rate_limit(e.headers, e.code)
            message = error_message(e)
            if e.code == 401 and attempt == 0 and token_rejected(message, tokens["access_token"]):
                continue  # expired or revoked: refresh once and try again
            raise TwitchError(describe(e.code, message), e.code) from None
        except (urllib.error.URLError, OSError) as e:
            raise TwitchError(f"couldn't reach Twitch: {getattr(e, 'reason', e)}") from None
        if not raw.strip():
            return None
        try:
            return json.loads(raw.decode("utf-8"))
        except ValueError:
            raise TwitchError("Twitch sent an answer that isn't JSON") from None
    return None


def error_body(e):
    """The body of an HTTP error answer, and the answer closed."""
    try:
        return e.read() or b""
    except Exception:
        return b""
    finally:
        try:
            e.close()
        except Exception:
            pass


def error_message(e):
    try:
        body = json.loads(error_body(e).decode("utf-8") or "{}")
        return str(body.get("message") or body.get("error") or "")
    except Exception:
        return ""


# How Helix words a 401 for a token that no longer works (expired, revoked, missing).
BAD_TOKEN = ("invalid oauth token", "invalid access token", "oauth token is missing", "token is expired",
             "token has expired")


def token_rejected(message, token):
    """Whether a Helix 401 means the token itself no longer works, so a refresh can
    help. Other 401s are about the request ("Missing scope: ...", "The ID in
    broadcaster_id must match the user ID found in the request's OAuth token"), and a
    refresh would only use up the single-use refresh token, on every poll of a value.
    For wording this doesn't know, it asks Twitch whether the token still works."""
    text = message.lower()
    if any(phrase in text for phrase in BAD_TOKEN):
        return True
    if "scope" in text or "must match" in text:
        return False
    try:
        status, _ = id_request("/oauth2/validate", token=token)
    except TwitchError:
        return False  # can't tell: show Twitch's message rather than refresh blindly
    return status == 401


def describe(status, message):
    if status == 429 and (wait_left() > 0 or not message):
        return f"Twitch asked to slow down; try again in {int(wait_left()) + 1} s"
    if status == 401 and "scope" in message.lower():
        scope = message.split(":", 1)[-1].strip() if "missing scope" in message.lower() else ""
        if scope in SCOPE_GROUP:
            return missing_scope_message(scope)
        return "the sign-in doesn't allow that (" + message + "); " + SIGN_IN_AGAIN
    if status == 401 and (not message or any(phrase in message.lower() for phrase in BAD_TOKEN)):
        return "Twitch refused the sign-in; " + SIGN_IN_AGAIN
    return f"Twitch said {status}: {message}" if message else f"Twitch said {status}"


def me_id():
    tokens = access_token()
    if not tokens.get("user_id"):
        raise TwitchError("the sign-in doesn't say which account it is; " + SIGN_IN_AGAIN)
    return tokens["user_id"]


def login_key(name):
    return (name or "").strip().lower().lstrip("@")


def is_me(name):
    return login_key(name) in ("", "me")


# Up to 25 letters, digits or _ (matched after login_key() lowercases). New accounts need
# 4 or more, but older ones may be shorter, so short names are left for Twitch to judge.
LOGIN_RE = re.compile(r"^[a-z0-9_]{1,25}$")


def login_problem(name, what="channel"):
    """Why a name can't be a Twitch login (up to 25 letters, digits or _), or None."""
    if LOGIN_RE.match(login_key(name)):
        return None
    return f"{name.strip().lstrip('@')!r} can't be a Twitch {what} name: those are up to 25 letters, digits or _"


_users = {}  # login -> user id, or None for a name Twitch refused


def refused(name):
    """Whether Twitch refused this name when it was looked up."""
    key = login_key(name)
    return key in _users and _users[key] is None


def user_ids(logins):
    """{login: id} for channel names ('me' or '' is the signed-in account); unknown names
    are left out. So are names no Twitch account can have (logged once), which aren't
    sent: in a batch, one of those would make Twitch refuse every name with it. And so
    are names Twitch refused anyway (see look_up)."""
    found = {}
    missing = []
    for login in logins:
        key = login_key(login)
        if key in ("", "me"):
            found[login] = me_id()
        elif key in _users:
            if _users[key] is not None:
                found[login] = _users[key]
        elif login_problem(login):
            report(f"name {login}", login_problem(login))
        else:
            missing.append((login, key))
    keys = sorted({key for _, key in missing})
    for i in range(0, len(keys), 100):
        look_up(keys[i:i + 100])
    for login, key in missing:
        if _users.get(key) is not None:
            found[login] = _users[key]
    return found


def look_up(keys):
    """Put the IDs of these names (up to 100) in _users with one GET /users. Twitch
    answers 400 for the whole request when it refuses one name in it, so then each name
    is asked about on its own: one bad name never blanks the others. A name Twitch
    refuses on its own is logged once and remembered, so it isn't asked about again."""
    try:
        answer = api("GET", "/users", [("login", key) for key in keys]) or {}
    except TwitchError as e:
        if e.status != 400:
            raise
        if len(keys) > 1:
            for key in keys:
                look_up([key])
        else:
            _users[keys[0]] = None
            report(f"name {keys[0]}", f"{keys[0]!r} isn't a name Twitch accepts ({e})")
        return
    for u in answer.get("data") or []:
        if u.get("id"):
            _users[u.get("login", "").lower()] = u["id"]


def user_id(name, what="channel"):
    name = (name or "").strip()
    if is_me(name):
        return me_id()
    problem = login_problem(name, what)
    if problem:
        raise TwitchError(problem)
    found = user_ids([name])
    if name not in found:
        if refused(name):
            raise TwitchError(f"Twitch doesn't accept {login_key(name)!r} as a {what} name")
        raise TwitchError(f"Twitch has no {what} called {name.lstrip('@')}")
    return found[name]


def channel_id(name):
    return user_id(name)


def needs_text(settings, key, what):
    text = (settings.get(key) or "").strip()
    if not text:
        raise TwitchError(f"enter {what}")
    return text


def first(answer):
    data = (answer or {}).get("data") or []
    return data[0] if data else {}


def want(settings, current, key="state"):
    state = (settings.get(key) or "toggle").strip().lower()
    if state in ("on", "add", "pause", "enable"):
        return True
    if state in ("off", "remove", "unpause", "resume", "disable"):
        return False
    return not current


def toggling(settings, key="state"):
    return (settings.get(key) or "toggle").strip().lower() in ("toggle", "")


def integer(value, what, low=None, high=None, default=None):
    if value in (None, ""):
        if default is None:
            raise TwitchError(f"enter {what}")
        return default
    try:
        n = int(float(value))
    except (TypeError, ValueError):
        raise TwitchError(f"{what} must be a number, not {value!r}") from None
    if (low is not None and n < low) or (high is not None and n > high):
        raise TwitchError(f"{what} must be {low} to {high}")
    return n


def split_list(text, sep=","):
    return [part.strip() for part in (text or "").split(sep) if part.strip()]


def poll_soon():
    timer = threading.Timer(1.0, lambda: safe_poll(force=True))
    timer.daemon = True
    timer.start()


# --- plugin state (last clip, last marker, last events, saved message turns) ------------

_state_lock = threading.Lock()
_state = {}


def load_state():
    with _state_lock:
        _state.clear()
        _state.update(read_json(STATE_FILE) or {})


def update_state(**changes):
    with _state_lock:
        _state.update(changes)
        snapshot = dict(_state)
    try:
        private_write(STATE_FILE, snapshot)
    except OSError as e:
        plugin.log(f"couldn't save the plugin's state: {e}", "warn")


def local_time(ts):
    return time.strftime("%H:%M", time.localtime(ts))


# --- actions: stream ---------------------------------------------------------------------


@plugin.action("marker")
def marker(settings):
    body = {"user_id": me_id()}
    description = (settings.get("description") or "").strip()
    if description:
        body["description"] = description[:140]
    info = first(api("POST", "/streams/markers", body=body))
    position = info.get("position_seconds")
    update_state(last_marker=clock(position) if position is not None else local_time(time.time()))
    publish_fixed()
    return None


def open_in_browser(url):
    """Open a page for the user, in a browser that outlives a plugin restart; False if
    none could be started."""
    try:
        open_for_user(url)
    except OSError:
        return False
    return True


@plugin.action("clip")
def clip(settings):
    info = first(api("POST", "/clips", {"broadcaster_id": channel_id(settings.get("channel"))}))
    edit_url = info.get("edit_url")
    odd = bool(edit_url) and not twitch_page(edit_url)
    if odd:  # never open whatever came back; only twitch.tv pages
        plugin.log(f"Twitch sent a clip editor address that isn't https on twitch.tv: {str(edit_url)[:200]!r}", "warn")
        edit_url = None
    if info.get("id"):
        update_state(last_clip_id=info["id"], last_clip_at=time.time(), last_clip_edit=edit_url)
        publish_fixed()
    if settings.get("open") and odd:
        return "made the clip, but didn't open the editor: Twitch's address for it isn't a twitch.tv page"
    if settings.get("open") and edit_url and not open_in_browser(edit_url):
        return "made the clip, but couldn't open a web browser"
    return None


@plugin.action("open-last-clip")
def open_last_clip(settings):
    clip_id = _state.get("last_clip_id")
    if not clip_id:
        return "no clip made from the deck yet"
    url = "https://clips.twitch.tv/" + urllib.parse.quote(str(clip_id), safe="")
    edit_url = _state.get("last_clip_edit")
    if settings.get("editor") and twitch_page(edit_url):
        url = edit_url
    if not open_in_browser(url):
        raise TwitchError("couldn't open a web browser")
    return None


def game_id(name):
    answer = api("GET", "/games", {"name": name}) or {}
    for game in answer.get("data") or []:
        if game.get("id"):
            return game["id"]
    raise TwitchError(f"Twitch has no category called {name!r}; use its exact name, as on twitch.tv")


TAG_RE = re.compile(r"^[^\s!\"#$%&'()*+,./:;<=>?@\[\\\]^`{|}~-]{1,25}$")


def check_tags(tags):
    if len(tags) > 10:
        raise TwitchError(f"Twitch allows up to 10 tags (that's {len(tags)})")
    for tag in tags:
        if not TAG_RE.match(tag):
            raise TwitchError(f"{tag!r} isn't a valid tag: up to 25 letters or digits, no spaces or symbols")
    return tags


def channel_update(title="", category="", tags=None):
    title, category = (title or "").strip(), (category or "").strip()
    if len(title) > 140:
        raise TwitchError(f"Twitch titles are up to 140 characters (this one is {len(title)})")
    body = {}
    if title:
        body["title"] = title
    if category:
        body["game_id"] = game_id(category)
    if tags is not None:
        body["tags"] = check_tags(tags)
    if not body:
        raise TwitchError("enter a title, a category or both")
    api("PATCH", "/channels", {"broadcaster_id": me_id()}, body)
    poll_soon()


@plugin.action("set-info")
def set_info(settings):
    channel_update(settings.get("title"), settings.get("category"))
    return None


def preset(number):
    raw = ((plugin.settings or {}).get(f"preset_{number}") or "").strip()
    if not raw:
        raise TwitchError(f"preset {number} is empty: fill it in on the Plugins page")
    parts = [p.strip() for p in raw.split("|")]
    while len(parts) < 3:
        parts.append("")
    title, category, tags = parts[0], parts[1], parts[2]
    return title, category, (split_list(tags) if tags else None)


@plugin.action("preset")
def apply_preset(settings):
    number = integer(settings.get("preset"), "the preset", 1, 5, default=1)
    title, category, tags = preset(number)
    channel_update(title, category, tags)
    return None


@plugin.action("tags")
def tags(settings):
    mode = (settings.get("mode") or "set").strip().lower()
    given = split_list(settings.get("tags"))
    if mode == "set":
        new = given
    else:
        if not given:
            raise TwitchError("enter one or more tags, separated by commas")
        current = first(api("GET", "/channels", {"broadcaster_id": me_id()})).get("tags") or []
        lower = {t.lower() for t in given}
        if mode == "add":
            new = current + [t for t in given if t.lower() not in {c.lower() for c in current}]
        else:
            new = [t for t in current if t.lower() not in lower]
    channel_update(tags=new)
    return None


LABELS = {
    "politics": "DebatedSocialIssuesAndPolitics",
    "drugs": "DrugsIntoxication",
    "gambling": "Gambling",
    "profanity": "ProfanityVulgarity",
    "sexual": "SexualThemes",
    "violence": "ViolentGraphic",
}


@plugin.action("label")
def label(settings):
    key = (settings.get("label") or "").strip().lower()
    label_id = LABELS.get(key) or next((v for v in LABELS.values() if v.lower() == key), None)
    if not label_id:
        raise TwitchError("pick a content label")
    current = None
    if toggling(settings):
        info = first(api("GET", "/channels", {"broadcaster_id": me_id()}))
        current = label_id in (info.get("content_classification_labels") or [])
    on = want(settings, current)
    api("PATCH", "/channels", {"broadcaster_id": me_id()},
        {"content_classification_labels": [{"id": label_id, "is_enabled": on}]})
    return None


@plugin.action("branded")
def branded(settings):
    current = None
    if toggling(settings):
        current = bool(first(api("GET", "/channels", {"broadcaster_id": me_id()})).get("is_branded_content"))
    api("PATCH", "/channels", {"broadcaster_id": me_id()}, {"is_branded_content": want(settings, current)})
    return None


# --- actions: ads --------------------------------------------------------------------------

AD_LENGTHS = (30, 60, 90, 120, 150, 180)


@plugin.action("ad")
def ad(settings):
    need("channel:edit:commercial")
    try:
        length = int(settings.get("length") or 30)
    except (TypeError, ValueError):
        raise TwitchError(f"the ad length is a number of seconds, not {settings.get('length')!r}") from None
    if length not in AD_LENGTHS:
        raise TwitchError("the ad length is 30, 60, 90, 120, 150 or 180 seconds")
    info = first(api("POST", "/channels/commercial", body={"broadcaster_id": me_id(), "length": length}))
    message = (info.get("message") or "").strip()
    _ads["cache"] = None
    poll_soon()
    return message or None


@plugin.action("snooze-ad")
def snooze_ad(settings):
    need("channel:manage:ads")
    info = first(api("POST", "/channels/ads/schedule/snooze", {"broadcaster_id": me_id()}))
    _ads["cache"] = None
    poll_soon()
    if info and str(info.get("snooze_count")) == "0":
        return "snoozed; that was the last snooze for now"
    return None


# --- actions: chat -------------------------------------------------------------------------


def send_chat(channel, message):
    sender = me_id()
    info = first(api("POST", "/chat/messages", body={
        "broadcaster_id": channel_id(channel),
        "sender_id": sender,
        "message": message[:500],
    }))
    if info and info.get("is_sent") is False:
        reason = (info.get("drop_reason") or {}).get("message") or "no reason given"
        raise TwitchError(f"Twitch didn't send it: {reason}")


@plugin.action("chat")
def chat(settings):
    send_chat(settings.get("channel"), needs_text(settings, "message", "the message to send"))
    return None


@plugin.action("chat-saved")
def chat_saved(settings):
    messages = split_list(settings.get("messages"), "|")
    if not messages:
        raise TwitchError("enter one or more messages, separated by |")
    if (settings.get("order") or "cycle") == "random":
        message = random.choice(messages)
    else:
        key = "turn_" + hashlib.sha1("|".join(messages).encode()).hexdigest()[:12]
        turn = int(_state.get(key) or 0) % len(messages)
        message = messages[turn]
        update_state(**{key: turn + 1})
    send_chat(settings.get("channel"), message)
    return None


CHAT_COLOURS = ("blue", "blue_violet", "cadet_blue", "chocolate", "coral", "dodger_blue", "firebrick",
                "golden_rod", "green", "hot_pink", "orange_red", "red", "sea_green", "spring_green", "yellow_green")


@plugin.action("chat-color")
def chat_color(settings):
    color = needs_text(settings, "color", "a colour").lower().replace(" ", "_")
    if not (color in CHAT_COLOURS or re.match(r"^#[0-9a-f]{6}$", color)):
        raise TwitchError(f"the colour is one of {', '.join(CHAT_COLOURS)}, or #RRGGBB with Turbo or Prime")
    api("PUT", "/chat/color", {"user_id": me_id(), "color": color})
    return None


ANNOUNCE_COLOURS = ("primary", "blue", "green", "orange", "purple")


@plugin.action("announce")
def announce(settings):
    need("moderator:manage:announcements")
    message = needs_text(settings, "message", "the announcement")
    color = (settings.get("color") or "primary").strip().lower()
    if color not in ANNOUNCE_COLOURS:
        raise TwitchError("the colour is primary, blue, green, orange or purple")
    api("POST", "/chat/announcements",
        {"broadcaster_id": channel_id(settings.get("channel")), "moderator_id": me_id()},
        {"message": message[:500], "color": color})
    return None


# --- actions: moderation ---------------------------------------------------------------------

MODES = {
    "emote": "emote_mode", "emotes": "emote_mode", "emote-only": "emote_mode",
    "followers": "follower_mode", "follower": "follower_mode", "followers-only": "follower_mode",
    "slow": "slow_mode",
    "subscribers": "subscriber_mode", "subscriber": "subscriber_mode", "subs": "subscriber_mode",
    "subscribers-only": "subscriber_mode",
    "unique": "unique_chat_mode",
    "delay": "non_moderator_chat_delay",
}


def chat_field(mode):
    field = MODES.get((mode or "").strip().lower())
    if not field:
        raise TwitchError("the chat mode is emote, followers, slow, subscribers, unique or delay")
    return field


def mod_params(settings):
    return {"broadcaster_id": channel_id(settings.get("channel")), "moderator_id": me_id()}


@plugin.action("chat-mode")
def chat_mode(settings):
    need("moderator:manage:chat_settings")
    field = chat_field(settings.get("mode"))
    params = mod_params(settings)
    current = None
    if toggling(settings):
        current = bool(first(api("GET", "/chat/settings", params)).get(field))
    on = want(settings, current)
    body = {field: on}
    duration = settings.get("duration")
    if on and duration not in (None, ""):
        if field == "slow_mode":
            body["slow_mode_wait_time"] = integer(duration, "slow mode's wait (seconds)", 3, 120)
        elif field == "follower_mode":
            body["follower_mode_duration"] = integer(duration, "the minutes followed", 0, 129600)
        elif field == "non_moderator_chat_delay":
            seconds = integer(duration, "the chat delay", 2, 6)
            if seconds not in (2, 4, 6):
                raise TwitchError("the chat delay is 2, 4 or 6 seconds")
            body["non_moderator_chat_delay_duration"] = seconds
    elif on and field == "non_moderator_chat_delay":
        body["non_moderator_chat_delay_duration"] = 2
    api("PATCH", "/chat/settings", params, body)
    poll_soon()
    return None


@plugin.action("shield")
def shield(settings):
    need("moderator:manage:shield_mode")
    params = mod_params(settings)
    current = None
    if toggling(settings):
        current = bool(first(api("GET", "/moderation/shield_mode", params)).get("is_active"))
    api("PUT", "/moderation/shield_mode", params, {"is_active": want(settings, current)})
    return None


BAN_DEFAULT = 600  # the manifest's default: a 10 minute timeout
BAN_FOREVER = 0  # "Ban (no end)" in the manifest: only when picked


@plugin.action("ban")
def ban(settings):
    need("moderator:manage:banned_users")
    target = user_id(needs_text(settings, "user", "the user's name"), "user")
    # No duration saved (an older key, a script) is the 10 minute timeout, never a ban
    # with no end: that has to be picked.
    duration = integer(settings.get("duration"), "the timeout", 0, 1209600, default=BAN_DEFAULT)
    data = {"user_id": target}
    if duration != BAN_FOREVER:
        data["duration"] = duration
    reason = (settings.get("reason") or "").strip()
    if reason:
        data["reason"] = reason[:500]
    api("POST", "/moderation/bans", mod_params(settings), {"data": data})
    return None


@plugin.action("unban")
def unban(settings):
    need("moderator:manage:banned_users")
    target = user_id(needs_text(settings, "user", "the user's name"), "user")
    api("DELETE", "/moderation/bans", dict(mod_params(settings), user_id=target))
    return None


@plugin.action("warn")
def warn(settings):
    need("moderator:manage:warnings")
    target = user_id(needs_text(settings, "user", "the user's name"), "user")
    reason = needs_text(settings, "reason", "the reason (Twitch shows it to the user)")
    api("POST", "/moderation/warnings", mod_params(settings), {"data": {"user_id": target, "reason": reason[:500]}})
    return None


@plugin.action("clear-chat")
def clear_chat(settings):
    need("moderator:manage:chat_messages")
    api("DELETE", "/moderation/chat", mod_params(settings))
    return None


@plugin.action("raid")
def raid(settings):
    need("channel:manage:raids")
    target = user_id(needs_text(settings, "channel", "the channel to raid"))
    api("POST", "/raids", {"from_broadcaster_id": me_id(), "to_broadcaster_id": target})
    return None


@plugin.action("cancel-raid")
def cancel_raid(settings):
    need("channel:manage:raids")
    api("DELETE", "/raids", {"broadcaster_id": me_id()})
    return None


@plugin.action("shoutout")
def shoutout(settings):
    need("moderator:manage:shoutouts")
    target = user_id(needs_text(settings, "channel", "the channel to shout out"))
    api("POST", "/chat/shoutouts", {"from_broadcaster_id": me_id(), "to_broadcaster_id": target, "moderator_id": me_id()})
    return None


@plugin.action("automod")
def automod(settings):
    need("moderator:manage:automod_settings")
    level = integer(settings.get("level"), "the AutoMod level", 0, 4, default=1)
    api("PUT", "/moderation/automod/settings", mod_params(settings), {"overall_level": level})
    return None


def role(settings, path, scope, what):
    need(scope)
    target = user_id(needs_text(settings, "user", "the user's name"), "user")
    params = {"broadcaster_id": me_id(), "user_id": target}
    current = None
    if toggling(settings, "change"):
        current = bool((api("GET", path, params) or {}).get("data"))
    if want(settings, current, "change"):
        api("POST", path, params)
    else:
        api("DELETE", path, params)
    return None


@plugin.action("vip")
def vip(settings):
    return role(settings, "/channels/vips", "channel:manage:vips", "VIP")


@plugin.action("mod")
def mod(settings):
    return role(settings, "/moderation/moderators", "channel:manage:moderators", "moderator")


# --- actions: channel points ---------------------------------------------------------------

POINTS_ONLY_OWN = ("Twitch only lets the app that made a reward change it or its redemptions; "
                   "make the reward with \"Create a channel points reward\"")


def find_reward(title, manageable=False):
    need("channel:manage:redemptions", "channel:read:redemptions")
    title = (title or "").strip()
    if not title:
        raise TwitchError("enter the reward's title")
    params = {"broadcaster_id": me_id()}
    if manageable:
        params["only_manageable_rewards"] = "true"
    rewards = (api("GET", "/channel_points/custom_rewards", params) or {}).get("data") or []
    for reward in rewards:
        if (reward.get("title") or "").strip().lower() == title.lower():
            return reward
    if manageable:
        others = (api("GET", "/channel_points/custom_rewards", {"broadcaster_id": me_id()}) or {}).get("data") or []
        if any((r.get("title") or "").strip().lower() == title.lower() for r in others):
            raise TwitchError(POINTS_ONLY_OWN)
    raise TwitchError(f"there's no channel points reward called {title!r}")


def points_call(method, path, params, body=None):
    try:
        return api(method, path, params, body)
    except TwitchError as e:
        if "403" in str(e):
            raise TwitchError(POINTS_ONLY_OWN) from None
        raise


@plugin.action("reward-create")
def reward_create(settings):
    need("channel:manage:redemptions")
    title = needs_text(settings, "title", "the reward's title")[:45]
    body = {"title": title, "cost": integer(settings.get("cost"), "the cost", 1, 2147483647, default=100)}
    prompt = (settings.get("prompt") or "").strip()
    if prompt:
        body["prompt"] = prompt[:200]
    if settings.get("input"):
        body["is_user_input_required"] = True
    api("POST", "/channel_points/custom_rewards", {"broadcaster_id": me_id()}, body)
    _rewards["at"] = 0.0
    return None


@plugin.action("reward")
def reward(settings):
    need("channel:manage:redemptions")
    found = find_reward(settings.get("reward"), manageable=True)
    what = (settings.get("change") or "toggle-pause").strip().lower()
    if what in ("pause", "unpause", "toggle-pause"):
        body = {"is_paused": what == "pause" if what != "toggle-pause" else not found.get("is_paused")}
    elif what in ("enable", "disable", "toggle-enable"):
        body = {"is_enabled": what == "enable" if what != "toggle-enable" else not found.get("is_enabled")}
    elif what == "cost":
        body = {"cost": integer(settings.get("cost"), "the cost", 1, 2147483647)}
    else:
        raise TwitchError("pick what to change")
    points_call("PATCH", "/channel_points/custom_rewards", {"broadcaster_id": me_id(), "id": found["id"]}, body)
    _rewards["at"] = 0.0
    poll_soon()
    return None


@plugin.action("redemption")
def redemption(settings):
    need("channel:manage:redemptions")
    found = find_reward(settings.get("reward"), manageable=True)
    params = {"broadcaster_id": me_id(), "reward_id": found["id"]}
    waiting = (points_call("GET", "/channel_points/custom_rewards/redemptions",
                           dict(params, status="UNFULFILLED", sort="OLDEST", first=1)) or {}).get("data") or []
    if not waiting:
        return "no redemptions waiting"
    status = "CANCELED" if (settings.get("outcome") or "fulfil") == "cancel" else "FULFILLED"
    points_call("PATCH", "/channel_points/custom_rewards/redemptions", dict(params, id=waiting[0]["id"]),
                {"status": status})
    _rewards["at"] = 0.0
    poll_soon()
    return None


# --- actions: polls and predictions --------------------------------------------------------


def current(kind):
    """The newest poll or prediction (kind 'polls' or 'predictions')."""
    return first(api("GET", f"/{kind}", {"broadcaster_id": me_id(), "first": 1}))


@plugin.action("poll-start")
def poll_start(settings):
    need("channel:manage:polls")
    title = needs_text(settings, "title", "the question")
    choices = split_list(settings.get("choices"))
    if not 2 <= len(choices) <= 5:
        raise TwitchError("a poll has 2 to 5 choices, separated by commas")
    if len(title) > 60 or any(len(c) > 25 for c in choices):
        raise TwitchError("poll questions are up to 60 characters, choices up to 25")
    duration = integer(settings.get("duration"), "the poll's length", 15, 1800, default=60)
    api("POST", "/polls", body={"broadcaster_id": me_id(), "title": title,
                                "choices": [{"title": c} for c in choices], "duration": duration})
    poll_soon()
    return None


@plugin.action("poll-end")
def poll_end(settings):
    need("channel:manage:polls")
    poll = current("polls")
    if poll.get("status") != "ACTIVE":
        return "no poll is running"
    status = "ARCHIVED" if (settings.get("how") or "end") == "archive" else "TERMINATED"
    api("PATCH", "/polls", body={"broadcaster_id": me_id(), "id": poll["id"], "status": status})
    poll_soon()
    return None


@plugin.action("prediction-start")
def prediction_start(settings):
    need("channel:manage:predictions")
    title = needs_text(settings, "title", "the question")
    outcomes = split_list(settings.get("outcomes"))
    if not 2 <= len(outcomes) <= 10:
        raise TwitchError("a prediction has 2 to 10 outcomes, separated by commas")
    if len(title) > 45 or any(len(o) > 25 for o in outcomes):
        raise TwitchError("prediction questions are up to 45 characters, outcomes up to 25")
    window = integer(settings.get("window"), "the time to predict", 30, 1800, default=120)
    api("POST", "/predictions", body={"broadcaster_id": me_id(), "title": title,
                                      "outcomes": [{"title": o} for o in outcomes], "prediction_window": window})
    poll_soon()
    return None


def open_prediction():
    need("channel:manage:predictions")
    prediction = current("predictions")
    if prediction.get("status") not in ("ACTIVE", "LOCKED"):
        raise TwitchError("no prediction is open")
    return prediction


def end_prediction(prediction, status, **extra):
    api("PATCH", "/predictions", body=dict({"broadcaster_id": me_id(), "id": prediction["id"], "status": status}, **extra))
    poll_soon()


@plugin.action("prediction-lock")
def prediction_lock(settings):
    prediction = open_prediction()
    if prediction["status"] == "LOCKED":
        return "the prediction is already locked"
    end_prediction(prediction, "LOCKED")
    return None


@plugin.action("prediction-resolve")
def prediction_resolve(settings):
    prediction = open_prediction()
    name = needs_text(settings, "outcome", "the winning outcome")
    outcomes = prediction.get("outcomes") or []
    winner = next((o for o in outcomes if (o.get("title") or "").strip().lower() == name.lower()), None)
    if winner is None and name.isdigit() and 1 <= int(name) <= len(outcomes):
        winner = outcomes[int(name) - 1]
    if winner is None:
        titles = ", ".join(o.get("title") or "?" for o in outcomes)
        raise TwitchError(f"the prediction has no outcome {name!r}; it has {titles}")
    end_prediction(prediction, "RESOLVED", winning_outcome_id=winner["id"])
    return None


@plugin.action("prediction-cancel")
def prediction_cancel(settings):
    end_prediction(open_prediction(), "CANCELED")
    return None


# --- WebSocket client (RFC 6455), from the OBS plugin in this repository -------------------

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


class WebSocket:
    def __init__(self, sock):
        self.sock = sock
        self._buf = bytearray()
        self._send_lock = threading.Lock()
        self._kind, self._parts, self._size = None, [], 0  # a fragmented message so far

    @classmethod
    def connect(cls, url, timeout=WS_CONNECT_TIMEOUT):
        parts = urllib.parse.urlsplit(url)
        if parts.scheme not in ("ws", "wss") or not parts.hostname:
            raise ConnectionError(f"not a WebSocket address: {url}")
        secure = parts.scheme == "wss"
        port = parts.port or (443 if secure else 80)
        path = (parts.path or "/") + ("?" + parts.query if parts.query else "")
        sock = socket.create_connection((parts.hostname, port), timeout=timeout)
        try:
            sock.setsockopt(socket.SOL_SOCKET, socket.SO_KEEPALIVE, 1)
            if secure:
                sock = ssl.create_default_context().wrap_socket(sock, server_hostname=parts.hostname)
            ws = cls(sock)
            ws._handshake(parts.hostname, port, path, secure)
            return ws
        except BaseException:
            sock.close()
            raise

    def _handshake(self, host, port, path, secure):
        key = base64.b64encode(os.urandom(16)).decode()
        name = host if port == (443 if secure else 80) else f"{host}:{port}"
        request = (
            f"GET {path} HTTP/1.1\r\n"
            f"Host: {name}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n"
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

    def settimeout(self, seconds):
        self.sock.settimeout(seconds)

    def readable(self, timeout):
        """True when there's something to read: a whole frame buffered here, decrypted
        bytes waiting in TLS, or data on the socket (waits up to `timeout` for that)."""
        if self._frame_size() is not None or (hasattr(self.sock, "pending") and self.sock.pending()):
            return True
        return bool(select.select([self.sock], [], [], timeout)[0])

    def _fill(self):
        chunk = self.sock.recv(65536)
        if not chunk:
            raise WebSocketClosed(1006, "connection lost")
        self._buf += chunk

    def _frame_size(self):
        """(header length, payload length) of the frame at the start of the buffer once
        all of it has arrived, else None. Raises for a frame a server mustn't send."""
        buf = self._buf
        if len(buf) < 2:
            return None
        b0, b1 = buf[0], buf[1]
        if b0 & 0x70:
            raise WebSocketClosed(1002, "the server set reserved frame bits")
        if b1 & 0x80:
            raise WebSocketClosed(1002, "the server sent a masked frame")
        n, head = b1 & 0x7F, 2
        if b0 & 0x08 and (not b0 & 0x80 or n > 125):
            raise WebSocketClosed(1002, "a bad control frame")
        if n == 126:
            if len(buf) < 4:
                return None
            (n,), head = struct.unpack_from("!H", buf, 2), 4
        elif n == 127:
            if len(buf) < 10:
                return None
            (n,), head = struct.unpack_from("!Q", buf, 2), 10
        if n > MAX_MESSAGE:
            raise WebSocketClosed(1009, "message too big")
        return (head, n) if len(buf) >= head + n else None

    def _take_frame(self):
        """(fin, opcode, payload) of the next frame, or None until all of it has arrived.
        Nothing leaves the buffer before that, so a timeout waiting for the rest of a
        frame loses nothing."""
        size = self._frame_size()
        if size is None:
            return None
        head, n = size
        b0 = self._buf[0]
        payload = bytes(self._buf[head:head + n])
        del self._buf[:head + n]
        return bool(b0 & 0x80), b0 & 0x0F, payload

    def _message(self, fin, opcode, payload):
        """Handle one frame: answer a ping, raise WebSocketClosed on close, collect a
        message's fragments. Returns the text of the message it completes, else None."""
        if opcode == OP_PING:
            self.send_frame(OP_PONG, payload)
            return None
        if opcode == OP_PONG:
            return None
        if opcode == OP_CLOSE:
            code = struct.unpack("!H", payload[:2])[0] if len(payload) >= 2 else 1005
            reason = payload[2:].decode("utf-8", "replace")
            try:
                self.send_frame(OP_CLOSE, payload[:2])
            except OSError:
                pass
            raise WebSocketClosed(code, reason)
        if opcode in (OP_TEXT, OP_BINARY):
            if self._kind is not None:
                raise WebSocketClosed(1002, "a new message started inside a fragmented one")
            self._kind, self._parts, self._size = opcode, [], 0
        elif opcode == OP_CONT:
            if self._kind is None:
                raise WebSocketClosed(1002, "a continuation frame came with no message")
        else:
            raise WebSocketClosed(1002, f"unknown frame type {opcode}")
        self._parts.append(payload)
        self._size += len(payload)
        if self._size > MAX_MESSAGE:
            raise WebSocketClosed(1009, "message too big")
        if not fin:
            return None
        text = b"".join(self._parts).decode("utf-8")
        self._kind, self._parts, self._size = None, [], 0
        return text

    def send_frame(self, opcode, payload, fin=True):
        frame = encode_frame(opcode, payload, fin=fin)
        with self._send_lock:
            self.sock.sendall(frame)

    def recv(self):
        """The next text message, waiting for it up to the socket's timeout (which raises
        socket.timeout and loses nothing). Answers pings; raises WebSocketClosed on close."""
        while True:
            frame = self._take_frame()
            if frame is None:
                self._fill()
                continue
            text = self._message(*frame)
            if text is not None:
                return text

    def poll(self):
        """One frame of what has arrived, for when readable() is True: the text of the
        message it completes, or None (a ping, answered here; a pong; part of a longer
        message; or a frame still arriving). It never waits for another frame, so a
        ping can't leave the reader waiting on the socket for a message that only comes
        with the next keepalive. A socket timeout here (TLS had nothing for us after
        all) loses nothing."""
        frame = self._take_frame()
        if frame is None:
            self._fill()
            frame = self._take_frame()
            if frame is None:
                return None
        return self._message(*frame)

    def close(self, code=1000):
        try:
            self.send_frame(OP_CLOSE, struct.pack("!H", code))
        except OSError:
            pass
        self.abort()

    def abort(self):
        try:
            self.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        try:
            self.sock.close()
        except OSError:
            pass


# --- EventSub over WebSocket -----------------------------------------------------------------

# value -> the subscriptions it needs: (type, version, scope needed or None)
EVENT_SUBS = {
    "last_follower": [("channel.follow", "2", "moderator:read:followers")],
    "session_follows": [("channel.follow", "2", "moderator:read:followers")],
    "last_subscriber": [("channel.subscribe", "1", "channel:read:subscriptions"),
                        ("channel.subscription.message", "1", "channel:read:subscriptions")],
    "session_subs": [("channel.subscribe", "1", "channel:read:subscriptions"),
                     ("channel.subscription.message", "1", "channel:read:subscriptions")],
    "last_gift": [("channel.subscription.gift", "1", "channel:read:subscriptions")],
    "last_cheer": [("channel.cheer", "1", "bits:read")],
    "last_raid": [("channel.raid", "1", None)],
    "last_redemption": [("channel.channel_points_custom_reward_redemption.add", "1",
                         ("channel:read:redemptions", "channel:manage:redemptions"))],
    "hype_train": [("channel.hype_train.begin", "2", "channel:read:hype_train"),
                   ("channel.hype_train.progress", "2", "channel:read:hype_train"),
                   ("channel.hype_train.end", "2", "channel:read:hype_train")],
    "ad_live": [("channel.ad_break.begin", "1", "channel:read:ads")],
    "live": [("stream.online", "1", None), ("stream.offline", "1", None)],
}
EVENT_VALUES = ("last_follower", "last_subscriber", "last_gift", "last_cheer", "last_raid", "last_redemption",
                "hype_train", "session_follows", "session_subs")


def condition(sub_type, broadcaster, moderator):
    if sub_type == "channel.follow":
        return {"broadcaster_user_id": broadcaster, "moderator_user_id": moderator}
    if sub_type == "channel.raid":
        return {"to_broadcaster_user_id": broadcaster}
    return {"broadcaster_user_id": broadcaster}


def sub_key(sub_type, version, cond):
    return (sub_type, version, json.dumps(cond, sort_keys=True))


def wanted_subscriptions():
    """{key: (type, version, condition, [args])} for what keys show right now."""
    if not group_on("events") or current_tokens() is None:
        return {}
    try:
        moderator = me_id()
    except TwitchError:
        return {}
    args = {arg for value_id in EVENT_SUBS for arg in plugin.watched(value_id)}
    try:
        ids = user_ids(sorted(args)) if args else {}
    except TwitchError as e:
        report("events", str(e))
        return {}
    wanted = {}
    for value_id, subs in EVENT_SUBS.items():
        for arg in plugin.watched(value_id):
            if arg not in ids:
                continue
            for sub_type, version, scope in subs:
                if scope and not has(*(scope if isinstance(scope, tuple) else (scope,))):
                    continue
                cond = condition(sub_type, ids[arg], moderator)
                key = sub_key(sub_type, version, cond)
                entry = wanted.setdefault(key, (sub_type, version, cond, []))
                if arg not in entry[3]:
                    entry[3].append(arg)
    return wanted


class EventSub:
    """One EventSub WebSocket session on a background thread: subscribes to what keys
    show, follows reconnect messages, and reconnects (and resubscribes) when the
    connection drops or the messages stop (keepalives included) for longer than the
    keepalive time Twitch gave in its welcome. Subscriptions Twitch refused are tried
    again later, waiting longer each time, and at once on the connection that replaces
    a lost session that had working ones. While Twitch refuses every one, no session
    is kept open (Twitch closes a session with none 10 s after its welcome) and a new
    one is opened only when the next try is due."""

    def __init__(self):
        self._wake = threading.Event()
        self._thread = None
        self._lock = threading.Lock()
        self.ws = None
        self.session = None
        self.keepalive = 10.0
        self.last_message = 0.0
        self.subscribed = {}  # key -> subscription id
        self.failed = {}  # key -> (monotonic time to try again, seconds waited last)
        self.args = {}  # key -> [args]
        self.seen = []  # recent message ids (Twitch may send one twice)
        self.connected = False
        self.stopped = False

    def wake(self):
        with self._lock:
            if self._thread is None or not self._thread.is_alive():
                self._thread = threading.Thread(target=self.run, name="twitch-eventsub", daemon=True)
                self._thread.start()
        self.failed = {}  # keys or settings changed: try everything again now
        self._wake.set()

    def stop(self):
        """Stop the session thread (it closes the connection itself)."""
        self.stopped = True
        self._wake.set()
        ws = self.ws
        if ws is not None:
            ws.abort()  # wakes a read in progress

    def drop(self):
        if self.ws is not None:
            self.ws.close()
        worked = bool(self.subscribed)
        self.ws, self.session, self.connected = None, None, False
        self.subscribed = {}
        if worked:
            # A session that worked is lost: the next one tries the refused subscriptions
            # at once, keeping their backoff. Not when nothing on it was subscribed: Twitch
            # closes such a session after 10 s, which mustn't turn the backoff into a new
            # session and a refused POST every few seconds.
            self.failed = {key: (0.0, waited) for key, (_, waited) in self.failed.items()}

    def retry_due(self, wanted):
        now = time.monotonic()
        return any(key in wanted and at <= now for key, (at, _) in list(self.failed.items()))

    def next_retry(self, wanted):
        """When every subscription keys want is one Twitch refused and none is due to be
        tried again yet, the monotonic time the first one is; else None."""
        now = time.monotonic()
        failed = self.failed
        times = [failed[key][0] if key in failed else 0.0 for key in wanted]
        return min(times) if times and min(times) > now else None

    def run(self):
        backoff = EVENTSUB_BACKOFF_FIRST
        while not self.stopped:
            try:
                wanted = wanted_subscriptions()
            except Exception as e:
                report("events", f"failed: {e}")
                wanted = {}
            self.args = {key: entry[3] for key, entry in wanted.items()}
            if not wanted:
                if self.ws is not None:
                    self.drop()
                self._wake.wait(5)
                self._wake.clear()
                continue
            try:
                if self.ws is None:
                    retry_at = self.next_retry(wanted)
                    if retry_at is not None:
                        # Twitch refused every subscription: a session opened now would
                        # have none. Wait for the first retry (or for keys to change).
                        self._wake.wait(max(0.0, retry_at - time.monotonic()))
                        self._wake.clear()
                        continue
                    self.open(EVENTSUB_URL)
                    self.sync(wanted)
                    backoff = EVENTSUB_BACKOFF_FIRST
                    report("events", None)
                    if not self.subscribed:
                        self.drop()  # refused, every one: Twitch would close it after 10 s
                        continue
                elif self._wake.is_set() or self.retry_due(wanted):
                    self._wake.clear()
                    self.sync(wanted)
                self.read_for(EVENTSUB_READ_SECONDS)
            except (OSError, ConnectionError, ValueError, TwitchError) as e:
                if self.stopped:
                    break
                if self.connected:
                    backoff = EVENTSUB_BACKOFF_FIRST  # it worked until now: retry soon
                self.drop()
                report("events", f"EventSub connection: {e}")
                self._wake.wait(backoff)
                self._wake.clear()
                backoff = min(EVENTSUB_BACKOFF_MAX, backoff * 2)
        self.drop()

    def open(self, url):
        ws = WebSocket.connect(url)
        ws.settimeout(WS_READ_TIMEOUT)
        try:
            welcome = json.loads(ws.recv())
        except BaseException:
            ws.abort()
            raise
        if (welcome.get("metadata") or {}).get("message_type") != "session_welcome":
            ws.abort()
            raise ConnectionError("Twitch didn't send a welcome message")
        session = (welcome.get("payload") or {}).get("session") or {}
        self.ws, self.session = ws, session.get("id")
        self.keepalive = float(session.get("keepalive_timeout_seconds") or 10)
        self.last_message = time.monotonic()
        self.connected = True

    def sync(self, wanted):
        """Create missing subscriptions (the first within Twitch's 10 s after the welcome),
        delete ones no key shows any more. One Twitch refused waits its turn to be tried
        again (EVENTSUB_RETRY_FIRST, doubling up to EVENTSUB_RETRY_MAX)."""
        failed = self.failed
        for key in [k for k in failed if k not in wanted]:
            failed.pop(key, None)
        for key, (sub_type, version, cond, _) in wanted.items():
            if key in self.subscribed or failed.get(key, (0.0, 0.0))[0] > time.monotonic():
                continue
            try:
                answer = api("POST", "/eventsub/subscriptions", body={
                    "type": sub_type, "version": version, "condition": cond,
                    "transport": {"method": "websocket", "session_id": self.session}})
                self.subscribed[key] = first(answer).get("id")
                failed.pop(key, None)
                report(f"event {sub_type}", None)
            except TwitchError as e:
                if "409" in str(e):  # already exists on this session
                    self.subscribed[key] = None
                    failed.pop(key, None)
                    continue
                waited = failed.get(key, (0.0, 0.0))[1]
                wait = min(EVENTSUB_RETRY_MAX, waited * 2) if waited else EVENTSUB_RETRY_FIRST
                failed[key] = (time.monotonic() + wait, wait)
                report(f"event {sub_type}", str(e))
        for key in [k for k in self.subscribed if k not in wanted]:
            sub_id = self.subscribed.pop(key)
            if sub_id:
                try:
                    api("DELETE", "/eventsub/subscriptions", {"id": sub_id})
                except TwitchError as e:
                    report("events", f"couldn't unsubscribe: {e}")

    def read_for(self, seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline and not self._wake.is_set() and not self.stopped:
            # Take one frame at a time, as it arrives. A ping is answered and counts for
            # nothing: Twitch's sign of life is a message (a keepalive at the least)
            # within the keepalive time from its welcome. A read timeout alone never
            # ends the session.
            text = None
            if self.ws.readable(WS_POLL):
                try:
                    text = self.ws.poll()
                except socket.timeout:
                    pass  # TLS had nothing for us yet; nothing is lost
            if text is not None:
                self.last_message = time.monotonic()
                self.handle(json.loads(text))
            elif (not self.ws.readable(0)
                  and time.monotonic() - self.last_message > self.keepalive + KEEPALIVE_GRACE):
                raise ConnectionError(f"no keepalive from Twitch in {self.keepalive + KEEPALIVE_GRACE:g} s")

    def handle(self, message):
        meta = message.get("metadata") or {}
        payload = message.get("payload") or {}
        kind = meta.get("message_type")
        message_id = meta.get("message_id")
        if message_id:
            if message_id in self.seen:
                return
            self.seen = (self.seen + [message_id])[-200:]
        if kind == "notification":
            sub = payload.get("subscription") or {}
            key = sub_key(sub.get("type"), sub.get("version"), sub.get("condition") or {})
            for arg in self.args.get(key, []):
                on_event(sub.get("type"), payload.get("event") or {}, arg)
        elif kind == "session_reconnect":
            url = ((payload.get("session") or {}).get("reconnect_url")) or EVENTSUB_URL
            old, subscribed = self.ws, dict(self.subscribed)
            self.open(url)  # subscriptions carry over; close the old one once welcomed
            self.subscribed = subscribed
            old.close()
        elif kind == "revocation":
            sub = payload.get("subscription") or {}
            key = sub_key(sub.get("type"), sub.get("version"), sub.get("condition") or {})
            self.subscribed.pop(key, None)
            # Tried again only after a long wait (or on a new connection, or when keys change).
            self.failed[key] = (time.monotonic() + EVENTSUB_RETRY_MAX, EVENTSUB_RETRY_MAX)
            report(f"event {sub.get('type')}", f"Twitch stopped sending these ({sub.get('status')})")
        # session_keepalive: only resets the timer


events = EventSub()


def parse_time(text):
    """RFC 3339 from Twitch ('2026-10-11T18:00:00Z', with or without a fraction) -> Unix seconds."""
    m = re.match(r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})", text or "")
    if not m:
        return None
    return datetime.strptime(m.group(1), "%Y-%m-%dT%H:%M:%S").replace(tzinfo=timezone.utc).timestamp()


def name_of(event, prefix="user"):
    if event.get("is_anonymous"):
        return "Anonymous"
    return event.get(f"{prefix}_name") or event.get(f"{prefix}_login") or "Anonymous"


_session = {}  # arg -> {"follows": n, "subs": n} since the plugin started
_ad_until = {}  # broadcaster arg -> Unix time an ad break ends
_hype = {}  # arg -> "L2 45%" while a hype train runs (not saved: it ends anyway)


def on_event(sub_type, event, arg):
    key = arg.lower()
    remembered = {}
    counts = _session.setdefault(key, {"follows": 0, "subs": 0})
    if sub_type == "channel.follow":
        counts["follows"] += 1
        remembered["last_follower"] = name_of(event)
    elif sub_type in ("channel.subscribe", "channel.subscription.message"):
        counts["subs"] += 1
        remembered["last_subscriber"] = name_of(event)
    elif sub_type == "channel.subscription.gift":
        remembered["last_gift"] = f"{name_of(event)}: {event.get('total', 1)}"
    elif sub_type == "channel.cheer":
        remembered["last_cheer"] = f"{name_of(event)}: {event.get('bits', 0)}"
    elif sub_type == "channel.raid":
        remembered["last_raid"] = f"{name_of(event, 'from_broadcaster_user')} ({event.get('viewers', 0)})"
    elif sub_type == "channel.channel_points_custom_reward_redemption.add":
        remembered["last_redemption"] = f"{name_of(event)}: {(event.get('reward') or {}).get('title') or '?'}"
    elif sub_type in ("channel.hype_train.begin", "channel.hype_train.progress"):
        _hype[key] = hype_text(event)
    elif sub_type == "channel.hype_train.end":
        _hype.pop(key, None)
    elif sub_type == "channel.ad_break.begin":
        start = parse_time(event.get("started_at")) or time.time()
        _ad_until[key] = start + int(event.get("duration_seconds") or 0)
    elif sub_type in ("stream.online", "stream.offline"):
        poll_soon()  # live status, viewers and uptime right away
    if remembered:
        last = dict(_state.get("last") or {})
        last[key] = dict(last.get(key) or {}, **remembered)
        update_state(last=last)
    publish_events()


def hype_text(event):
    level = event.get("level") or 1
    goal = event.get("goal") or 0
    progress = event.get("progress") or 0
    percent = int(progress * 100 // goal) if goal else 0
    return f"L{level} {percent}%"


def publish_events():
    for value_id in EVENT_VALUES:
        for arg in plugin.watched(value_id):
            key = arg.lower()
            if value_id == "session_follows":
                value = str((_session.get(key) or {}).get("follows", 0))
            elif value_id == "session_subs":
                value = str((_session.get(key) or {}).get("subs", 0))
            elif value_id == "hype_train":
                value = _hype.get(key) or NOTHING
            else:
                value = ((_state.get("last") or {}).get(key) or {}).get(value_id) or NOTHING
            put(value_id, arg, value)


# --- live values -----------------------------------------------------------------------------

_reported = {}
_published = {}
_publish_lock = threading.Lock()


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


def put(value_id, arg, value):
    """Publish a value with an argument if it changed."""
    full = f"{value_id}:{arg}"
    with _publish_lock:
        if _published.get(full) == value:
            return
        _published[full] = value
    plugin.set_value(value_id, value, arg=arg)


def clock(seconds):
    seconds = max(0, int(seconds or 0))
    hours, rest = divmod(seconds, 3600)
    return f"{hours}:{rest // 60:02d}:{rest % 60:02d}" if hours else f"{rest // 60}:{rest % 60:02d}"


def uptime(started_at, now=None):
    started = parse_time(started_at)
    if started is None:
        return NOTHING
    seconds = max(0, int((now or time.time()) - started))
    hours, minutes = seconds // 3600, seconds % 3600 // 60
    return f"{hours}h {minutes:02d}m" if hours else f"{minutes}m"


def split_mode_arg(arg):
    """'emote' or 'emote@somechannel' -> (field, channel or '')."""
    mode, _, channel = arg.partition("@")
    return chat_field(mode), channel.strip()


ALL_VALUES = ("live", "viewers", "uptime", "title", "category", "tags", "followers", "subscribers", "chatters",
              "chat_mode", "next_ad", "ad_snoozes", "preroll_free", "ad_live", "reward_enabled", "reward_paused",
              "reward_pending", "poll", "poll_leader", "poll_time", "prediction", "prediction_leader",
              "prediction_time") + EVENT_VALUES
LIVE_VALUES = ("live", "viewers", "uptime")
AD_VALUES = ("next_ad", "ad_snoozes", "preroll_free", "ad_live")
REWARD_VALUES = ("reward_enabled", "reward_paused", "reward_pending")
POLL_VALUES = ("poll", "poll_leader", "poll_time")
PREDICTION_VALUES = ("prediction", "prediction_leader", "prediction_time")


def publish_unknown(value_ids=ALL_VALUES):
    for value_id in value_ids:
        for arg in plugin.watched(value_id):
            put(value_id, arg, NOTHING)


def watched_any(value_ids):
    return any(plugin.watched(v) for v in value_ids)


def poll_live():
    channels = sorted({arg for v in LIVE_VALUES for arg in plugin.watched(v)})
    if not channels:
        return
    try:
        ids = user_ids(channels)
        streams = {}
        wanted_ids = sorted(set(ids.values()))
        for i in range(0, len(wanted_ids), 100):
            answer = api("GET", "/streams", [("user_id", u) for u in wanted_ids[i:i + 100]] + [("first", "100")]) or {}
            for s in answer.get("data") or []:
                streams[s.get("user_id")] = s
    except TwitchError as e:
        report("live status", str(e))
        return publish_unknown(LIVE_VALUES)
    report("live status", None)
    for arg in channels:
        if arg not in ids:
            if not login_problem(arg) and not refused(arg):  # (those are logged already)
                report(f"channel {arg}", "Twitch has no channel with that name")
            values = dict.fromkeys(LIVE_VALUES, NOTHING)
        else:
            stream = streams.get(ids[arg])
            if stream and stream.get("type", "live") == "live":
                values = {"live": "on", "viewers": str(stream.get("viewer_count", 0)),
                          "uptime": uptime(stream.get("started_at"))}
            else:
                values = {"live": "off", "viewers": "0", "uptime": "offline"}
        for value_id in LIVE_VALUES:
            if arg in plugin.watched(value_id):
                put(value_id, arg, values[value_id])


def poll_info():
    info_channels = sorted({arg for v in ("title", "category", "tags") for arg in plugin.watched(v)})
    follow_channels = plugin.watched("followers")
    mode_args = plugin.watched("chat_mode")
    fallback_follower = [] if events.connected and has("moderator:read:followers") else plugin.watched("last_follower")
    if not (info_channels or follow_channels or mode_args or fallback_follower):
        return
    parsed_modes = {}
    for arg in mode_args:
        try:
            parsed_modes[arg] = split_mode_arg(arg)
        except TwitchError as e:
            report(f"chat_mode {arg}", str(e))
            put("chat_mode", arg, NOTHING)
    mode_channels = {channel or "me" for _, channel in parsed_modes.values()}
    try:
        ids = user_ids(sorted(set(info_channels) | set(follow_channels) | mode_channels | set(fallback_follower)))
    except TwitchError as e:
        report("channel info", str(e))
        return publish_unknown(("title", "category", "tags", "followers", "chat_mode"))
    channels = {}
    try:
        wanted_ids = sorted({ids[c] for c in info_channels if c in ids})
        for i in range(0, len(wanted_ids), 100):
            answer = api("GET", "/channels", [("broadcaster_id", b) for b in wanted_ids[i:i + 100]]) or {}
            for c in answer.get("data") or []:
                channels[c.get("broadcaster_id")] = c
        report("channel info", None)
    except TwitchError as e:
        report("channel info", str(e))
    for arg in info_channels:
        c = channels.get(ids.get(arg))
        if arg in plugin.watched("title"):
            put("title", arg, (c.get("title") or NOTHING) if c else NOTHING)
        if arg in plugin.watched("category"):
            put("category", arg, (c.get("game_name") or NOTHING) if c else NOTHING)
        if arg in plugin.watched("tags"):
            put("tags", arg, (", ".join(c.get("tags") or []) or NOTHING) if c else NOTHING)
    for arg in sorted(set(follow_channels) | set(fallback_follower)):
        total, latest = None, None
        if arg in ids:
            try:
                answer = api("GET", "/channels/followers", {"broadcaster_id": ids[arg], "first": 1}) or {}
                total = answer.get("total")
                data = answer.get("data") or []
                latest = data[0].get("user_name") if data else None
                report(f"followers of {arg}", None)
            except TwitchError as e:
                report(f"followers of {arg}", str(e))
        if arg in follow_channels:
            put("followers", arg, NOTHING if total is None else str(total))
        if arg in fallback_follower and latest:
            last = dict(_state.get("last") or {})
            if (last.get(arg.lower()) or {}).get("last_follower") != latest:
                last[arg.lower()] = dict(last.get(arg.lower()) or {}, last_follower=latest)
                update_state(last=last)
            put("last_follower", arg, latest)
    settings_by_channel = {}
    for channel in mode_channels:
        if channel not in ids:
            continue
        params = {"broadcaster_id": ids[channel]}
        if any(f == "non_moderator_chat_delay" and (c or "me") == channel for f, c in parsed_modes.values()):
            params["moderator_id"] = me_id()
        try:
            settings_by_channel[channel] = first(api("GET", "/chat/settings", params))
            report(f"chat settings of {channel}", None)
        except TwitchError as e:
            report(f"chat settings of {channel}", str(e))
    for arg, (field, channel) in parsed_modes.items():
        chat_settings = settings_by_channel.get(channel or "me")
        if not chat_settings or field not in chat_settings:
            value = NOTHING
        else:
            value = "on" if chat_settings.get(field) else "off"
        put("chat_mode", arg, value)


def poll_counts():
    """Subscribers (owner only) and chatters (moderators), every 60 s."""
    for value_id, path, scope in (("subscribers", "/subscriptions", "channel:read:subscriptions"),
                                  ("chatters", "/chat/chatters", "moderator:read:chatters")):
        for arg in plugin.watched(value_id):
            if not has(scope):
                report(value_id, missing_scope_message(scope))
                put(value_id, arg, NOTHING)
                continue
            try:
                params = {"broadcaster_id": channel_id(arg), "first": 1}
                if value_id == "chatters":
                    params["moderator_id"] = me_id()
                total = (api("GET", path, params) or {}).get("total")
                put(value_id, arg, NOTHING if total is None else str(total))
                report(f"{value_id} of {arg}", None)
            except TwitchError as e:
                report(f"{value_id} of {arg}", str(e))
                put(value_id, arg, NOTHING)


_ads = {"cache": None}  # {arg: (fetched_at, schedule)}


def epoch(value):
    """Twitch's ad schedule times: Unix seconds (number or text) or RFC 3339."""
    if value in (None, "", 0, "0"):
        return None
    try:
        return float(value)
    except (TypeError, ValueError):
        return parse_time(str(value))


def poll_ads():
    args = sorted({arg for v in AD_VALUES for arg in plugin.watched(v)})
    if not args:
        return
    cache = {}
    for arg in args:
        if not has("channel:read:ads"):
            report("ads", missing_scope_message("channel:read:ads"))
            continue
        try:
            cache[arg] = (time.time(), first(api("GET", "/channels/ads", {"broadcaster_id": channel_id(arg)})))
            report(f"ads of {arg}", None)
        except TwitchError as e:
            report(f"ads of {arg}", str(e))
    _ads["cache"] = cache
    publish_ads()


def publish_ads(now=None):
    """Recompute the countdowns from the last schedule (every tick, no request)."""
    now = now or time.time()
    cache = _ads["cache"] or {}
    for arg in sorted({arg for v in AD_VALUES for arg in plugin.watched(v)}):
        entry = cache.get(arg)
        if not entry:
            for value_id in AD_VALUES:
                if arg in plugin.watched(value_id):
                    put(value_id, arg, NOTHING)
            continue
        fetched, s = entry
        next_at = epoch(s.get("next_ad_at"))
        last_at = epoch(s.get("last_ad_at"))
        ad_end = max(_ad_until.get(arg.lower(), 0), (last_at or 0) + int(s.get("duration") or 0))
        preroll = s.get("preroll_free_time")
        values = {
            "next_ad": clock(next_at - now) if next_at and next_at > now else NOTHING,
            "ad_snoozes": NOTHING if s.get("snooze_count") is None else str(s.get("snooze_count")),
            "preroll_free": NOTHING if preroll is None else clock(max(0, int(preroll) - (now - fetched))),
            "ad_live": "on" if ad_end > now else "off",
        }
        for value_id in AD_VALUES:
            if arg in plugin.watched(value_id):
                put(value_id, arg, values[value_id])


_rewards = {"at": 0.0}


def poll_rewards():
    titles = sorted({arg for v in REWARD_VALUES for arg in plugin.watched(v)})
    if not titles:
        return
    if not has("channel:read:redemptions", "channel:manage:redemptions"):
        report("rewards", missing_scope_message("channel:manage:redemptions"))
        return publish_unknown(REWARD_VALUES)
    try:
        broadcaster = me_id()
        rewards = (api("GET", "/channel_points/custom_rewards", {"broadcaster_id": broadcaster}) or {}).get("data") or []
        report("rewards", None)
    except TwitchError as e:
        report("rewards", str(e))
        return publish_unknown(REWARD_VALUES)
    by_title = {(r.get("title") or "").strip().lower(): r for r in rewards}
    for title in titles:
        r = by_title.get(title.strip().lower())
        if arg_in("reward_enabled", title):
            put("reward_enabled", title, NOTHING if r is None else ("on" if r.get("is_enabled") else "off"))
        if arg_in("reward_paused", title):
            put("reward_paused", title, NOTHING if r is None else ("on" if r.get("is_paused") else "off"))
        if arg_in("reward_pending", title):
            value = NOTHING
            if r is not None:
                try:
                    answer = api("GET", "/channel_points/custom_rewards/redemptions", {
                        "broadcaster_id": broadcaster, "reward_id": r["id"], "status": "UNFULFILLED", "first": 50}) or {}
                    n = len(answer.get("data") or [])
                    value = f"{n}+" if (answer.get("pagination") or {}).get("cursor") else str(n)
                    report(f"redemptions of {title}", None)
                except TwitchError as e:
                    report(f"redemptions of {title}", POINTS_ONLY_OWN if "403" in str(e) else str(e))
            put("reward_pending", title, value)


def arg_in(value_id, arg):
    return arg in plugin.watched(value_id)


_votes = {"poll": {}, "prediction": {}}  # arg -> newest poll/prediction


def poll_votes():
    for kind, values, scopes in (("polls", POLL_VALUES, ("channel:read:polls", "channel:manage:polls")),
                                 ("predictions", PREDICTION_VALUES,
                                  ("channel:read:predictions", "channel:manage:predictions"))):
        args = sorted({arg for v in values for arg in plugin.watched(v)})
        if not args:
            continue
        name = kind[:-1]
        if not has(*scopes):
            report(kind, missing_scope_message(scopes[-1]))
            publish_unknown(values)
            continue
        for arg in args:
            try:
                _votes[name][arg] = (time.time(), first(api("GET", f"/{kind}", {"broadcaster_id": channel_id(arg), "first": 1})))
                report(f"{kind} of {arg}", None)
            except TwitchError as e:
                report(f"{kind} of {arg}", str(e))
                _votes[name].pop(arg, None)
    publish_votes()


def leader(options, count):
    total = sum(int(o.get(count) or 0) for o in options)
    if not options:
        return NOTHING
    best = max(options, key=lambda o: int(o.get(count) or 0))
    if not total:
        return f"{best.get('title')} 0%"
    return f"{best.get('title')} {round(int(best.get(count) or 0) * 100 / total)}%"


def publish_votes(now=None):
    now = now or time.time()
    for arg in sorted({arg for v in POLL_VALUES for arg in plugin.watched(v)}):
        fetched, p = _votes["poll"].get(arg, (now, {}))
        status = p.get("status")
        ended = epoch(p.get("ended_at"))
        shown = status == "ACTIVE" or (status in ("COMPLETED", "TERMINATED") and ended and now - ended < RECENT_SECONDS)
        if not shown:
            values = dict.fromkeys(POLL_VALUES, NOTHING)
        else:
            started = epoch(p.get("started_at")) or fetched
            left = started + int(p.get("duration") or 0) - now
            values = {
                "poll": p.get("title") or NOTHING,
                "poll_leader": leader(p.get("choices") or [], "votes"),
                "poll_time": clock(left) if status == "ACTIVE" and left > 0 else "ended",
            }
        for value_id in POLL_VALUES:
            if arg_in(value_id, arg):
                put(value_id, arg, values[value_id])
    for arg in sorted({arg for v in PREDICTION_VALUES for arg in plugin.watched(v)}):
        fetched, p = _votes["prediction"].get(arg, (now, {}))
        status = p.get("status")
        ended = epoch(p.get("ended_at"))
        shown = status in ("ACTIVE", "LOCKED") or (status in ("RESOLVED", "CANCELED") and ended
                                                   and now - ended < RECENT_SECONDS)
        if not shown:
            values = dict.fromkeys(PREDICTION_VALUES, NOTHING)
        else:
            created = epoch(p.get("created_at")) or fetched
            left = created + int(p.get("prediction_window") or 0) - now
            if status == "ACTIVE":
                time_value = clock(left) if left > 0 else "locked"
            elif status == "LOCKED":
                time_value = "locked"
            elif status == "RESOLVED":
                winner = next((o for o in p.get("outcomes") or [] if o.get("id") == p.get("winning_outcome_id")), {})
                time_value = f"{winner.get('title') or '?'} won"
            else:
                time_value = "cancelled"
            values = {
                "prediction": p.get("title") or NOTHING,
                "prediction_leader": leader(p.get("outcomes") or [], "channel_points"),
                "prediction_time": time_value,
            }
        for value_id in PREDICTION_VALUES:
            if arg_in(value_id, arg):
                put(value_id, arg, values[value_id])


def publish_fixed():
    """last_clip and last_marker: from this plugin's own actions, no argument."""
    clip_at = _state.get("last_clip_at")
    plugin.set_value("last_clip", local_time(clip_at) if clip_at else NOTHING)
    plugin.set_value("last_marker", _state.get("last_marker") or NOTHING)


# job name -> (seconds, value ids, function)
JOBS = {
    "live": (10, LIVE_VALUES, poll_live),
    "info": (30, ("title", "category", "tags", "followers", "chat_mode", "last_follower"), poll_info),
    "counts": (60, ("subscribers", "chatters"), poll_counts),
    "ads": (30, AD_VALUES, poll_ads),
    "rewards": (30, REWARD_VALUES, poll_rewards),
    "votes": (10, POLL_VALUES + PREDICTION_VALUES, poll_votes),
}
_schedule = dict.fromkeys(JOBS, 0.0)


def reschedule():
    for name in JOBS:
        _schedule[name] = 0.0


def signed_in():
    try:
        return current_tokens() is not None and bool(client_id())
    except TwitchError:
        return False


_poll_lock = threading.Lock()


def safe_poll(force=False):
    if not _poll_lock.acquire(blocking=False):
        return
    try:
        # At start and hourly while signed in, even with only action keys on the deck.
        if signed_in() and not check_sign_in():
            publish_unknown()
            return
        if not watched_any(ALL_VALUES):
            return
        if not signed_in():
            publish_unknown()
            return
        if wait_left() > 0:
            return
        now = time.monotonic()
        for name, (seconds, value_ids, fn) in JOBS.items():
            if not watched_any(value_ids):
                continue
            if force or now >= _schedule[name]:
                _schedule[name] = now + seconds
                try:
                    fn()
                except TwitchError as e:
                    report(name, str(e))
        publish_ads()
        publish_votes()
        publish_events()
    except Exception as e:  # keep the timer alive whatever happens
        report("poll", f"failed: {e}")
    finally:
        _poll_lock.release()


@plugin.every(1)
def tick():
    safe_poll()


@plugin.on_start
def started():
    load_state()
    publish_fixed()


@plugin.on_watch
def watch_changed(values):
    watching = set(values)
    with _publish_lock:
        for full in [k for k in _published if k not in watching]:
            del _published[full]
    reschedule()
    events.wake()


@plugin.on_settings
def settings_changed(settings):
    _users.clear()
    reschedule()
    if plugin.watching:
        events.wake()


if __name__ == "__main__":
    plugin.run()
