#!/usr/bin/env python3
"""HTTP trace and recorder for the Twitch plugin.

    py -3 plugins/tools/twitch_trace.py run --trace TRACE.jsonl [--no-browser] [overrides]
    py -3 plugins/tools/twitch_trace.py summary TRACE.jsonl [--json]
    py -3 plugins/tools/twitch_trace.py fixtures TRACE.jsonl FIXTURES.json

`run` imports plugins/twitch/plugin.py (it doesn't change it) and runs it over stdio as the
host would, with urllib patched so that every request and answer goes to TRACE.jsonl, one
JSON object per line. twitch_live_test.py starts the plugin this way. It refuses to run
without PERRITODECK_PLUGIN_DATA, so a traced plugin never uses the real data folder.

What a trace keeps out: access and refresh tokens, device codes and user codes, client
secrets, and the token in every Authorization header ("Bearer [redacted]"), wherever
they turn up, in bodies, headers, query strings or error text. The client ID stays (Twitch
calls it public), and so does everything else: status codes, rate-limit headers, Twitch's
error bodies. A trace can still name the accounts involved; `fixtures` replaces those.

Records (`kind`):
- `start`, `stop`: the plugin process starting and ending (plugin version, overrides).
- `http`: `method`, `url`, `path`, `query` (lists), `request_headers`, `request_body`
  (JSON or form fields), then `status`, `response_headers`, `response_body` and `ms`,
  or `error` when Twitch couldn't be reached.
- `ws`: EventSub over WebSocket: `event` `connect` (`url`, `session`, `keepalive`),
  `connect_failed`, `close` (the plugin dropping its session) and `message` (`message_type`,
  `subscription_type`, the notification's `event_payload`, a revocation's `status`).
- `open`: an address the plugin asked to open in a browser, with --no-browser.
Every record has `seq` (1, 2, ...) and `t` (Unix seconds).

`summary` prints call counts, the busiest minute of Helix calls, the validation rhythm,
refreshes, revokes, EventSub sessions and Twitch's error answers.

`fixtures` turns a trace into recorded contract fixtures (format below) with the accounts
replaced by stand-ins (user1, user2, ... and made-up IDs) the same way throughout, so the
file can go in the repository. One fixture per distinct request and answer: method, path,
status, the query's parameter names and Twitch's error message.

    {
      "format": "perritodeck-twitch-fixtures/1",
      "recorded": "2026-10-13",            # the trace's first day (UTC)
      "source": "trace.jsonl",
      "fixtures": [
        {"name": "GET /helix/users 200",
         "request": {"method": "GET", "path": "/helix/users", "query": {"login": ["user2"]},
                     "body": null},
         "response": {"status": 200, "headers": {"Ratelimit-Remaining": "799", ...},
                      "body": {"data": [...]}}}
      ],
      "events": [
        {"subscription_type": "channel.follow", "subscription_version": "2",
         "condition": {...}, "event": {...}}
      ]
    }

A fake server replays one with find_fixture(fixtures, method, path, query): the first
fixture with that method and path whose recorded query is contained in the request's
(test_plugin.py's fake doesn't use them yet). Standard library only.
"""

import argparse
import collections
import io
import itertools
import json
import os
import re
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from twitch_common import TWITCH_DIR, Redactor, manifest  # noqa: E402

FORMAT = "perritodeck-twitch-fixtures/1"
DATA_ENV = "PERRITODECK_PLUGIN_DATA"


# --- recording ------------------------------------------------------------------------------


def decode_body(raw, content_type=""):
    """A request or answer body as JSON, form fields or text."""
    if raw is None:
        return None
    if isinstance(raw, bytes):
        text = raw.decode("utf-8", "replace")
    else:
        text = str(raw)
    if not text:
        return None
    if "x-www-form-urlencoded" in (content_type or "").lower():
        return dict(urllib.parse.parse_qsl(text, keep_blank_values=True))
    try:
        return json.loads(text)
    except ValueError:
        return text[:2000]


def header_dict(headers):
    if headers is None:
        return {}
    try:
        items = headers.items()
    except AttributeError:
        items = dict(headers).items()
    return {k: v for k, v in items}


class Answer:
    """What urlopen returns, rebuilt after its body was read for the trace."""

    def __init__(self, status, headers, body, url, reason=""):
        self.status, self.headers, self._body, self.url, self.reason = status, headers, io.BytesIO(body), url, reason
        self.code = status

    def read(self, *args):
        return self._body.read(*args)

    def getcode(self):
        return self.status

    def geturl(self):
        return self.url

    def info(self):
        return self.headers

    def close(self):
        pass

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False


class Tracer:
    def __init__(self, path):
        folder = os.path.dirname(os.path.abspath(path))
        os.makedirs(folder, exist_ok=True)
        self.file = open(path, "a", encoding="utf-8")
        self.lock = threading.Lock()
        self.seq = itertools.count(1)
        self.redactor = Redactor()
        self.real_urlopen = urllib.request.urlopen
        # Continue a trace's numbering when a restarted plugin appends to it.
        try:
            with open(path, encoding="utf-8") as f:
                last = 0
                for line in f:
                    try:
                        last = max(last, int(json.loads(line).get("seq") or 0))
                    except ValueError:
                        pass
            self.seq = itertools.count(last + 1)
        except OSError:
            pass

    def write(self, record):
        with self.lock:
            record = dict({"seq": next(self.seq), "t": round(time.time(), 3)}, **record)
            line = json.dumps(self.redactor.scrub(record), ensure_ascii=False)
            self.file.write(line + "\n")
            self.file.flush()

    # urllib.request.urlopen, traced
    def urlopen(self, req, data=None, timeout=None, **kwargs):
        if isinstance(req, str):
            req = urllib.request.Request(req, data=data)
        elif data is not None:
            req.data = data
        method = req.get_method()
        headers = header_dict(dict(req.header_items()))
        content_type = next((v for k, v in headers.items() if k.lower() == "content-type"), "")
        parts = urllib.parse.urlsplit(req.full_url)
        record = {"kind": "http", "method": method, "url": req.full_url, "path": parts.path,
                  "query": urllib.parse.parse_qs(parts.query, keep_blank_values=True),
                  "request_headers": headers, "request_body": decode_body(req.data, content_type)}
        self.redactor.learn_all(record)
        start = time.monotonic()
        args = {} if timeout is None else {"timeout": timeout}
        try:
            resp = self.real_urlopen(req, **args, **kwargs)
        except urllib.error.HTTPError as e:
            try:
                raw = e.read() or b""
            except Exception:
                raw = b""
            hdrs = e.headers
            record.update(status=e.code, response_headers=header_dict(hdrs),
                          response_body=decode_body(raw, (hdrs or {}).get("Content-Type", "")),
                          ms=round((time.monotonic() - start) * 1000))
            self.write(record)
            raise urllib.error.HTTPError(e.url if hasattr(e, "url") else req.full_url, e.code, e.msg, hdrs,
                                         io.BytesIO(raw)) from None
        except (urllib.error.URLError, OSError) as e:
            record.update(error=str(getattr(e, "reason", e)), ms=round((time.monotonic() - start) * 1000))
            self.write(record)
            raise
        with resp:
            raw, status, hdrs = resp.read(), resp.status, resp.headers
            reason = getattr(resp, "reason", "")
        record.update(status=status, response_headers=header_dict(hdrs),
                      response_body=decode_body(raw, hdrs.get("Content-Type", "")),
                      ms=round((time.monotonic() - start) * 1000))
        self.write(record)
        return Answer(status, hdrs, raw, req.full_url, reason)

    def install(self, twitch):
        """Patch urllib and the plugin's EventSub session to record into this trace."""
        urllib.request.urlopen = self.urlopen
        tracer = self
        events = twitch.EventSub
        real_open, real_drop, real_handle = events.open, events.drop, events.handle

        def traced_open(session, url):
            try:
                result = real_open(session, url)
            except Exception as e:
                tracer.write({"kind": "ws", "event": "connect_failed", "url": url, "error": str(e)})
                raise
            tracer.write({"kind": "ws", "event": "connect", "url": url, "session": session.session,
                          "keepalive": session.keepalive})
            return result

        def traced_drop(session):
            if session.ws is not None:
                tracer.write({"kind": "ws", "event": "close", "session": session.session,
                              "subscribed": len(session.subscribed)})
            return real_drop(session)

        def traced_handle(session, message):
            meta = message.get("metadata") or {}
            payload = message.get("payload") or {}
            sub = payload.get("subscription") or {}
            record = {"kind": "ws", "event": "message", "message_type": meta.get("message_type"),
                      "session": session.session}
            if meta.get("subscription_type"):
                record.update(subscription_type=meta.get("subscription_type"),
                              subscription_version=meta.get("subscription_version"))
            if record["message_type"] == "notification":
                record.update(condition=sub.get("condition"), event_payload=payload.get("event"))
            elif record["message_type"] == "revocation":
                record.update(subscription_type=sub.get("type"), status=sub.get("status"))
            elif record["message_type"] == "session_reconnect":
                record["reconnect_url"] = (payload.get("session") or {}).get("reconnect_url")
            tracer.write(record)
            return real_handle(session, message)

        events.open, events.drop, events.handle = traced_open, traced_drop, traced_handle


def no_browser(tracer):
    def fake_open(target, cwd=None):
        if tracer is not None:
            tracer.write({"kind": "open", "target": str(target)})
        sys.stderr.write("[twitch_trace] not opening a browser (--no-browser)\n")
    return fake_open


def parse_setting(text):
    name, sep, value = text.partition("=")
    if not sep or not re.match(r"^[A-Z][A-Z0-9_]*$", name):
        raise argparse.ArgumentTypeError(f"--set takes NAME=VALUE with an upper-case plugin constant, not {text!r}")
    try:
        return name, json.loads(value)
    except ValueError:
        return name, value


def run_plugin(args):
    if not os.environ.get(DATA_ENV):
        sys.exit(f"{DATA_ENV} isn't set: point it at a test folder (twitch_live_test.py does), "
                 "so a traced plugin never uses PerritoDeck's real data folder")
    plugin_py = os.path.join(TWITCH_DIR, "plugin.py")
    sys.argv = [plugin_py]  # as if run directly, for anything that looks at it
    if TWITCH_DIR not in sys.path:
        sys.path.insert(0, TWITCH_DIR)
    import plugin as twitch  # noqa: E402  (registers the actions; doesn't run)

    tracer = Tracer(args.trace) if args.trace else None
    overrides = {}
    for name, attr in (("id_url", "ID_URL"), ("helix_url", "HELIX_URL"), ("eventsub_url", "EVENTSUB_URL")):
        if getattr(args, name):
            overrides[attr] = getattr(args, name)
    for name, value in args.set or []:
        if not hasattr(twitch, name):
            sys.exit(f"the plugin has no constant {name}")
        overrides[name] = value
    for name, value in overrides.items():
        setattr(twitch, name, value)
    if args.fast_polls:  # dry runs against the fakes only: every poll five times as often
        twitch.JOBS.update({k: (max(1, round(s / 5)), v, f) for k, (s, v, f) in twitch.JOBS.items()})
        overrides["JOBS"] = "fast"
    if tracer:
        tracer.install(twitch)
        tracer.write({"kind": "start", "plugin_version": manifest().get("version"), "pid": os.getpid(),
                      "overrides": overrides, "no_browser": bool(args.no_browser)})
    if args.no_browser:
        twitch.open_for_user = no_browser(tracer)
    try:
        twitch.plugin.run()
    finally:
        if tracer:
            tracer.write({"kind": "stop", "pid": os.getpid()})


# --- reading a trace ---------------------------------------------------------------------------


def load(path):
    records = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                records.append(json.loads(line))
            except ValueError:
                pass
    return records


class TraceReader:
    """Reads a trace as it grows (the harness checks it while the plugin runs)."""

    def __init__(self, path):
        self.path = path
        self.offset = 0
        self.partial = ""
        self.all = []

    def refresh(self):
        try:
            with open(self.path, encoding="utf-8") as f:
                f.seek(self.offset)
                chunk = f.read()
                self.offset = f.tell()
        except OSError:
            return self.all
        text = self.partial + chunk
        lines = text.split("\n")
        self.partial = lines.pop()
        for line in lines:
            if line.strip():
                try:
                    self.all.append(json.loads(line))
                except ValueError:
                    pass
        return self.all

    def last_seq(self):
        self.refresh()
        return self.all[-1]["seq"] if self.all else 0

    def since(self, seq=0):
        return [r for r in self.refresh() if r.get("seq", 0) > seq]


def http_records(records, method=None, path=None, status=None):
    out = []
    for r in records:
        if r.get("kind") != "http":
            continue
        if method and r.get("method") != method:
            continue
        if path and r.get("path") != path:
            continue
        if status is not None and r.get("status") != status:
            continue
        out.append(r)
    return out


def grant(record):
    body = record.get("request_body")
    return body.get("grant_type") if isinstance(body, dict) else None


def busiest_minute(times):
    times = sorted(times)
    best, j = 0, 0
    for i, t in enumerate(times):
        while times[j] < t - 60:
            j += 1
        best = max(best, i - j + 1)
    return best


def analyse(records):
    http = [r for r in records if r.get("kind") == "http"]
    helix = [r for r in http if "/helix/" in (r.get("path") or "")]
    counts = collections.Counter(f"{r['method']} {r.get('path')} {r.get('status', 'unreachable')}" for r in http)
    validations = [r["t"] for r in http if r.get("path") == "/oauth2/validate"]
    refreshes = [r for r in http if r.get("path") == "/oauth2/token" and grant(r) == "refresh_token"]
    errors = {}
    for r in http:
        status = r.get("status")
        if status is None or status >= 400:
            body = r.get("response_body")
            message = body.get("message") or body.get("error") if isinstance(body, dict) else body
            key = (r["method"], r.get("path"), status, str(message or r.get("error") or ""))
            errors[key] = errors.get(key, 0) + 1
    ws = [r for r in records if r.get("kind") == "ws"]
    first_t = records[0]["t"] if records else None
    last_t = records[-1]["t"] if records else None
    return {
        "records": len(records),
        "seconds": round(last_t - first_t, 1) if records else 0,
        "requests": len(http),
        "helix_requests": len(helix),
        "helix_busiest_minute": busiest_minute([r["t"] for r in helix]),
        "counts": dict(sorted(counts.items())),
        "validations": len(validations),
        "validation_gaps_minutes": [round((b - a) / 60, 1) for a, b in zip(validations, validations[1:])],
        "refreshes": len(refreshes),
        "revokes": len([r for r in http if r.get("path") == "/oauth2/revoke"]),
        "plugin_starts": len([r for r in records if r.get("kind") == "start"]),
        "eventsub_connects": len([r for r in ws if r.get("event") == "connect"]),
        "eventsub_closes": len([r for r in ws if r.get("event") == "close"]),
        "eventsub_reconnect_messages": len([r for r in ws if r.get("message_type") == "session_reconnect"]),
        "eventsub_notifications": dict(collections.Counter(
            r.get("subscription_type") for r in ws if r.get("message_type") == "notification")),
        "errors": [{"method": m, "path": p, "status": s, "message": msg, "times": n}
                   for (m, p, s, msg), n in sorted(errors.items(), key=lambda kv: str(kv[0]))],
    }


def print_summary(summary, out=sys.stdout):
    w = out.write
    w(f"{summary['records']} records over {summary['seconds']} s; {summary['plugin_starts']} plugin start(s)\n")
    w(f"{summary['requests']} requests, {summary['helix_requests']} to Helix; "
      f"busiest minute: {summary['helix_busiest_minute']} Helix calls (Twitch allows 800)\n")
    w(f"validations: {summary['validations']}; minutes between them: {summary['validation_gaps_minutes'] or '-'}\n")
    w(f"refreshes: {summary['refreshes']}; revokes: {summary['revokes']}\n")
    w(f"EventSub: {summary['eventsub_connects']} connection(s), {summary['eventsub_closes']} closed by the plugin, "
      f"{summary['eventsub_reconnect_messages']} session_reconnect message(s); notifications: "
      f"{summary['eventsub_notifications'] or '-'}\n")
    w("calls:\n")
    for key, n in summary["counts"].items():
        w(f"  {n:5d}  {key}\n")
    if summary["errors"]:
        w("Twitch's error answers:\n")
        for e in summary["errors"]:
            w(f"  {e['times']:5d}  {e['method']} {e['path']} {e['status'] or 'unreachable'}: {e['message']}\n")


# --- contract fixtures ---------------------------------------------------------------------------

NAME_KEYS = re.compile(r"^(.*_)?(login|user_name|display_name|broadcaster_name|moderator_name|requester_name|name)$")
NOT_NAMES = {"name", "game_name", "category_name", "tag_name"}  # games and tags, not people
ID_KEYS = re.compile(r"^(.*_)?(user_id|broadcaster_id|moderator_id|sender_id|gifter_id)$")
URL_HOST = re.compile(r"(?:https?|wss?)://([^/?#\s]+)")
# Hosts left as they are in addresses, so twitch.tv never becomes user1.tv. Any other host
# is anonymised like the rest of the text: a viewer's own site can carry their login.
KEEP_HOSTS = ("twitch.tv", "jtvnw.net", "twitchcdn.net", "twitchsvc.net", "ttvnw.net", "example.invalid",
              "localhost", "127.0.0.1", "[::1]")
# What viewers write (a cheer's or a resub's message, a redemption's input) can name anyone,
# people no name field mentions included, so it's replaced whole.
VIEWER_TEXT = "viewer text"
# Twitch's own channels, which the read-only cases use (--public-channel, B8): not people,
# and "twitch" taken for a name would turn every "Twitch" in Twitch's messages into User1.
TWITCH_CHANNELS = {"twitch", "twitchdev", "twitchgaming", "twitchmusic", "twitchrivals", "twitchpresents",
                   "twitchsupport"}


def user_id_key(obj, key, users=False):
    """Whether `key` in the dict `obj` holds a person's user ID: the usual names, "id" in
    a /users answer, and any <x>_id next to <x>_login or <x>_name (creator_id beside
    creator_name, say), unless that name is a game's, a category's or a tag's."""
    if ID_KEYS.match(key) or (users and key == "id"):
        return True
    if not key.endswith("_id"):
        return False
    stem = key[:-3]
    return f"{stem}_login" in obj or (f"{stem}_name" in obj and f"{stem}_name" not in NOT_NAMES)


def kept_host(hostport):
    """Whether an address's host is one of KEEP_HOSTS (or under one). One with a user in
    it (user@host) never is."""
    if "@" in hostport:
        return False
    host = (hostport[:hostport.find("]") + 1] if hostport.startswith("[") else hostport.split(":")[0]).lower()
    return any(host == keep or host.endswith("." + keep) for keep in KEEP_HOSTS)


def without_viewer_text(event):
    """An EventSub event with its message (a cheer's text, or a resub's text and emotes)
    replaced by VIEWER_TEXT, keeping its shape."""
    if not isinstance(event, dict) or "message" not in event:
        return event
    message = event["message"]
    if isinstance(message, str):
        message = VIEWER_TEXT
    elif isinstance(message, dict):
        message = {k: VIEWER_TEXT if k == "text" else [] if k in ("emotes", "fragments") else v
                   for k, v in message.items()}
    return dict(event, message=message)
# Wiped whatever they hold: a stream's or clip's thumbnail_url has the login in it
# (.../live_user_<login>-{width}x{height}.jpg), and so can a VOD's.
WIPE_KEYS = {"email": "", "description": "", "profile_image_url": "https://example.invalid/image.png",
             "offline_image_url": "https://example.invalid/image.png",
             "thumbnail_url": "https://example.invalid/thumbnail-{width}x{height}.jpg", "user_input": VIEWER_TEXT}


class Anonymiser:
    """Replaces account names and IDs with stand-ins, the same one for the same account
    everywhere in a file: logins user1, user2, ...; display names User1, ...; user IDs
    1000001, ... Game names, titles, Twitch's wording, Twitch's own channels and hosts
    (KEEP_HOSTS) stay. What viewers write is replaced whole (VIEWER_TEXT)."""

    def __init__(self):
        self.names = {}
        self.ids = {}

    def name(self, value):
        key = str(value).strip().lower()
        if not key or key in TWITCH_CHANNELS:
            return value
        if key not in self.names:
            self.names[key] = f"user{len(self.names) + 1}"
        stand_in = self.names[key]
        return stand_in.capitalize() if str(value) != key else stand_in

    def user_id(self, value):
        if not str(value).strip():
            return value
        if str(value) not in self.ids:
            self.ids[str(value)] = str(1000001 + len(self.ids))
        return self.ids[str(value)]

    def collect(self, obj, path=""):
        """First pass: learn every name and ID, so text that mentions them can be fixed."""
        if isinstance(obj, dict):
            users = path.endswith("/users")
            for k, v in obj.items():
                learn = (self.name if k not in NOT_NAMES and NAME_KEYS.match(k)
                         else self.user_id if user_id_key(obj, k, users) else None)
                # A query keeps each parameter's values in a list (?login=a&login=b).
                for item in v if isinstance(v, list) else [v]:
                    if learn and isinstance(item, str):
                        learn(item)
                    else:
                        self.collect(item, path)
        elif isinstance(obj, list):
            for v in obj:
                self.collect(v, path)

    def apply(self, obj, path="", key=None, is_id=False):
        users = path.endswith("/users")
        if isinstance(obj, dict):
            return {k: self.apply(v, path, k, user_id_key(obj, k, users)) for k, v in obj.items()}
        if isinstance(obj, list):
            return [self.apply(v, path, key, is_id) for v in obj]
        if not isinstance(obj, str) or key is None:
            return self.text(obj) if isinstance(obj, str) else obj
        if key in WIPE_KEYS:
            return WIPE_KEYS[key]
        if key == "session_id":
            return "session-1"
        if key not in NOT_NAMES and NAME_KEYS.match(key):
            return self.name(obj)
        if is_id:
            return self.user_id(obj)
        return self.text(obj)

    def text(self, s):
        parts, last = [], 0
        for url in URL_HOST.finditer(s):
            if kept_host(url.group(1)):
                parts += [self.replace(s[last:url.start()]), url.group(0)]
                last = url.end()
        return "".join(parts + [self.replace(s[last:])])

    def replace(self, s):
        # A name ends at anything but a letter or digit: "_" counts as a boundary, since
        # Twitch builds names into addresses that way (live_user_<login>-, <hash>_<login>_).
        for real, stand_in in sorted(self.names.items(), key=lambda kv: len(kv[0]), reverse=True):
            s = re.sub(r"(?<![A-Za-z0-9])" + re.escape(real) + r"(?![A-Za-z0-9])", stand_in, s, flags=re.I)
        for real, stand_in in self.ids.items():
            s = re.sub(r"(?<![0-9])" + re.escape(real) + r"(?![0-9])", stand_in, s)
        return s


KEEP_HEADERS = ("ratelimit-limit", "ratelimit-remaining", "ratelimit-reset", "content-type", "retry-after")


def to_fixtures(records, source=""):
    anon = Anonymiser()
    http = [r for r in records if r.get("kind") == "http" and r.get("status") is not None]
    notifications = [r for r in records if r.get("kind") == "ws" and r.get("message_type") == "notification"]
    for r in http:
        anon.collect(r.get("query"), r.get("path", ""))
        anon.collect(r.get("request_body"), r.get("path", ""))
        anon.collect(r.get("response_body"), r.get("path", ""))
    for r in notifications:
        anon.collect(r.get("condition"))
        anon.collect(r.get("event_payload"))
    fixtures, seen = [], set()
    for r in http:
        body = r.get("response_body")
        message = (body.get("message") or "") if isinstance(body, dict) else ""
        key = (r["method"], r["path"], r["status"], tuple(sorted((r.get("query") or {}).keys())), message)
        if key in seen:
            continue
        seen.add(key)
        path = r["path"]
        headers = {k: v for k, v in (r.get("response_headers") or {}).items() if k.lower() in KEEP_HEADERS}
        fixtures.append({
            "name": f"{r['method']} {path} {r['status']}" + (f" ({anon.text(message)})" if message else ""),
            "request": {"method": r["method"], "path": path,
                        "query": anon.apply(r.get("query") or {}, path, "query"),
                        "body": anon.apply(r.get("request_body"), path, "body")},
            "response": {"status": r["status"], "headers": headers, "body": anon.apply(body, path, "body")},
        })
    events, seen_events = [], set()
    for r in notifications:
        if r.get("subscription_type") in seen_events:
            continue
        seen_events.add(r.get("subscription_type"))
        events.append({"subscription_type": r.get("subscription_type"),
                       "subscription_version": r.get("subscription_version"),
                       "condition": anon.apply(r.get("condition"), "", "condition"),
                       "event": anon.apply(without_viewer_text(r.get("event_payload")), "", "event")})
    first = next((r["t"] for r in records if "t" in r), time.time())
    return {"format": FORMAT, "recorded": time.strftime("%Y-%m-%d", time.gmtime(first)),
            "source": os.path.basename(source), "fixtures": fixtures, "events": events}


def find_fixture(fixtures, method, path, query=None):
    """The recorded answer for a request: the first fixture with this method and path
    whose recorded query is contained in `query` ({name: [values]} or {name: value})."""
    if isinstance(fixtures, dict):
        fixtures = fixtures.get("fixtures") or []
    given = {k: (v if isinstance(v, list) else [v]) for k, v in (query or {}).items()}
    for f in fixtures:
        req = f["request"]
        if req["method"] != method or req["path"] != path:
            continue
        if all(set(v) <= set(given.get(k, [])) for k, v in (req.get("query") or {}).items()):
            return f
    return None


def main(argv=None):
    parser = argparse.ArgumentParser(description="Trace the Twitch plugin's HTTP calls; summarise a trace; "
                                     "turn one into contract fixtures.")
    sub = parser.add_subparsers(dest="command", required=True)
    run = sub.add_parser("run", help="run the plugin over stdio with its HTTP calls traced")
    run.add_argument("--trace", help="the JSONL file to append to (no trace without it)")
    run.add_argument("--no-browser", action="store_true", help="don't open web pages (record them instead)")
    run.add_argument("--id-url", help="id.twitch.tv's address (the fakes)")
    run.add_argument("--helix-url", help="Helix's address (the fakes)")
    run.add_argument("--eventsub-url", help="EventSub's WebSocket address (the fakes)")
    run.add_argument("--set", action="append", type=parse_setting, metavar="NAME=VALUE",
                     help="override a plugin constant (dry runs against the fakes)")
    run.add_argument("--fast-polls", action="store_true", help="poll five times as often (dry runs only)")
    summary = sub.add_parser("summary", help="call counts, validation rhythm and errors in a trace")
    summary.add_argument("trace")
    summary.add_argument("--json", action="store_true", help="print the summary as JSON")
    fixtures = sub.add_parser("fixtures", help="turn a trace into contract fixtures")
    fixtures.add_argument("trace")
    fixtures.add_argument("out")
    args = parser.parse_args(argv)
    if args.command == "run":
        return run_plugin(args)
    if args.command == "summary":
        result = analyse(load(args.trace))
        if args.json:
            print(json.dumps(result, indent=2, ensure_ascii=False))
        else:
            print_summary(result)
        return 0
    data = to_fixtures(load(args.trace), args.trace)
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=2, ensure_ascii=False)
        f.write("\n")
    print(f"{len(data['fixtures'])} fixture(s) and {len(data['events'])} event(s) in {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
