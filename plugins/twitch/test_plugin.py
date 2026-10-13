import base64
import hashlib
import http.server
import itertools
import json
import os
import re
import shutil
import socket
import stat
import struct
import sys
import tempfile
import threading
import time
import unittest
import urllib.parse
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as twitch  # noqa: E402

CLIENT_ID = "wbmytr93xzw8zbg0p1izqyzzc5mbiz"
ME = {"id": "141981764", "login": "perritodev", "display_name": "PerritoDev"}
OTHER = {"id": "12826", "login": "otherstreamer", "display_name": "OtherStreamer"}
VIEWER = {"id": "55555", "login": "someviewer", "display_name": "SomeViewer"}
OLD = {"id": "301", "login": "zx", "display_name": "ZX"}  # a short name from Twitch's early days
USERS = {u["login"]: u for u in (ME, OTHER, VIEWER, OLD)}
ALL_SCOPES = set(twitch.BASE_SCOPES) | set(twitch.SCOPE_GROUP)
NEEDS = {
    ("POST", "/streams/markers"): "channel:manage:broadcast",
    ("PATCH", "/channels"): "channel:manage:broadcast",
    ("POST", "/channels/commercial"): "channel:edit:commercial",
    ("GET", "/channels/ads"): "channel:read:ads",
    ("POST", "/channels/ads/schedule/snooze"): "channel:manage:ads",
    ("POST", "/clips"): "clips:edit",
    ("POST", "/chat/messages"): "user:write:chat",
    ("PUT", "/chat/color"): "user:manage:chat_color",
    ("POST", "/chat/announcements"): "moderator:manage:announcements",
    ("PATCH", "/chat/settings"): "moderator:manage:chat_settings",
    ("GET", "/moderation/shield_mode"): "moderator:manage:shield_mode",
    ("PUT", "/moderation/shield_mode"): "moderator:manage:shield_mode",
    ("POST", "/moderation/bans"): "moderator:manage:banned_users",
    ("DELETE", "/moderation/bans"): "moderator:manage:banned_users",
    ("POST", "/moderation/warnings"): "moderator:manage:warnings",
    ("DELETE", "/moderation/chat"): "moderator:manage:chat_messages",
    ("POST", "/raids"): "channel:manage:raids",
    ("DELETE", "/raids"): "channel:manage:raids",
    ("POST", "/chat/shoutouts"): "moderator:manage:shoutouts",
    ("PUT", "/moderation/automod/settings"): "moderator:manage:automod_settings",
    ("GET", "/channels/vips"): "channel:manage:vips",
    ("POST", "/channels/vips"): "channel:manage:vips",
    ("DELETE", "/channels/vips"): "channel:manage:vips",
    ("GET", "/moderation/moderators"): "channel:manage:moderators",
    ("POST", "/moderation/moderators"): "channel:manage:moderators",
    ("DELETE", "/moderation/moderators"): "channel:manage:moderators",
    ("POST", "/channel_points/custom_rewards"): "channel:manage:redemptions",
    ("PATCH", "/channel_points/custom_rewards"): "channel:manage:redemptions",
    ("PATCH", "/channel_points/custom_rewards/redemptions"): "channel:manage:redemptions",
    ("POST", "/polls"): "channel:manage:polls",
    ("PATCH", "/polls"): "channel:manage:polls",
    ("POST", "/predictions"): "channel:manage:predictions",
    ("PATCH", "/predictions"): "channel:manage:predictions",
    ("GET", "/subscriptions"): "channel:read:subscriptions",
    ("GET", "/chat/chatters"): "moderator:read:chatters",
}
# What GET /users accepts as a login. Exactly which names Twitch refuses is unconfirmed,
# so tests can refuse more with Fake.state["refused_logins"].
LOGIN = re.compile(r"^[a-z0-9_]{1,25}$")
NOW = 1791741600 + 2 * 3600 + 5 * 60  # 2026-10-11T20:05:00Z


def iso(ts):
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(ts))


def manifest():
    with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "perritodeck-plugin.json"), encoding="utf-8") as f:
        return json.load(f)


def wait_for(cond, timeout=3.0, message="condition"):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if cond():
            return
        time.sleep(0.01)
    raise AssertionError(f"timed out waiting for {message}")


class Fake(http.server.BaseHTTPRequestHandler):
    """id.twitch.tv (/oauth2/…) and Helix (/helix/…) in one server."""
    state = {}

    def send_json(self, code, data=None, headers=None):
        body = b"" if data is None else json.dumps(data).encode()
        self.send_response(code)
        if data is not None:
            self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

    def handle_any(self):
        st = Fake.state
        length = self.headers.get("Content-Length")
        raw = self.rfile.read(int(length)) if length else b""
        url = urllib.parse.urlparse(self.path)
        query = urllib.parse.parse_qs(url.query)
        st["calls"].append((self.command, url.path, query, raw, dict(self.headers)))
        if url.path.startswith("/oauth2/"):
            return self.oauth(url.path, dict(urllib.parse.parse_qsl(raw.decode())))
        if not url.path.startswith("/helix/"):
            return self.send_json(404, {"error": "Not Found", "status": 404, "message": ""})
        limit = {"Ratelimit-Limit": "800", "Ratelimit-Remaining": str(st["remaining"]),
                 "Ratelimit-Reset": str(int(st["reset"]))}
        if self.headers.get("Client-Id") != CLIENT_ID:
            return self.send_json(401, {"error": "Unauthorized", "status": 401, "message": "Client ID and OAuth token do not match"})
        if self.headers.get("Authorization") != f"Bearer {st['access']}":
            return self.send_json(401, {"error": "Unauthorized", "status": 401,
                                        "message": st.get("bad_token_message", "Invalid OAuth token")}, limit)
        if st.get("throttle"):
            return self.send_json(429, {"error": "Too Many Requests", "status": 429, "message": ""},
                                  dict(limit, **{"Ratelimit-Remaining": "0"}))
        path, cmd = url.path[len("/helix"):], self.command
        if path in st.get("other_401", {}):  # a 401 about the request, not the token
            return self.send_json(401, {"error": "Unauthorized", "status": 401, "message": st["other_401"][path]}, limit)
        scope = NEEDS.get((cmd, path))
        if scope and scope not in st["scopes"]:
            return self.send_json(401, {"error": "Unauthorized", "status": 401, "message": f"Missing scope: {scope}"}, limit)
        body = json.loads(raw) if raw else None
        one = {k: v[0] for k, v in query.items()}
        st["requests"].append((cmd, path, one, body))
        code, answer = self.helix(cmd, path, query, one, body)
        self.send_json(code, answer, limit)

    def helix(self, cmd, path, query, one, body):
        st = Fake.state
        ok = (204, None)
        bad = lambda message, code=400: (code, {"error": "Bad Request", "status": code, "message": message})  # noqa: E731
        if path == "/users":
            if "login" not in query:
                return 200, {"data": [ME]}
            refused = st.get("refused_logins", set())
            if not all(LOGIN.match(x.lower()) and x.lower() not in refused for x in query["login"]):
                return bad("Invalid login names, emails or IDs in request")  # one bad name spoils the batch
            return 200, {"data": [USERS[x] for x in query["login"] if x in USERS]}
        if path == "/streams":
            return 200, {"data": [s for s in st["streams"] if s["user_id"] in query.get("user_id", [])], "pagination": {}}
        if path == "/channels" and cmd == "GET":
            return 200, {"data": [c for b, c in st["channels"].items() if b in query.get("broadcaster_id", [])]}
        if path == "/channels" and cmd == "PATCH":
            if one.get("broadcaster_id") != ME["id"]:
                return bad("broadcaster_id does not match", 401)
            if not body:
                return bad("No fields specified")
            channel = st["channels"][ME["id"]]
            for key in ("title", "tags", "is_branded_content"):
                if key in body:
                    channel[key] = body[key]
            if "game_id" in body:
                channel["game_id"] = body["game_id"]
                channel["game_name"] = {g["id"]: g["name"] for g in st["games"]}[body["game_id"]]
            for label in body.get("content_classification_labels") or []:
                labels = channel["content_classification_labels"]
                if label["is_enabled"] and label["id"] not in labels:
                    labels.append(label["id"])
                elif not label["is_enabled"] and label["id"] in labels:
                    labels.remove(label["id"])
            return ok
        if path == "/games":
            return 200, {"data": [g for g in st["games"] if g["name"] in query.get("name", [])]}
        if path == "/streams/markers":
            st["markers"].append(body)
            return 200, {"data": [{"id": "123", "created_at": "2026-10-11T20:00:00Z",
                                   "description": body.get("description", ""), "position_seconds": 3844}]}
        if path == "/channels/commercial":
            if not st["live"]:
                return bad("To start a commercial, the broadcaster must be streaming live.")
            st["ads"].append(body)
            return 200, {"data": [{"length": body["length"], "message": "", "retry_after": 480}]}
        if path == "/channels/ads":
            return 200, {"data": [st["ad_schedule"]]}
        if path == "/channels/ads/schedule/snooze":
            s = st["ad_schedule"]
            if s["snooze_count"] == 0:
                return bad("No snoozes available", 429)
            s["snooze_count"] -= 1
            s["next_ad_at"] += 300
            return 200, {"data": [{"snooze_count": s["snooze_count"], "snooze_refresh_at": s["snooze_refresh_at"],
                                   "next_ad_at": s["next_ad_at"]}]}
        if path == "/clips":
            st["clips"].append(one)
            return 202, {"data": [{"id": "FiveWordsForClipSlug",
                                   "edit_url": st.get("edit_url", "https://clips.twitch.tv/FiveWordsForClipSlug/edit")}]}
        if path == "/chat/messages":
            st["chat"].append(body)
            if "badword" in body["message"]:
                return 200, {"data": [{"message_id": "", "is_sent": False,
                                       "drop_reason": {"code": "msg_rejected", "message": "Your message is being checked by mods"}}]}
            return 200, {"data": [{"message_id": "abc-123", "is_sent": True}]}
        if path == "/chat/color":
            st["color"] = one["color"]
            return ok
        if path == "/chat/announcements":
            st["announcements"].append((one, body))
            return ok
        if path == "/chat/settings":
            settings = st["chat_settings"].setdefault(one["broadcaster_id"], {
                "broadcaster_id": one["broadcaster_id"], "emote_mode": False, "follower_mode": False,
                "follower_mode_duration": None, "slow_mode": False, "slow_mode_wait_time": None,
                "subscriber_mode": False, "unique_chat_mode": False, "non_moderator_chat_delay": False,
                "non_moderator_chat_delay_duration": None})
            if cmd == "PATCH":
                if one.get("moderator_id") != ME["id"]:
                    return bad("moderator_id does not match", 401)
                settings.update(body)
            shown = dict(settings)
            if "moderator_id" not in one:  # Twitch shows the delay only to moderators
                shown.pop("non_moderator_chat_delay")
                shown.pop("non_moderator_chat_delay_duration")
            return 200, {"data": [shown]}
        if path == "/channels/followers":
            latest = st["latest_follower"]
            return 200, {"data": [latest] if latest and "moderator:read:followers" in st["scopes"] else [],
                         "total": st["followers"][one["broadcaster_id"]], "pagination": {}}
        if path == "/moderation/shield_mode":
            if cmd == "PUT":
                st["shield"] = body["is_active"]
            return 200, {"data": [{"is_active": st["shield"], "moderator_id": ME["id"]}]}
        if path in ("/moderation/bans", "/moderation/warnings", "/moderation/chat", "/raids", "/chat/shoutouts",
                    "/moderation/automod/settings"):
            st["moderation"].append((cmd, path, one, body))
            if path == "/raids" and cmd == "POST":
                return 200, {"data": [{"created_at": "2026-10-11T20:00:00Z", "is_mature": False}]}
            if path == "/moderation/bans" and cmd == "POST":
                return 200, {"data": [{"broadcaster_id": one["broadcaster_id"], "user_id": body["data"]["user_id"]}]}
            if path == "/moderation/automod/settings":
                return 200, {"data": [{"broadcaster_id": one["broadcaster_id"], "overall_level": body["overall_level"]}]}
            return ok
        if path in ("/channels/vips", "/moderation/moderators"):
            group = st["roles"].setdefault(path, set())
            if cmd == "GET":
                return 200, {"data": [{"user_id": one["user_id"]}] if one["user_id"] in group else []}
            (group.add if cmd == "POST" else group.discard)(one["user_id"])
            return ok
        if path == "/channel_points/custom_rewards":
            rewards = st["rewards"]
            if cmd == "GET":
                found = [r for r in rewards if one.get("only_manageable_rewards") != "true" or r["mine"]]
                return 200, {"data": [{k: v for k, v in r.items() if k != "mine"} for r in found]}
            if cmd == "POST":
                new = {"id": f"r{len(rewards) + 1}", "title": body["title"], "cost": body["cost"], "is_enabled": True,
                       "is_paused": False, "mine": True, "prompt": body.get("prompt", "")}
                rewards.append(new)
                return 200, {"data": [new]}
            r = next(r for r in rewards if r["id"] == one["id"])
            if not r["mine"]:
                return bad("The ID in the Client-Id header must match the client ID used to create the custom reward.", 403)
            r.update(body)
            return 200, {"data": [r]}
        if path == "/channel_points/custom_rewards/redemptions":
            r = next(r for r in st["rewards"] if r["id"] == one["reward_id"])
            if not r["mine"]:
                return bad("The ID in the Client-Id header must match the client ID used to create the custom reward.", 403)
            reds = st["redemptions"].setdefault(r["id"], [])
            if cmd == "GET":
                waiting = [x for x in reds if x["status"] == one.get("status", "UNFULFILLED")]
                n = int(one.get("first", 20))
                page = {"cursor": "next"} if len(waiting) > n else {}
                return 200, {"data": waiting[:n], "pagination": page}
            x = next(x for x in reds if x["id"] == one["id"])
            x["status"] = body["status"]
            return 200, {"data": [x]}
        if path in ("/polls", "/predictions"):
            items = st[path[1:]]
            if cmd == "GET":
                return 200, {"data": items[-1:]}
            if cmd == "POST":
                if path == "/polls":
                    new = {"id": f"p{len(items) + 1}", "title": body["title"], "status": "ACTIVE",
                           "duration": body["duration"], "started_at": iso(st["now"]), "ended_at": None,
                           "choices": [{"id": f"c{i}", "title": c["title"], "votes": 0} for i, c in enumerate(body["choices"])]}
                else:
                    new = {"id": f"x{len(items) + 1}", "title": body["title"], "status": "ACTIVE",
                           "prediction_window": body["prediction_window"], "created_at": iso(st["now"]),
                           "ended_at": None, "winning_outcome_id": None,
                           "outcomes": [{"id": f"o{i}", "title": o["title"], "channel_points": 0, "users": 0}
                                        for i, o in enumerate(body["outcomes"])]}
                items.append(new)
                return 200, {"data": [new]}
            item = next(x for x in items if x["id"] == body["id"])
            item["status"] = body["status"]
            if body.get("winning_outcome_id"):
                item["winning_outcome_id"] = body["winning_outcome_id"]
            return 200, {"data": [item]}
        if path == "/subscriptions":
            if one["broadcaster_id"] != ME["id"]:
                return bad("broadcaster_id must match the User ID in the token", 401)
            return 200, {"data": [], "total": 321, "points": 400, "pagination": {}}
        if path == "/chat/chatters":
            if "moderator_id" not in one:
                return bad("Missing required parameter \"moderator_id\"")
            if one["moderator_id"] != ME["id"]:
                return bad("The ID in moderator_id must match the user ID in the user access token.", 401)
            if one["broadcaster_id"] != ME["id"]:  # not a moderator there
                return bad("The user in moderator_id must have moderator privileges in the broadcaster's chat room.", 403)
            return 200, {"data": [{"user_id": "9", "user_login": "fan", "user_name": "Fan"}], "total": 42,
                         "pagination": {"cursor": "next"}}
        if path == "/eventsub/subscriptions":
            if cmd == "POST":
                st["sub_posts"].append((body["type"], body["transport"].get("session_id"), time.monotonic()))
                if body["transport"].get("session_id") != st["session"]:
                    return bad("websocket transport session does not exist or has already disconnected")
                if st["fail_subs"].get(body["type"]):  # Twitch has a bad moment
                    st["fail_subs"][body["type"]] -= 1
                    return bad("Internal Server Error", 503)
                sub = {"id": f"sub-{len(st['subs']) + 1}", "status": "enabled", "type": body["type"],
                       "version": body["version"], "condition": body["condition"], "transport": body["transport"], "cost": 0}
                st["subs"].append(sub)
                return 202, {"data": [sub], "total": len(st["subs"]), "total_cost": 0, "max_total_cost": 10}
            st["deleted_subs"].append(one["id"])
            return ok
        return 404, {"error": "Not Found", "status": 404, "message": ""}

    def oauth(self, path, form):
        st = Fake.state
        if path == "/oauth2/validate":
            if self.headers.get("Authorization") == f"OAuth {st['access']}":
                return self.send_json(200, {"client_id": CLIENT_ID, "login": ME["login"], "scopes": sorted(st["scopes"]),
                                            "user_id": ME["id"], "expires_in": 5000})
            return self.send_json(401, {"status": 401, "message": "invalid access token"})
        if path == "/oauth2/revoke":
            if st.get("revoke_down"):
                return self.send_json(503, {"status": 503, "message": "Service Unavailable"})
            if form.get("client_id") != CLIENT_ID:
                return self.send_json(404, {"status": 404, "message": "client does not exist"})
            if form.get("token") != st["access"]:
                return self.send_json(400, {"status": 400, "message": "Invalid token"})
            st["revoked"].append(form["token"])
            st["access"] = st["refresh"] = "revoked"
            return self.send_json(200)
        if form.get("client_id") != CLIENT_ID:
            return self.send_json(400, {"status": 400, "message": "invalid client"})
        if path == "/oauth2/device":
            st["device_scopes"] = form.get("scopes")
            return self.send_json(200, {"device_code": "dev-code-1", "expires_in": 1800, "interval": 0,
                                        "user_code": "ABCDEFGH",
                                        "verification_uri": st.get("verification_uri",
                                                                   "https://www.twitch.tv/activate?public=true&device-code=ABCDEFGH")})
        if path == "/oauth2/token" and form.get("grant_type") == twitch.DEVICE_GRANT:
            if form.get("device_code") != "dev-code-1" or st.get("device_used"):
                return self.send_json(400, {"status": 400, "message": "invalid device code"})
            st["device_polls"] += 1
            if st["device_polls"] <= st["pending_polls"]:
                return self.send_json(400, {"status": 400, "message": "authorization_pending"})
            st["device_used"] = True
            st["access"], st["refresh"] = "access-1", "refresh-1"
            st["scopes"] = set(form["scopes"].split(" "))
            return self.send_json(200, {"access_token": "access-1", "expires_in": 14400, "refresh_token": "refresh-1",
                                        "scope": sorted(st["scopes"]), "token_type": "bearer"})
        if path == "/oauth2/token" and form.get("grant_type") == "refresh_token":
            if "client_secret" in form:
                return self.send_json(400, {"status": 400, "message": "public clients have no secret"})
            if form.get("refresh_token") != st["refresh"]:
                return self.send_json(400, {"error": "Bad Request", "status": 400, "message": "Invalid refresh token"})
            st["refreshes"] += 1
            # Public clients: refresh tokens are single use, a new one comes each time.
            st["access"], st["refresh"] = f"access-r{st['refreshes']}", f"refresh-r{st['refreshes']}"
            return self.send_json(200, {"access_token": st["access"], "refresh_token": st["refresh"], "expires_in": 14400,
                                        "scope": sorted(st["scopes"]), "token_type": "bearer"})
        self.send_json(400, {"status": 400, "message": "unsupported grant"})

    do_GET = do_POST = do_PUT = do_PATCH = do_DELETE = handle_any

    def log_message(self, *args):
        pass


# --- a fake EventSub WebSocket server --------------------------------------------------------

GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


class WsConn:
    def __init__(self, sock, path):
        self.sock, self.path, self.buf = sock, path, b""
        self.closed_by_client = False
        self.lock = threading.Lock()

    def read(self, n):
        while len(self.buf) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise ConnectionError("client went away")
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def send(self, message, pause_after=None):
        """Send a text message; with pause_after=n, its first n bytes, a pause, then the rest
        (as a slow network or TLS can deliver a frame)."""
        data = twitch.encode_frame(twitch.OP_TEXT, json.dumps(message).encode(), mask=False)
        with self.lock:
            if pause_after:
                self.sock.sendall(data[:pause_after])
                time.sleep(0.3)
                data = data[pause_after:]
            self.sock.sendall(data)

    def ping(self, payload):
        with self.lock:
            self.sock.sendall(twitch.encode_frame(twitch.OP_PING, payload, mask=False))


class FakeEventSub:
    """Accepts WebSocket connections, sends session_welcome, and lets tests push
    notifications, keepalives, reconnects and revocations."""

    def __init__(self, keepalive=10):
        self.keepalive = keepalive
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(8)
        self.port = self.listener.getsockname()[1]
        self.url = f"ws://127.0.0.1:{self.port}/ws"
        self.conns = []
        self.sessions = itertools.count(1)
        self.ids = itertools.count(1)
        self.unmasked = 0
        self.pongs = []  # payloads of the client's pongs
        # Like Twitch, close a session that has no subscription this long after its
        # welcome, with 4003 (None: never).
        self.close_unused = None
        self.unused_closed = []  # sessions closed for that
        self.stopped = False
        threading.Thread(target=self._accept, daemon=True).start()

    def _accept(self):
        while True:
            try:
                sock, _ = self.listener.accept()
            except OSError:
                return
            threading.Thread(target=self._serve, args=(sock,), daemon=True).start()

    def _serve(self, sock):
        head = b""
        while b"\r\n\r\n" not in head:
            chunk = sock.recv(4096)
            if not chunk:
                return
            head += chunk
        lines = head.decode().split("\r\n")
        headers = {l.split(":", 1)[0].lower(): l.split(":", 1)[1].strip() for l in lines[1:] if ":" in l}
        accept = base64.b64encode(hashlib.sha1((headers["sec-websocket-key"] + GUID).encode()).digest()).decode()
        sock.sendall(("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                      f"Sec-WebSocket-Accept: {accept}\r\n\r\n").encode())
        conn = WsConn(sock, lines[0].split(" ")[1])
        session = f"sess-{next(self.sessions)}"
        conn.session = session
        self.conns.append(conn)
        Fake.state["session"] = session
        conn.send(self.message("session_welcome", {"session": {
            "id": session, "status": "connected", "connected_at": "2026-10-11T20:00:00.123456789Z",
            "keepalive_timeout_seconds": self.keepalive, "reconnect_url": None}}))
        if self.close_unused is not None:
            timer = threading.Timer(self.close_unused, self._close_if_unused, (conn,))
            timer.daemon = True
            timer.start()
        try:
            while True:  # the client only sends pongs and close frames
                b0, b1 = conn.read(2)
                if not b1 & 0x80:
                    self.unmasked += 1
                n = b1 & 0x7F
                if n == 126:
                    (n,) = struct.unpack("!H", conn.read(2))
                key = conn.read(4) if b1 & 0x80 else b"\0\0\0\0"
                payload = twitch.mask_bytes(conn.read(n), key)
                if b0 & 0x0F == twitch.OP_PONG:
                    self.pongs.append(payload)
                if b0 & 0x0F == twitch.OP_CLOSE:
                    conn.closed_by_client = True
                    break
        except (OSError, ConnectionError):
            pass
        finally:
            sock.close()

    def _close_if_unused(self, conn):
        if self.stopped or conn.closed_by_client:
            return
        if any(s["transport"]["session_id"] == conn.session for s in Fake.state["subs"]):
            return
        if Fake.state.get("session") == conn.session:
            Fake.state["session"] = None  # POSTs for it fail from now on
        self.unused_closed.append(conn.session)
        try:
            with conn.lock:
                conn.sock.sendall(twitch.encode_frame(twitch.OP_CLOSE, struct.pack("!H", 4003) + b"connection unused",
                                                      mask=False))
            conn.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass

    def message(self, kind, payload, **meta):
        return {"metadata": dict({"message_id": f"m-{next(self.ids)}", "message_type": kind,
                                  "message_timestamp": "2026-10-11T20:00:00.5Z"}, **meta), "payload": payload}

    @property
    def current(self):
        return self.conns[-1]

    def notify(self, sub_type, event, message_id=None):
        sub = next(s for s in reversed(Fake.state["subs"]) if s["type"] == sub_type)
        message = self.message("notification", {"subscription": sub, "event": event},
                               subscription_type=sub_type, subscription_version=sub["version"])
        if message_id:
            message["metadata"]["message_id"] = message_id
        self.current.send(message)

    def stop(self):
        self.stopped = True
        try:
            self.listener.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.listener.close()
        for conn in self.conns:
            try:
                conn.sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            conn.sock.close()


class Base(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fake)
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
        cls.base = f"http://127.0.0.1:{cls.server.server_port}"

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()

    def setUp(self):
        Fake.state = {
            "calls": [], "requests": [], "access": "access-0", "refresh": "refresh-0", "refreshes": 0,
            "scopes": set(ALL_SCOPES), "remaining": 799, "reset": time.time() + 60, "device_polls": 0,
            "pending_polls": 2, "live": True, "markers": [], "ads": [], "clips": [], "chat": [], "chat_settings": {},
            "shield": False, "announcements": [], "moderation": [], "roles": {}, "now": NOW,
            "streams": [{"id": "40952121085", "user_id": ME["id"], "user_login": ME["login"], "type": "live",
                         "title": "Speedrunning", "viewer_count": 78, "started_at": "2026-10-11T18:00:00Z"}],
            "channels": {
                ME["id"]: {"broadcaster_id": ME["id"], "broadcaster_login": ME["login"], "title": "Speedrunning",
                           "game_id": "509658", "game_name": "Just Chatting", "tags": ["English", "Speedrun"],
                           "content_classification_labels": [], "is_branded_content": False},
                OTHER["id"]: {"broadcaster_id": OTHER["id"], "broadcaster_login": OTHER["login"], "title": "Chill",
                              "game_id": "", "game_name": "", "tags": [], "content_classification_labels": [],
                              "is_branded_content": False},
            },
            "games": [{"id": "509658", "name": "Just Chatting"}, {"id": "27471", "name": "Minecraft"},
                      {"id": "2692", "name": "Super Mario 64"}],
            "followers": {ME["id"]: 1234, OTHER["id"]: 5},
            "latest_follower": {"user_id": "777", "user_login": "newestfan", "user_name": "NewestFan",
                                "followed_at": "2026-10-11T19:00:00Z"},
            "ad_schedule": {"next_ad_at": NOW + 754, "last_ad_at": NOW - 1800, "duration": 90,
                            "preroll_free_time": 600, "snooze_count": 3, "snooze_refresh_at": NOW + 3600},
            "rewards": [{"id": "r1", "title": "Hydrate!", "cost": 500, "is_enabled": True, "is_paused": False, "mine": True},
                        {"id": "r9", "title": "Song request", "cost": 1000, "is_enabled": True, "is_paused": True,
                         "mine": False}],
            "redemptions": {"r1": [{"id": "red1", "status": "UNFULFILLED"}, {"id": "red2", "status": "UNFULFILLED"}]},
            "polls": [], "predictions": [], "subs": [], "deleted_subs": [], "session": None, "fail_subs": {},
            "revoked": [], "sub_posts": [],
        }
        self.data = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.data, True)
        twitch.plugin._data_dir = self.data
        self.sent = []
        twitch.plugin._send = self.sent.append
        twitch.plugin.settings = {"client_id": CLIENT_ID}
        twitch.plugin.watching = set()
        for name, value in (("ID_URL", self.base), ("HELIX_URL", self.base + "/helix"),
                            ("poll_soon", lambda: None), ("MIN_DEVICE_POLL", 0.01)):
            patcher = mock.patch.object(twitch, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        twitch._limit["until"] = 0.0
        twitch._users.clear()
        twitch._reported.clear()
        twitch._published.clear()
        twitch._session.clear()
        twitch._ad_until.clear()
        twitch._hype.clear()
        twitch._ads["cache"] = None
        twitch._votes.update(poll={}, prediction={})
        twitch._check.update(next=0.0, told=False)
        twitch._sign_in["current"] = None
        twitch.reschedule()
        twitch.load_state()
        twitch.events.stop()
        twitch.events = twitch.EventSub()
        self.addCleanup(lambda: twitch.events.stop())

    def signed_in(self, expires_in=14400, validated=True, scopes=None):
        twitch.save_tokens({"client_id": CLIENT_ID, "access_token": "access-0", "refresh_token": "refresh-0",
                            "expires_at": time.time() + expires_in, "scope": sorted(scopes or ALL_SCOPES),
                            "user_id": ME["id"], "login": ME["login"],
                            "validated_at": time.time() if validated else 0})

    def values(self):
        return {m["id"]: m["value"] for m in self.sent if m.get("event") == "value"}

    def logs(self):
        return [m["message"] for m in self.sent if m.get("event") == "log"]

    def helix_calls(self, method=None, path=None):
        return [c for c in Fake.state["calls"] if c[1].startswith("/helix/")
                and (method is None or c[0] == method) and (path is None or c[1] == "/helix" + path)]

    def last(self, method, path):
        return [r for r in Fake.state["requests"] if r[0] == method and r[1] == path][-1]


class SignInTest(Base):
    def test_device_sign_in(self):
        with mock.patch.object(twitch, "open_for_user") as browser:
            answer = twitch.sign_in({})
        self.assertEqual(answer, "go to twitch.tv/activate and enter ABCDEFGH")
        browser.assert_called_once_with("https://www.twitch.tv/activate?public=true&device-code=ABCDEFGH")
        self.assertEqual(set(Fake.state["device_scopes"].split(" ")), ALL_SCOPES)
        attempt = twitch._sign_in["current"]
        self.assertTrue(attempt.done.wait(5))
        self.assertIsNone(attempt.error)
        self.assertEqual(Fake.state["device_polls"], 3)  # two "authorization_pending", then the token
        poll = next(c for c in Fake.state["calls"] if c[1] == "/oauth2/token")
        form = dict(urllib.parse.parse_qsl(poll[3].decode()))
        self.assertEqual(form["grant_type"], "urn:ietf:params:oauth:grant-type:device_code")
        self.assertEqual((form["client_id"], form["device_code"]), (CLIENT_ID, "dev-code-1"))
        tokens = twitch.load_tokens()
        self.assertEqual((tokens["access_token"], tokens["refresh_token"], tokens["user_id"], tokens["login"]),
                         ("access-1", "refresh-1", ME["id"], ME["login"]))
        if os.name == "posix":
            self.assertEqual(stat.S_IMODE(os.stat(os.path.join(self.data, "tokens.json")).st_mode), 0o600)
        for m in self.sent:  # tokens never reach the log
            self.assertNotIn("access-1", json.dumps(m))
            self.assertNotIn("refresh-1", json.dumps(m))
        # The log names the account, so approving as the wrong channel shows.
        self.assertIn("signed in to Twitch as perritodev", self.logs())
        # A fresh token isn't validated again straight away.
        twitch.safe_poll()
        self.assertFalse([c for c in Fake.state["calls"] if c[1] == "/oauth2/validate"])

    def test_sign_in_opens_only_twitch_pages(self):
        for odd in ("http://www.twitch.tv/activate", "https://twitch.tv.example.com/activate",
                    "https://evil.example\\@www.twitch.tv/activate", "https://user@www.twitch.tv/activate",
                    "https://www.twitch.tv:8443/activate", "javascript:alert(1)"):
            Fake.state["verification_uri"] = odd
            with mock.patch.object(twitch, "open_for_user") as browser:
                self.assertEqual(twitch.sign_in({}), "go to twitch.tv/activate and enter ABCDEFGH")
            browser.assert_called_once_with("https://www.twitch.tv/activate")
            self.assertTrue(any("isn't https on twitch.tv" in m for m in self.logs()), odd)
            twitch._sign_in["current"].cancel("test")
        for fine in ("https://www.twitch.tv/activate?device-code=X", "https://twitch.tv/activate",
                     "https://id.twitch.tv/device"):
            self.assertTrue(twitch.twitch_page(fine), fine)
        for odd in (None, "", " https://www.twitch.tv/", "https://www.twitch.tv/\n", "https://[::1/"):
            self.assertFalse(twitch.twitch_page(odd), odd)

    def test_scopes_asked_for(self):
        scopes = twitch.wanted_scopes()
        self.assertIn("moderator:read:chatters", scopes)  # the chatters value
        self.assertIn("moderator:manage:automod_settings", scopes)
        self.assertNotIn("moderator:read:automod_settings", scopes)  # the plugin never reads AutoMod settings
        self.assertEqual(len(scopes), len(set(scopes)))

    def test_feature_groups_choose_the_scopes(self):
        twitch.plugin.settings = {"client_id": CLIENT_ID, "group_polls": False, "group_moderation": False,
                                  "group_events": True, "group_points": "", "group_ads": True}
        scopes = twitch.wanted_scopes()
        self.assertIn("channel:manage:broadcast", scopes)
        self.assertIn("bits:read", scopes)
        self.assertIn("channel:manage:redemptions", scopes)  # "" counts as the default (on)
        self.assertNotIn("channel:manage:polls", scopes)
        self.assertNotIn("moderator:manage:banned_users", scopes)
        with mock.patch.object(twitch, "open_for_user"):
            twitch.sign_in({})
        self.assertTrue(twitch._sign_in["current"].done.wait(5))
        self.assertEqual(Fake.state["device_scopes"].split(" "), scopes)
        # An action of a turned-off group says what to do, without calling Twitch.
        with self.assertRaisesRegex(RuntimeError, "sign in again with Polls and predictions turned on in the plugin's settings"):
            twitch.poll_start({"title": "Q?", "choices": "a, b"})
        with self.assertRaisesRegex(RuntimeError, "sign in again with Moderation turned on"):
            twitch.ban({"user": "someviewer"})
        self.assertFalse(self.helix_calls("POST", "/polls"))
        # Turned on since, but not signed in again.
        twitch.plugin.settings["group_polls"] = True
        with self.assertRaisesRegex(RuntimeError, r"doesn't allow that \(channel:manage:polls\); press \"Sign in"):
            twitch.poll_start({"title": "Q?", "choices": "a, b"})

    def test_device_sign_in_expired_code(self):
        Fake.state["device_used"] = True
        with mock.patch.object(twitch, "open_for_user"):
            twitch.sign_in({})
        attempt = twitch._sign_in["current"]
        self.assertTrue(attempt.done.wait(5))
        self.assertIn("expired", attempt.error)
        self.assertIsNone(twitch.load_tokens())

    def test_sign_in_errors(self):
        twitch.plugin.settings = {}
        with self.assertRaisesRegex(RuntimeError, "client ID"):
            twitch.sign_in({})
        twitch.plugin.settings = {"client_id": "unknown"}
        with self.assertRaisesRegex(RuntimeError, "doesn't know that client ID"):
            twitch.sign_in({})

    def test_refresh_on_401(self):
        self.signed_in()
        Fake.state["access"] = "access-new-elsewhere"  # the saved token was revoked
        self.assertIsNone(twitch.marker({"description": "Great play"}))
        self.assertEqual(Fake.state["markers"], [{"user_id": ME["id"], "description": "Great play"}])
        tokens = twitch.load_tokens()
        self.assertEqual((tokens["access_token"], tokens["refresh_token"]), ("access-r1", "refresh-r1"))
        refresh = next(c for c in Fake.state["calls"] if c[1] == "/oauth2/token")
        self.assertEqual(dict(urllib.parse.parse_qsl(refresh[3].decode())),
                         {"client_id": CLIENT_ID, "grant_type": "refresh_token", "refresh_token": "refresh-0"})
        auths = [c[4]["Authorization"] for c in self.helix_calls("POST", "/streams/markers")]
        self.assertEqual(auths, ["Bearer access-0", "Bearer access-r1"])
        Fake.state["access"] = "revoked-again"
        twitch.marker({})
        self.assertEqual(twitch.load_tokens()["refresh_token"], "refresh-r2")

    def test_refresh_before_expiry(self):
        self.signed_in(expires_in=30)
        Fake.state["access"] = "access-r1"
        twitch.marker({})
        self.assertEqual(len(self.helix_calls("POST", "/streams/markers")), 1)
        self.assertEqual(Fake.state["refreshes"], 1)

    def test_invalid_refresh_token_drops_the_sign_in(self):
        self.signed_in(expires_in=0)
        Fake.state["refresh"] = "something-else"
        with self.assertRaisesRegex(RuntimeError, "Sign in to Twitch"):
            twitch.marker({})
        self.assertIsNone(twitch.load_tokens())

    def test_missing_scope_is_not_refreshed(self):
        self.signed_in()
        Fake.state["scopes"].discard("clips:edit")
        with self.assertRaisesRegex(RuntimeError, r"doesn't allow that \(Missing scope: clips:edit\)"):
            twitch.clip({})
        self.assertEqual(Fake.state["refreshes"], 0)
        # A Helix "Missing scope" for a group scope gets the plain advice.
        Fake.state["scopes"].discard("moderator:manage:chat_messages")
        twitch.plugin.settings["group_moderation"] = False
        with mock.patch.object(twitch, "need", lambda *s: None):
            with self.assertRaisesRegex(RuntimeError, "sign in again with Moderation turned on"):
                twitch.clear_chat({})

    def test_401_about_the_request_is_not_refreshed(self):
        self.signed_in()
        # Twitch answers 401 for a request the token can't make, too: no refresh for those,
        # and the key and log show what Twitch said. (Polled every minute, a refresh each
        # time would churn through single-use refresh tokens.)
        twitch.plugin.watching = {"subscribers:otherstreamer"}
        twitch.poll_counts()
        twitch.poll_counts()
        self.assertEqual(Fake.state["refreshes"], 0)
        self.assertEqual(self.values()["subscribers:otherstreamer"], "–")
        self.assertIn("subscribers of otherstreamer: Twitch said 401: broadcaster_id must match the User ID in the token",
                      self.logs())
        self.assertFalse(self.validates())  # the wording was clear enough
        # Wording it doesn't know: Twitch is asked whether the token still works.
        Fake.state["other_401"] = {"/streams/markers": "Something about this request"}
        with self.assertRaisesRegex(RuntimeError, "^Twitch said 401: Something about this request$"):
            twitch.marker({})
        self.assertEqual((len(self.validates()), Fake.state["refreshes"]), (1, 0))
        del Fake.state["other_401"]
        Fake.state["access"], Fake.state["bad_token_message"] = "elsewhere", "Token not accepted"
        twitch.marker({})  # the token doesn't work: refreshed, and sent again
        self.assertEqual((len(self.validates()), Fake.state["refreshes"]), (2, 1))
        self.assertEqual(len(Fake.state["markers"]), 1)

    def validates(self):
        return [c for c in Fake.state["calls"] if c[1] == "/oauth2/validate"]

    def test_validation_at_start_and_hourly(self):
        self.signed_in()  # checked recently by an earlier run: still checked at start
        clock = [1000.0]
        with mock.patch.object(twitch.time, "monotonic", lambda: clock[0]):
            twitch.safe_poll()  # no key shows a Twitch value: it's checked all the same
            self.assertEqual(len(self.validates()), 1)
            self.assertEqual(self.validates()[0][4]["Authorization"], "OAuth access-0")
            self.assertIn("signed in to Twitch as perritodev", self.logs())
            clock[0] += 3599
            twitch.safe_poll()
            self.assertEqual(len(self.validates()), 1)
            clock[0] += 2
            twitch.safe_poll()
            self.assertEqual(len(self.validates()), 2)
            self.assertEqual(self.logs().count("signed in to Twitch as perritodev"), 1)  # said once per run
            # While Twitch can't be reached the sign-in is kept, and checked again a minute later.
            clock[0] += 3601
            with mock.patch.object(twitch, "id_request", side_effect=twitch.TwitchError("couldn't reach Twitch: down")):
                twitch.safe_poll()
                twitch.safe_poll()
                self.assertEqual(twitch.id_request.call_count, 1)
            self.assertIsNotNone(twitch.load_tokens())
            self.assertTrue(any("couldn't reach Twitch" in m for m in self.logs()))
            clock[0] += 61
            twitch.safe_poll()
            self.assertEqual(len(self.validates()), 3)
            self.assertTrue(any("sign-in: working again" in m for m in self.logs()))
        self.assertFalse(self.helix_calls())

    def test_validation_refreshes_once_then_asks_to_sign_in_again(self):
        self.signed_in()
        twitch.plugin.watching = {"live:me"}
        Fake.state["access"] = "elsewhere"  # the saved token stopped working
        twitch.safe_poll()
        self.assertEqual(Fake.state["refreshes"], 1)
        self.assertEqual(twitch.load_tokens()["access_token"], "access-r1")
        self.assertEqual(self.values()["live:me"], "on")
        # Then the refresh token stops working too (the app was disconnected on twitch.tv).
        Fake.state["access"], Fake.state["refresh"] = "gone", "gone"
        twitch._check["next"] = 0.0
        twitch.safe_poll()
        self.assertIsNone(twitch.load_tokens())
        self.assertEqual(Fake.state["refreshes"], 1)
        self.assertIn('sign-in: Twitch ended the sign-in; press "Sign in to Twitch" again', self.logs())
        self.assertEqual(self.values()["live:me"], "–")
        calls = len(Fake.state["calls"])
        twitch.safe_poll()
        self.assertEqual(len(Fake.state["calls"]), calls)  # nothing more is sent

    def test_sign_out_revokes_the_token(self):
        self.signed_in()
        twitch.plugin.watching = {"live:me"}
        self.assertIsNone(twitch.sign_out({}))
        revoke = [c for c in Fake.state["calls"] if c[1] == "/oauth2/revoke"]
        self.assertEqual(len(revoke), 1)
        self.assertEqual(dict(urllib.parse.parse_qsl(revoke[0][3].decode())), {"client_id": CLIENT_ID, "token": "access-0"})
        self.assertEqual(Fake.state["revoked"], ["access-0"])
        self.assertIsNone(twitch.load_tokens())
        self.assertEqual(self.values()["live:me"], "–")
        self.assertIn("signed out of Twitch", self.logs())
        # Signed out already: nothing to revoke.
        self.assertIsNone(twitch.sign_out({}))
        self.assertEqual(len([c for c in Fake.state["calls"] if c[1] == "/oauth2/revoke"]), 1)

    def test_sign_out_refreshes_an_expired_token_to_revoke_it(self):
        self.signed_in(expires_in=0)
        self.assertIsNone(twitch.sign_out({}))
        self.assertEqual(Fake.state["revoked"], ["access-r1"])  # the refresh token behind it is ended too
        self.assertIsNone(twitch.load_tokens())

    def test_sign_out_when_twitch_cant_end_it(self):
        self.signed_in()
        Fake.state["revoke_down"] = True
        self.assertEqual(twitch.sign_out({}), "signed out here, but Twitch couldn't end the sign-in; see the log")
        self.assertIsNone(twitch.load_tokens())  # forgotten here all the same
        warning = next(m for m in self.sent if m.get("event") == "log" and m["level"] == "warn")
        self.assertIn("couldn't end the sign-in at Twitch (Twitch said 503: Service Unavailable)", warning["message"])
        self.assertIn("Connections on twitch.tv", warning["message"])
        # A token Twitch had ended already: signed out with nothing to warn about.
        Fake.state["revoke_down"] = False
        self.signed_in()
        Fake.state["access"] = "elsewhere"
        self.assertIsNone(twitch.sign_out({}))
        self.assertIsNone(twitch.load_tokens())

    def test_sign_out_cancels_a_waiting_sign_in(self):
        Fake.state["pending_polls"] = 1000
        with mock.patch.object(twitch, "open_for_user"):
            twitch.sign_in({})
        attempt = twitch._sign_in["current"]
        twitch.sign_out({})
        self.assertTrue(attempt.done.wait(5))
        self.assertEqual(attempt.error, "cancelled by signing out")
        self.assertIsNone(twitch.load_tokens())

    def test_sign_out_while_the_approval_is_on_its_way(self):
        # Twitch has approved, and the plugin is asking which account it is, when Sign out
        # is pressed: it stays signed out, and the token Twitch just gave is ended there.
        Fake.state["pending_polls"] = 0
        asking, answer = threading.Event(), threading.Event()
        real_me = twitch.me

        def slow_me(token, cid):
            asking.set()
            answer.wait(5)
            return real_me(token, cid)

        with mock.patch.object(twitch, "me", slow_me), mock.patch.object(twitch, "open_for_user"):
            twitch.sign_in({})
            attempt = twitch._sign_in["current"]
            self.assertTrue(asking.wait(5))
            self.assertIsNone(twitch.sign_out({}))
            answer.set()
            self.assertTrue(attempt.done.wait(5))
        self.assertEqual(attempt.error, "cancelled by signing out")
        self.assertIsNone(twitch.load_tokens())
        self.assertFalse(os.path.exists(os.path.join(self.data, "tokens.json")))
        self.assertEqual(Fake.state["revoked"], ["access-1"])
        self.assertIn("signed out of Twitch", self.logs())
        self.assertNotIn("signed in to Twitch as perritodev", self.logs())
        # Signed in normally, the account is named once (by the attempt, not again at finish).
        Fake.state.update(device_used=False, device_polls=0)
        with mock.patch.object(twitch, "open_for_user"):
            twitch.sign_in({})
        self.assertTrue(twitch._sign_in["current"].done.wait(5))
        self.assertEqual(self.logs().count("signed in to Twitch as perritodev"), 1)
        self.assertEqual(twitch.load_tokens()["access_token"], "access-1")

    def test_not_signed_in(self):
        with self.assertRaisesRegex(RuntimeError, "not signed in"):
            twitch.marker({})
        twitch.plugin.watching = {"live:me", "viewers:otherstreamer", "last_cheer:me"}
        twitch.safe_poll()
        self.assertEqual(self.values(), {"live:me": "–", "viewers:otherstreamer": "–", "last_cheer:me": "–"})
        self.assertFalse(self.helix_calls())

    def test_rate_limit(self):
        self.signed_in()
        Fake.state["throttle"] = True
        Fake.state["reset"] = time.time() + 20
        with self.assertRaisesRegex(RuntimeError, "slow down"):
            twitch.marker({})
        Fake.state["throttle"] = False
        calls = len(self.helix_calls())
        with self.assertRaisesRegex(RuntimeError, r"try again in \d+ s"):
            twitch.marker({})
        twitch.plugin.watching = {"live:me"}
        twitch.safe_poll()
        self.assertEqual(len(self.helix_calls()), calls)  # neither sent
        twitch._limit["until"] = 0.0
        Fake.state["remaining"] = 1
        twitch.marker({})
        self.assertGreater(twitch.wait_left(), 10)


class ActionTest(Base):
    def setUp(self):
        super().setUp()
        self.signed_in()

    def test_marker_and_clip_values(self):
        twitch.marker({"description": "x" * 200})
        self.assertEqual(len(Fake.state["markers"][0]["description"]), 140)
        with mock.patch.object(twitch, "open_for_user") as browser:
            self.assertIsNone(twitch.clip({}))
            browser.assert_not_called()
            twitch.clip({"channel": "OtherStreamer", "open": True})
            browser.assert_called_once_with("https://clips.twitch.tv/FiveWordsForClipSlug/edit")
            twitch.open_last_clip({})
            browser.assert_called_with("https://clips.twitch.tv/FiveWordsForClipSlug")
        self.assertEqual(Fake.state["clips"], [{"broadcaster_id": ME["id"]}, {"broadcaster_id": OTHER["id"]}])
        with mock.patch.object(twitch, "open_for_user", side_effect=OSError("no browser")):
            with self.assertRaisesRegex(RuntimeError, "couldn't open a web browser"):
                twitch.open_last_clip({})
            self.assertIn("couldn't open a web browser", twitch.clip({"open": True}))
        v = self.values()
        self.assertEqual(v["last_marker"], "1:04:04")
        self.assertRegex(v["last_clip"], r"^\d\d:\d\d$")
        # Remembered across restarts.
        twitch._state.clear()
        twitch.load_state()
        self.assertEqual(twitch._state["last_marker"], "1:04:04")
        with self.assertRaisesRegex(RuntimeError, "no channel called nobodyhere"):
            twitch.clip({"channel": "nobodyhere"})

    def test_clip_editor_address_must_be_twitch(self):
        Fake.state["edit_url"] = "https://clips.example.com/edit"
        with mock.patch.object(twitch, "open_for_user") as browser:
            self.assertEqual(twitch.clip({"open": True}),
                             "made the clip, but didn't open the editor: Twitch's address for it isn't a twitch.tv page")
            browser.assert_not_called()
            self.assertIsNone(twitch._state["last_clip_edit"])
            twitch.open_last_clip({"editor": True})  # the clip page instead
            browser.assert_called_once_with("https://clips.twitch.tv/FiveWordsForClipSlug")
        self.assertTrue(any("isn't https on twitch.tv" in m for m in self.logs()))
        # An editor address in a tampered state file isn't opened either.
        twitch.update_state(last_clip_id="a/../b?c", last_clip_edit="http://www.twitch.tv/edit")
        with mock.patch.object(twitch, "open_for_user") as browser:
            twitch.open_last_clip({"editor": True})
            browser.assert_called_once_with("https://clips.twitch.tv/a%2F..%2Fb%3Fc")

    def test_ads(self):
        self.assertIsNone(twitch.ad({"length": "90"}))
        self.assertEqual(Fake.state["ads"], [{"broadcaster_id": ME["id"], "length": 90}])
        with self.assertRaisesRegex(RuntimeError, "30, 60, 90"):
            twitch.ad({"length": "45"})
        self.assertIsNone(twitch.snooze_ad({}))
        self.assertEqual(self.last("POST", "/channels/ads/schedule/snooze")[2], {"broadcaster_id": ME["id"]})
        Fake.state["ad_schedule"]["snooze_count"] = 1
        self.assertEqual(twitch.snooze_ad({}), "snoozed; that was the last snooze for now")
        with self.assertRaisesRegex(RuntimeError, "No snoozes available"):
            twitch.snooze_ad({})
        Fake.state["live"] = False
        with self.assertRaisesRegex(RuntimeError, "must be streaming live"):
            twitch.ad({})

    def test_chat(self):
        self.assertIsNone(twitch.chat({"message": "Hello chat!"}))
        twitch.chat({"message": "hi", "channel": "otherstreamer"})
        self.assertEqual(Fake.state["chat"], [
            {"broadcaster_id": ME["id"], "sender_id": ME["id"], "message": "Hello chat!"},
            {"broadcaster_id": OTHER["id"], "sender_id": ME["id"], "message": "hi"}])
        with self.assertRaisesRegex(RuntimeError, "didn't send it: Your message is being checked"):
            twitch.chat({"message": "badword"})
        with self.assertRaisesRegex(RuntimeError, "enter the message"):
            twitch.chat({"message": " "})

    def test_saved_messages_cycle(self):
        settings = {"messages": "One | Two|Three"}
        for _ in range(4):
            twitch.chat_saved(settings)
        self.assertEqual([m["message"] for m in Fake.state["chat"]], ["One", "Two", "Three", "One"])
        with mock.patch.object(twitch.random, "choice", lambda items: items[-1]):
            twitch.chat_saved(dict(settings, order="random"))
        self.assertEqual(Fake.state["chat"][-1]["message"], "Three")
        with self.assertRaisesRegex(RuntimeError, "separated by |"):
            twitch.chat_saved({"messages": " | "})

    def test_chat_colour_and_announcement(self):
        twitch.chat_color({"color": "Blue Violet"})
        self.assertEqual(Fake.state["color"], "blue_violet")
        twitch.chat_color({"color": "#9146FF"})
        self.assertEqual(Fake.state["color"], "#9146ff")
        self.assertEqual(self.helix_calls("PUT", "/chat/color")[-1][2], {"user_id": [ME["id"]], "color": ["#9146ff"]})
        with self.assertRaisesRegex(RuntimeError, "Turbo or Prime"):
            twitch.chat_color({"color": "mauve"})
        twitch.announce({"message": "Giveaway soon!", "color": "purple"})
        self.assertEqual(Fake.state["announcements"][-1],
                         ({"broadcaster_id": ME["id"], "moderator_id": ME["id"]}, {"message": "Giveaway soon!", "color": "purple"}))
        with self.assertRaisesRegex(RuntimeError, "primary, blue"):
            twitch.announce({"message": "x", "color": "pink"})

    def test_set_info_presets_and_tags(self):
        self.assertIsNone(twitch.set_info({"title": "New title", "category": "Minecraft"}))
        patch = self.last("PATCH", "/channels")
        self.assertEqual(patch[2], {"broadcaster_id": ME["id"]})
        self.assertEqual(patch[3], {"title": "New title", "game_id": "27471"})
        with self.assertRaisesRegex(RuntimeError, "no category called 'Mincraft'"):
            twitch.set_info({"category": "Mincraft"})
        with self.assertRaisesRegex(RuntimeError, "a title, a category or both"):
            twitch.set_info({})
        with self.assertRaisesRegex(RuntimeError, "140 characters"):
            twitch.set_info({"title": "x" * 141})
        twitch.plugin.settings["preset_2"] = "Mario time | Super Mario 64 | Speedrun, English"
        twitch.apply_preset({"preset": "2"})
        self.assertEqual(self.last("PATCH", "/channels")[3],
                         {"title": "Mario time", "game_id": "2692", "tags": ["Speedrun", "English"]})
        twitch.plugin.settings["preset_3"] = "Just a title"
        twitch.apply_preset({"preset": "3"})
        self.assertEqual(self.last("PATCH", "/channels")[3], {"title": "Just a title"})
        with self.assertRaisesRegex(RuntimeError, "preset 4 is empty"):
            twitch.apply_preset({"preset": "4"})
        twitch.tags({"mode": "add", "tags": "Cozy, english"})
        self.assertEqual(Fake.state["channels"][ME["id"]]["tags"], ["Speedrun", "English", "Cozy"])
        twitch.tags({"mode": "remove", "tags": "speedrun"})
        self.assertEqual(Fake.state["channels"][ME["id"]]["tags"], ["English", "Cozy"])
        twitch.tags({"mode": "set", "tags": ""})
        self.assertEqual(Fake.state["channels"][ME["id"]]["tags"], [])
        with self.assertRaisesRegex(RuntimeError, "isn't a valid tag"):
            twitch.tags({"tags": "two words"})
        with self.assertRaisesRegex(RuntimeError, "up to 10 tags"):
            twitch.tags({"tags": ",".join(f"t{i}" for i in range(11))})

    def test_labels_and_branded(self):
        twitch.label({"label": "gambling"})
        self.assertEqual(Fake.state["channels"][ME["id"]]["content_classification_labels"], ["Gambling"])
        self.assertEqual(self.last("PATCH", "/channels")[3],
                         {"content_classification_labels": [{"id": "Gambling", "is_enabled": True}]})
        twitch.label({"label": "gambling"})
        self.assertEqual(Fake.state["channels"][ME["id"]]["content_classification_labels"], [])
        twitch.label({"label": "profanity", "state": "on"})
        twitch.branded({})
        self.assertTrue(Fake.state["channels"][ME["id"]]["is_branded_content"])
        twitch.branded({"state": "off"})
        self.assertFalse(Fake.state["channels"][ME["id"]]["is_branded_content"])
        with self.assertRaisesRegex(RuntimeError, "pick a content label"):
            twitch.label({"label": "mature"})
        # The manifest's default is what a key saved without touching the picker sends.
        default = next(f for a in manifest()["actions"] if a["id"] == "label" for f in a["fields"]
                       if f["id"] == "label")["default"]
        twitch.label({"label": default, "state": "on"})
        self.assertIn("DebatedSocialIssuesAndPolitics", Fake.state["channels"][ME["id"]]["content_classification_labels"])

    def test_chat_modes(self):
        settings = lambda: Fake.state["chat_settings"][ME["id"]]  # noqa: E731
        twitch.chat_mode({"mode": "emote"})
        self.assertTrue(settings()["emote_mode"])
        twitch.chat_mode({"mode": "emote"})
        self.assertFalse(settings()["emote_mode"])
        twitch.chat_mode({"mode": "slow", "state": "on", "duration": "30"})
        self.assertEqual((settings()["slow_mode"], settings()["slow_mode_wait_time"]), (True, 30))
        twitch.chat_mode({"mode": "followers", "state": "on", "duration": 10})
        self.assertEqual((settings()["follower_mode"], settings()["follower_mode_duration"]), (True, 10))
        twitch.chat_mode({"mode": "delay", "state": "on", "duration": 4})
        self.assertEqual((settings()["non_moderator_chat_delay"], settings()["non_moderator_chat_delay_duration"]), (True, 4))
        twitch.chat_mode({"mode": "unique", "state": "on"})
        self.assertTrue(settings()["unique_chat_mode"])
        patch = self.last("PATCH", "/chat/settings")
        self.assertEqual(patch[2], {"broadcaster_id": ME["id"], "moderator_id": ME["id"]})
        self.assertEqual(patch[3], {"unique_chat_mode": True})
        twitch.chat_mode({"mode": "emote", "state": "on", "channel": "otherstreamer"})
        self.assertTrue(Fake.state["chat_settings"][OTHER["id"]]["emote_mode"])
        with self.assertRaisesRegex(RuntimeError, "must be 3 to 120"):
            twitch.chat_mode({"mode": "slow", "state": "on", "duration": 500})
        with self.assertRaisesRegex(RuntimeError, "2, 4 or 6"):
            twitch.chat_mode({"mode": "delay", "state": "on", "duration": 3})
        with self.assertRaisesRegex(RuntimeError, "the chat mode is"):
            twitch.chat_mode({"mode": "loud"})

    def test_shield(self):
        twitch.shield({})
        self.assertTrue(Fake.state["shield"])
        twitch.shield({})
        self.assertFalse(Fake.state["shield"])
        twitch.shield({"state": "on"})
        self.assertEqual(self.last("PUT", "/moderation/shield_mode")[3], {"is_active": True})

    def test_moderation(self):
        twitch.ban({"user": "@SomeViewer", "duration": "600", "reason": "Spam"})
        twitch.ban({"user": "someviewer", "duration": "0"})  # "Ban (no end)", picked
        twitch.ban({"user": "someviewer"})  # no duration saved: the 10 minute default, not a ban
        twitch.ban({"user": "someviewer", "duration": ""})
        twitch.unban({"user": "someviewer"})
        twitch.warn({"user": "someviewer", "reason": "Be nice"})
        twitch.clear_chat({})
        twitch.raid({"channel": "otherstreamer"})
        twitch.cancel_raid({})
        twitch.shoutout({"channel": "otherstreamer"})
        twitch.automod({"level": "3"})
        mod = {"broadcaster_id": ME["id"], "moderator_id": ME["id"]}
        self.assertEqual(Fake.state["moderation"], [
            ("POST", "/moderation/bans", mod, {"data": {"user_id": VIEWER["id"], "duration": 600, "reason": "Spam"}}),
            ("POST", "/moderation/bans", mod, {"data": {"user_id": VIEWER["id"]}}),
            ("POST", "/moderation/bans", mod, {"data": {"user_id": VIEWER["id"], "duration": 600}}),
            ("POST", "/moderation/bans", mod, {"data": {"user_id": VIEWER["id"], "duration": 600}}),
            ("DELETE", "/moderation/bans", dict(mod, user_id=VIEWER["id"]), None),
            ("POST", "/moderation/warnings", mod, {"data": {"user_id": VIEWER["id"], "reason": "Be nice"}}),
            ("DELETE", "/moderation/chat", mod, None),
            ("POST", "/raids", {"from_broadcaster_id": ME["id"], "to_broadcaster_id": OTHER["id"]}, None),
            ("DELETE", "/raids", {"broadcaster_id": ME["id"]}, None),
            ("POST", "/chat/shoutouts", {"from_broadcaster_id": ME["id"], "to_broadcaster_id": OTHER["id"],
                                         "moderator_id": ME["id"]}, None),
            ("PUT", "/moderation/automod/settings", mod, {"overall_level": 3}),
        ])
        with self.assertRaisesRegex(RuntimeError, "no user called nobodyhere"):
            twitch.ban({"user": "nobodyhere"})
        # New accounts need 4 characters or more, but shorter old names are Twitch's to judge.
        twitch.ban({"user": "@ZX", "duration": "60"})
        self.assertEqual(Fake.state["moderation"][-1][3], {"data": {"user_id": OLD["id"], "duration": 60}})
        calls = len(self.helix_calls())
        for bad in ("some viewer", "x" * 26, "some-viewer", "émile", "a.b"):
            with self.assertRaisesRegex(RuntimeError, "can't be a Twitch user name: those are up to 25 letters, digits or _"):
                twitch.ban({"user": bad})
        with self.assertRaisesRegex(RuntimeError, "'bad name' can't be a Twitch channel name"):
            twitch.raid({"channel": "@bad name"})
        self.assertEqual(len(self.helix_calls()), calls)  # none sent
        with self.assertRaisesRegex(RuntimeError, "enter the reason"):
            twitch.warn({"user": "someviewer"})
        with self.assertRaisesRegex(RuntimeError, "must be 0 to 4"):
            twitch.automod({"level": 7})

    def test_vips_and_mods(self):
        twitch.vip({"user": "someviewer"})
        self.assertEqual(Fake.state["roles"]["/channels/vips"], {VIEWER["id"]})
        twitch.vip({"user": "someviewer"})
        self.assertEqual(Fake.state["roles"]["/channels/vips"], set())
        twitch.mod({"user": "someviewer", "change": "add"})
        twitch.mod({"user": "someviewer", "change": "add"})
        self.assertEqual(Fake.state["roles"]["/moderation/moderators"], {VIEWER["id"]})
        self.assertEqual(self.last("POST", "/moderation/moderators")[2], {"broadcaster_id": ME["id"], "user_id": VIEWER["id"]})
        twitch.mod({"user": "someviewer", "change": "remove"})
        self.assertEqual(Fake.state["roles"]["/moderation/moderators"], set())

    def test_channel_points(self):
        twitch.reward({"reward": "hydrate!"})
        self.assertTrue(Fake.state["rewards"][0]["is_paused"])
        twitch.reward({"reward": "Hydrate!", "change": "unpause"})
        self.assertFalse(Fake.state["rewards"][0]["is_paused"])
        twitch.reward({"reward": "Hydrate!", "change": "toggle-enable"})
        self.assertFalse(Fake.state["rewards"][0]["is_enabled"])
        twitch.reward({"reward": "Hydrate!", "change": "cost", "cost": "750"})
        self.assertEqual(Fake.state["rewards"][0]["cost"], 750)
        self.assertEqual(self.last("PATCH", "/channel_points/custom_rewards")[2], {"broadcaster_id": ME["id"], "id": "r1"})
        with self.assertRaisesRegex(RuntimeError, "only lets the app that made a reward"):
            twitch.reward({"reward": "Song request"})
        with self.assertRaisesRegex(RuntimeError, "no channel points reward called 'Nope'"):
            twitch.reward({"reward": "Nope"})
        twitch.reward_create({"title": "Stretch", "cost": "200", "prompt": "Stand up", "input": True})
        self.assertEqual(self.last("POST", "/channel_points/custom_rewards")[3],
                         {"title": "Stretch", "cost": 200, "prompt": "Stand up", "is_user_input_required": True})
        twitch.redemption({"reward": "Hydrate!"})
        twitch.redemption({"reward": "Hydrate!", "outcome": "cancel"})
        self.assertEqual([r["status"] for r in Fake.state["redemptions"]["r1"]], ["FULFILLED", "CANCELED"])
        get = [r for r in Fake.state["requests"] if r[1] == "/channel_points/custom_rewards/redemptions" and r[0] == "GET"][0]
        self.assertEqual(get[2], {"broadcaster_id": ME["id"], "reward_id": "r1", "status": "UNFULFILLED",
                                  "sort": "OLDEST", "first": "1"})
        self.assertEqual(twitch.redemption({"reward": "Hydrate!"}), "no redemptions waiting")

    def test_polls(self):
        twitch.poll_start({"title": "Which game next?", "choices": "Mario, Zelda, Metroid", "duration": "120"})
        self.assertEqual(self.last("POST", "/polls")[3], {
            "broadcaster_id": ME["id"], "title": "Which game next?", "duration": 120,
            "choices": [{"title": "Mario"}, {"title": "Zelda"}, {"title": "Metroid"}]})
        twitch.poll_end({})
        self.assertEqual(self.last("PATCH", "/polls")[3], {"broadcaster_id": ME["id"], "id": "p1", "status": "TERMINATED"})
        self.assertEqual(twitch.poll_end({}), "no poll is running")
        twitch.poll_start({"title": "Again?", "choices": "Yes, No"})
        twitch.poll_end({"how": "archive"})
        self.assertEqual(self.last("PATCH", "/polls")[3]["status"], "ARCHIVED")
        with self.assertRaisesRegex(RuntimeError, "2 to 5 choices"):
            twitch.poll_start({"title": "Q", "choices": "Only one"})
        with self.assertRaisesRegex(RuntimeError, "must be 15 to 1800"):
            twitch.poll_start({"title": "Q", "choices": "a, b", "duration": 5})

    def test_predictions(self):
        twitch.prediction_start({"title": "Will we win?", "outcomes": "Yes, No", "window": "60"})
        self.assertEqual(self.last("POST", "/predictions")[3], {
            "broadcaster_id": ME["id"], "title": "Will we win?", "prediction_window": 60,
            "outcomes": [{"title": "Yes"}, {"title": "No"}]})
        twitch.prediction_lock({})
        self.assertEqual(Fake.state["predictions"][-1]["status"], "LOCKED")
        self.assertEqual(twitch.prediction_lock({}), "the prediction is already locked")
        with self.assertRaisesRegex(RuntimeError, "has no outcome 'Maybe'; it has Yes, No"):
            twitch.prediction_resolve({"outcome": "Maybe"})
        twitch.prediction_resolve({"outcome": "no"})
        self.assertEqual(self.last("PATCH", "/predictions")[3], {
            "broadcaster_id": ME["id"], "id": "x1", "status": "RESOLVED", "winning_outcome_id": "o1"})
        with self.assertRaisesRegex(RuntimeError, "no prediction is open"):
            twitch.prediction_cancel({})
        twitch.prediction_start({"title": "Again?", "outcomes": "A, B, C"})
        twitch.prediction_resolve({"outcome": "2"})
        self.assertEqual(Fake.state["predictions"][-1]["winning_outcome_id"], "o1")
        twitch.prediction_start({"title": "Once more?", "outcomes": "A, B"})
        twitch.prediction_cancel({})
        self.assertEqual(Fake.state["predictions"][-1]["status"], "CANCELED")


class ValueTest(Base):
    def setUp(self):
        super().setUp()
        self.signed_in()

    def test_channel_values(self):
        twitch.plugin.watching = {
            "live:me", "viewers:me", "uptime:perritodev", "live:otherstreamer", "viewers:otherstreamer",
            "uptime:otherstreamer", "title:me", "category:me", "category:otherstreamer", "tags:me", "followers:me",
            "followers:otherstreamer", "chat_mode:emote", "chat_mode:slow@otherstreamer", "chat_mode:delay",
            "live:nobodyhere", "chat_mode:loud", "subscribers:me", "chatters:", "last_follower:me",
        }
        Fake.state["chat_settings"][ME["id"]] = {"broadcaster_id": ME["id"], "emote_mode": True, "slow_mode": False,
                                                 "follower_mode": False, "subscriber_mode": False, "unique_chat_mode": False,
                                                 "non_moderator_chat_delay": True}
        with mock.patch.object(twitch.time, "time", return_value=NOW):
            twitch.poll_live()
        twitch.poll_info()
        twitch.poll_counts()
        v = self.values()
        self.assertEqual(v, {
            "live:me": "on", "viewers:me": "78", "uptime:perritodev": "2h 05m",
            "live:otherstreamer": "off", "viewers:otherstreamer": "0", "uptime:otherstreamer": "offline",
            "title:me": "Speedrunning", "category:me": "Just Chatting", "category:otherstreamer": "–",
            "tags:me": "English, Speedrun", "followers:me": "1234", "followers:otherstreamer": "5",
            "chat_mode:emote": "on", "chat_mode:slow@otherstreamer": "off", "chat_mode:delay": "on",
            "live:nobodyhere": "–", "chat_mode:loud": "–", "subscribers:me": "321", "chatters:": "42",
            "last_follower:me": "NewestFan",  # from the followers list, while EventSub isn't connected
        })
        self.assertEqual(len(self.helix_calls("GET", "/streams")), 1)
        self.assertEqual(sorted(self.helix_calls("GET", "/streams")[0][2]["user_id"]), sorted([ME["id"], OTHER["id"]]))
        self.assertEqual(len(self.helix_calls("GET", "/channels")), 1)
        self.assertEqual(len(self.helix_calls("GET", "/users")), 1)
        self.assertTrue(all(c[4]["Client-Id"] == CLIENT_ID for c in self.helix_calls()))
        # Get Chatters: /chat/chatters, with you as the moderator; only the total is read.
        self.assertEqual(self.last("GET", "/chat/chatters")[2],
                         {"broadcaster_id": ME["id"], "moderator_id": ME["id"], "first": "1"})
        # Only changes are published again.
        self.sent.clear()
        twitch.poll_info()
        self.assertEqual(self.values(), {})

    def test_chatters_where_you_dont_moderate(self):
        twitch.plugin.watching = {"chatters:otherstreamer", "chatters:me"}
        twitch.poll_counts()
        self.assertEqual(self.values(), {"chatters:otherstreamer": "–", "chatters:me": "42"})
        self.assertTrue(any(m.startswith("chatters of otherstreamer: Twitch said 403: The user in moderator_id")
                            for m in self.logs()))

    def test_a_bad_channel_name_spoils_only_its_own_values(self):
        # Twitch refuses a whole GET /users batch for one name no account can have, so
        # those are left out: the other channels' values still work.
        twitch.plugin.watching = {"live:me", "live:otherstreamer", "live:bad name", "viewers:bad name",
                                  "title:otherstreamer", "title:a.b", "chat_mode:slow@x-y"}
        twitch.safe_poll()
        v = self.values()
        self.assertEqual((v["live:me"], v["live:otherstreamer"], v["title:otherstreamer"]), ("on", "off", "Chill"))
        self.assertEqual((v["live:bad name"], v["viewers:bad name"], v["title:a.b"], v["chat_mode:slow@x-y"]),
                         ("–", "–", "–", "–"))
        for c in self.helix_calls("GET", "/users"):
            self.assertEqual(c[2]["login"], ["otherstreamer"])
        problems = [m for m in self.logs() if "can't be a Twitch channel name" in m]
        self.assertEqual(sorted(problems), [
            "name a.b: 'a.b' can't be a Twitch channel name: those are up to 25 letters, digits or _",
            "name bad name: 'bad name' can't be a Twitch channel name: those are up to 25 letters, digits or _",
            "name x-y: 'x-y' can't be a Twitch channel name: those are up to 25 letters, digits or _"])
        self.assertFalse(any("Twitch has no channel" in m for m in self.logs()))

    def test_a_name_twitch_refuses_spoils_only_its_own_values(self):
        # A name that passes the plugin's check but that Twitch refuses still makes it
        # answer 400 for the whole GET /users batch. The names are then asked about one at
        # a time, and the refused one is remembered, so it costs no more requests.
        Fake.state["refused_logins"] = {"ab"}
        twitch.plugin.watching = {"live:me", "live:otherstreamer", "live:ab", "title:otherstreamer", "title:AB"}
        twitch.safe_poll()
        v = self.values()
        self.assertEqual((v["live:me"], v["live:otherstreamer"], v["title:otherstreamer"]), ("on", "off", "Chill"))
        self.assertEqual((v["live:ab"], v["title:AB"]), ("–", "–"))
        self.assertEqual([sorted(c[2]["login"]) for c in self.helix_calls("GET", "/users")],
                         [["ab", "otherstreamer"], ["ab"], ["otherstreamer"]])
        self.assertEqual([m for m in self.logs() if m.startswith("name ab")], [
            "name ab: 'ab' isn't a name Twitch accepts (Twitch said 400: Invalid login names, emails or IDs in request)"])
        self.assertFalse(any("Twitch has no channel" in m for m in self.logs()))
        twitch.reschedule()
        twitch.safe_poll()
        self.assertEqual(len(self.helix_calls("GET", "/users")), 3)  # not asked about again
        with self.assertRaisesRegex(RuntimeError, "^Twitch doesn't accept 'ab' as a user name$"):
            twitch.ban({"user": "@Ab"})
        self.assertEqual(len(self.helix_calls("GET", "/users")), 3)
        # Other errors aren't taken for a refused name.
        twitch.plugin.watching = {"live:someviewer"}
        Fake.state["throttle"] = True
        twitch.reschedule()
        twitch.poll_live()
        self.assertNotIn("someviewer", twitch._users)
        self.assertEqual(self.values()["live:someviewer"], "–")

    def test_missing_scopes_for_values(self):
        Fake.state["scopes"] -= {"channel:read:subscriptions", "channel:read:ads"}
        self.signed_in(scopes=set(ALL_SCOPES) - {"channel:read:subscriptions", "channel:read:ads"})
        twitch.plugin.settings["group_events"] = False
        twitch.plugin.watching = {"subscribers:me", "next_ad:me"}
        twitch.safe_poll()
        self.assertEqual(self.values(), {"subscribers:me": "–", "next_ad:me": "–"})
        self.assertTrue(any("sign in again with Events and totals turned on" in m for m in self.logs()))
        self.assertFalse(self.helix_calls("GET", "/subscriptions"))

    def test_ad_values(self):
        twitch.plugin.watching = {"next_ad:me", "ad_snoozes:me", "preroll_free:", "ad_live:me"}
        with mock.patch.object(twitch.time, "time", return_value=NOW):
            twitch.poll_ads()
        self.assertEqual(self.values(), {"next_ad:me": "12:34", "ad_snoozes:me": "3", "preroll_free:": "10:00",
                                         "ad_live:me": "off"})
        twitch.publish_ads(now=NOW + 34)  # the countdown runs between requests
        self.assertEqual(self.values()["next_ad:me"], "12:00")
        self.assertEqual(self.values()["preroll_free:"], "9:26")
        Fake.state["ad_schedule"].update(last_ad_at=NOW - 30, next_ad_at=0)
        with mock.patch.object(twitch.time, "time", return_value=NOW):
            twitch.poll_ads()
        self.assertEqual((self.values()["ad_live:me"], self.values()["next_ad:me"]), ("on", "–"))

    def test_reward_values(self):
        twitch.plugin.watching = {"reward_enabled:Hydrate!", "reward_paused:hydrate!", "reward_pending:Hydrate!",
                                  "reward_paused:Song request", "reward_pending:Song request", "reward_enabled:Nope"}
        Fake.state["redemptions"]["r1"] += [{"id": f"x{i}", "status": "UNFULFILLED"} for i in range(60)]
        twitch.poll_rewards()
        self.assertEqual(self.values(), {
            "reward_enabled:Hydrate!": "on", "reward_paused:hydrate!": "off", "reward_pending:Hydrate!": "50+",
            "reward_paused:Song request": "on", "reward_pending:Song request": "–", "reward_enabled:Nope": "–"})
        Fake.state["redemptions"]["r1"] = [{"id": "a", "status": "UNFULFILLED"}]
        twitch.poll_rewards()
        self.assertEqual(self.values()["reward_pending:Hydrate!"], "1")
        self.assertTrue(any("only lets the app that made a reward" in m for m in self.logs()))

    def test_poll_and_prediction_values(self):
        twitch.plugin.watching = {"poll:me", "poll_leader:me", "poll_time:me", "prediction:me",
                                  "prediction_leader:me", "prediction_time:me"}
        Fake.state["polls"].append({
            "id": "p1", "title": "Which game?", "status": "ACTIVE", "duration": 120, "started_at": iso(NOW - 30),
            "ended_at": None, "choices": [{"id": "a", "title": "Mario", "votes": 31}, {"id": "b", "title": "Zelda", "votes": 19}]})
        Fake.state["predictions"].append({
            "id": "x1", "title": "Win?", "status": "ACTIVE", "prediction_window": 60, "created_at": iso(NOW - 10),
            "ended_at": None, "winning_outcome_id": None,
            "outcomes": [{"id": "o0", "title": "Yes", "channel_points": 450}, {"id": "o1", "title": "No", "channel_points": 550}]})
        with mock.patch.object(twitch.time, "time", return_value=NOW):
            twitch.poll_votes()
        self.assertEqual(self.values(), {
            "poll:me": "Which game?", "poll_leader:me": "Mario 62%", "poll_time:me": "1:30",
            "prediction:me": "Win?", "prediction_leader:me": "No 55%", "prediction_time:me": "0:50"})
        twitch.publish_votes(now=NOW + 60)
        self.assertEqual((self.values()["poll_time:me"], self.values()["prediction_time:me"]), ("0:30", "locked"))
        Fake.state["polls"][-1].update(status="COMPLETED", ended_at=iso(NOW + 90))
        Fake.state["predictions"][-1].update(status="RESOLVED", ended_at=iso(NOW + 200), winning_outcome_id="o0")
        with mock.patch.object(twitch.time, "time", return_value=NOW + 300):
            twitch.poll_votes()
        self.assertEqual((self.values()["poll_time:me"], self.values()["prediction_time:me"]), ("ended", "Yes won"))
        twitch.publish_votes(now=NOW + 3600)  # long over: nothing to show
        self.assertEqual(self.values()["poll:me"], "–")
        self.assertEqual(self.values()["prediction_time:me"], "–")

    def test_polling_intervals(self):
        twitch.plugin.watching = {"live:me", "followers:me", "subscribers:me", "poll:me"}
        clock = [1000.0]
        with mock.patch.object(twitch.time, "monotonic", lambda: clock[0]):
            twitch.safe_poll()
            for _ in range(60):  # a minute, one tick a second
                clock[0] += 1
                twitch.safe_poll()
        self.assertEqual(len(self.helix_calls("GET", "/streams")), 7)  # every 10 s
        self.assertEqual(len(self.helix_calls("GET", "/channels/followers")), 3)  # every 30 s
        self.assertEqual(len(self.helix_calls("GET", "/subscriptions")), 2)  # every 60 s
        self.assertEqual(len(self.helix_calls("GET", "/polls")), 7)  # every 10 s
        twitch.plugin.watching = set()
        before = len(self.helix_calls())
        twitch.reschedule()
        twitch.safe_poll()
        self.assertEqual(len(self.helix_calls()), before)

    def test_helpers(self):
        base = 1791741600  # 2026-10-11T18:00:00Z
        self.assertEqual(twitch.uptime("2026-10-11T18:00:00Z", base + 59 * 60), "59m")
        self.assertEqual(twitch.uptime("2026-10-11T18:00:00.123456789Z", base + 3 * 3600 + 7 * 60), "3h 07m")
        self.assertEqual(twitch.uptime("garbage", base), "–")
        self.assertEqual(twitch.clock(3725), "1:02:05")
        self.assertEqual(twitch.epoch("1791741600"), 1791741600)
        self.assertEqual(twitch.epoch("2026-10-11T18:00:00Z"), 1791741600)
        self.assertIsNone(twitch.epoch(0))


class EventSubTest(Base):
    def setUp(self):
        super().setUp()
        self.signed_in()
        self.ws = FakeEventSub()
        self.addCleanup(self.ws.stop)
        for name, value in (("EVENTSUB_URL", self.ws.url), ("WS_POLL", 0.05), ("EVENTSUB_READ_SECONDS", 0.2),
                            ("EVENTSUB_BACKOFF_FIRST", 0.05)):
            patcher = mock.patch.object(twitch, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)

    def subs(self, session=None):
        return [s for s in Fake.state["subs"] if session is None or s["transport"]["session_id"] == session]

    def watch(self, *values):
        twitch.plugin.watching = set(values)
        twitch.watch_changed(sorted(values))

    def test_subscribe_and_notifications(self):
        self.watch("last_follower:me", "session_follows:me", "last_cheer:me", "hype_train:me", "last_raid:me",
                   "last_subscriber:me", "session_subs:me", "last_gift:me", "last_redemption:me", "live:me")
        wait_for(lambda: len(self.subs()) == 12, message="12 subscriptions")
        by_type = {s["type"]: s for s in self.subs("sess-1")}
        self.assertEqual(by_type["channel.follow"]["version"], "2")
        self.assertEqual(by_type["channel.follow"]["condition"],
                         {"broadcaster_user_id": ME["id"], "moderator_user_id": ME["id"]})
        self.assertEqual(by_type["channel.raid"]["condition"], {"to_broadcaster_user_id": ME["id"]})
        self.assertEqual(by_type["channel.hype_train.progress"]["version"], "2")
        self.assertEqual(by_type["channel.follow"]["transport"], {"method": "websocket", "session_id": "sess-1"})
        self.assertEqual(set(by_type), {
            "channel.follow", "channel.subscribe", "channel.subscription.message", "channel.subscription.gift",
            "channel.cheer", "channel.raid", "channel.channel_points_custom_reward_redemption.add",
            "channel.hype_train.begin", "channel.hype_train.progress", "channel.hype_train.end", "stream.online",
            "stream.offline"})
        self.assertEqual(self.ws.current.path, "/ws")
        self.assertEqual(self.ws.unmasked, 0)
        values = lambda: self.values()  # noqa: E731
        self.ws.notify("channel.follow", {"user_id": "9", "user_login": "newfan", "user_name": "NewFan",
                                          "broadcaster_user_id": ME["id"], "followed_at": "2026-10-11T20:00:00Z"},
                       message_id="dup-1")
        wait_for(lambda: values().get("last_follower:me") == "NewFan", message="follower")
        self.ws.notify("channel.follow", {"user_name": "NewFan"}, message_id="dup-1")  # sent twice by Twitch
        self.ws.notify("channel.cheer", {"is_anonymous": True, "user_name": None, "bits": 100})
        wait_for(lambda: values().get("last_cheer:me") == "Anonymous: 100", message="cheer")
        self.assertEqual(values()["session_follows:me"], "1")
        self.ws.notify("channel.subscribe", {"user_name": "Subby", "tier": "1000", "is_gift": False})
        self.ws.notify("channel.subscription.message", {"user_name": "Loyal", "cumulative_months": 12})
        self.ws.notify("channel.subscription.gift", {"user_name": "Generous", "total": 5, "is_anonymous": False})
        self.ws.notify("channel.raid", {"from_broadcaster_user_name": "Raider", "viewers": 42})
        self.ws.notify("channel.channel_points_custom_reward_redemption.add",
                       {"user_name": "Thirsty", "reward": {"id": "r1", "title": "Hydrate!", "cost": 500}})
        self.ws.notify("channel.hype_train.progress", {"level": 2, "total": 1450, "progress": 450, "goal": 1000})
        wait_for(lambda: values().get("hype_train:me") == "L2 45%", message="hype train")
        v = values()
        self.assertEqual((v["last_subscriber:me"], v["session_subs:me"], v["last_gift:me"], v["last_raid:me"],
                          v["last_redemption:me"]), ("Loyal", "2", "Generous: 5", "Raider (42)", "Thirsty: Hydrate!"))
        self.ws.notify("channel.hype_train.end", {"level": 3, "total": 3000})
        wait_for(lambda: values().get("hype_train:me") == "–", message="hype train over")
        # The latest events are remembered across restarts; session counts aren't.
        with open(os.path.join(self.data, "state.json"), encoding="utf-8") as f:
            saved = json.load(f)
        self.assertEqual(saved["last"]["me"]["last_raid"], "Raider (42)")
        # Nothing watched any more: the session closes.
        self.watch()
        wait_for(lambda: self.ws.conns[0].closed_by_client, message="close")

    def test_reconnect_keeps_subscriptions(self):
        self.watch("last_cheer:me")
        wait_for(lambda: len(self.subs()) == 1, message="subscription")
        old = self.ws.current
        old.send(self.ws.message("session_reconnect", {"session": {
            "id": "sess-1", "status": "reconnecting", "keepalive_timeout_seconds": None,
            "reconnect_url": f"ws://127.0.0.1:{self.ws.port}/ws?reconnect=1", "connected_at": "2026-10-11T20:00:00Z"}}))
        wait_for(lambda: len(self.ws.conns) == 2 and old.closed_by_client, message="moved to the new connection")
        self.assertEqual(self.ws.current.path, "/ws?reconnect=1")
        time.sleep(0.3)
        self.assertEqual(len(self.subs()), 1)  # subscriptions carried over: none made again
        self.ws.notify("channel.cheer", {"user_name": "Cheery", "bits": 5})
        wait_for(lambda: self.values().get("last_cheer:me") == "Cheery: 5", message="cheer on the new connection")

    def test_keepalive_timeout_reconnects_and_resubscribes(self):
        self.ws.keepalive = 0.3
        with mock.patch.object(twitch, "KEEPALIVE_GRACE", 0.1):
            self.watch("last_raid:me")
            wait_for(lambda: len(self.subs("sess-1")) == 1, message="first subscription")
            # The fake sends no keepalives: the plugin gives up after 0.4 s and starts over.
            wait_for(lambda: len(self.subs("sess-2")) == 1, timeout=4, message="resubscribed on a new session")
        self.assertTrue(any("no keepalive" in m for m in self.logs()))

    def test_keepalives_keep_it_open(self):
        self.ws.keepalive = 0.3
        with mock.patch.object(twitch, "KEEPALIVE_GRACE", 0.1):
            self.watch("last_raid:me")
            wait_for(lambda: len(self.subs()) == 1, message="subscription")
            for _ in range(6):  # 0.6 s in all, past the 0.4 s limit without them
                self.ws.current.send(self.ws.message("session_keepalive", {}))
                time.sleep(0.1)
        self.assertEqual(len(self.ws.conns), 1)

    def test_pings_and_slow_frames_keep_the_session(self):
        # Twitch pings the connection now and then. The plugin used to answer and then
        # wait on the socket for the next message, which may only come with the next
        # keepalive, and took the read timeout for a lost connection. Here the read
        # timeout (0.15 s) is shorter than the gap between keepalives (0.5 s), as
        # Twitch's 10 s is to its 30 s.
        self.ws.keepalive = 0.6
        with mock.patch.object(twitch, "KEEPALIVE_GRACE", 0.2), mock.patch.object(twitch, "WS_READ_TIMEOUT", 0.15):
            self.watch("last_raid:me")
            wait_for(lambda: len(self.subs()) == 1, message="subscription")
            for i in range(4):
                self.ws.current.ping(b"p%d" % i)
                time.sleep(0.3)
                # One keepalive arrives in two parts, 0.3 s apart (longer than the read timeout).
                self.ws.current.send(self.ws.message("session_keepalive", {}), pause_after=3 if i == 2 else None)
                time.sleep(0.2)
            self.ws.notify("channel.raid", {"from_broadcaster_user_name": "Raider", "viewers": 7})
            wait_for(lambda: self.values().get("last_raid:me") == "Raider (7)", message="raid")
        self.assertEqual(len(self.ws.conns), 1)
        self.assertEqual(self.ws.pongs, [b"p0", b"p1", b"p2", b"p3"])  # each answered with its own payload
        self.assertFalse([m for m in self.logs() if "EventSub connection" in m])
        self.assertEqual(len(self.subs()), 1)  # never subscribed again

    def test_pings_alone_are_not_keepalives(self):
        # A ping shows the connection is up, not that the EventSub session is: without
        # messages for the keepalive time from the welcome, it starts over.
        self.ws.keepalive = 0.3
        with mock.patch.object(twitch, "KEEPALIVE_GRACE", 0.1):
            self.watch("last_raid:me")
            wait_for(lambda: len(self.subs("sess-1")) == 1, message="first subscription")
            first = self.ws.current
            deadline = time.monotonic() + 4
            while not self.subs("sess-2") and time.monotonic() < deadline:
                try:
                    first.ping(b"x")
                except OSError:
                    pass
                time.sleep(0.05)
        self.assertEqual(len(self.subs("sess-2")), 1)
        self.assertTrue(any("no keepalive from Twitch in 0.4 s" in m for m in self.logs()))

    def test_failed_subscriptions_are_tried_again(self):
        Fake.state["fail_subs"] = {"channel.raid": 2}  # Twitch has two bad moments
        with mock.patch.object(twitch, "EVENTSUB_RETRY_FIRST", 0.1):
            self.watch("last_raid:me", "last_cheer:me")
            wait_for(lambda: {s["type"] for s in self.subs()} == {"channel.raid", "channel.cheer"}, timeout=4,
                     message="both subscriptions, with no key changed")
        posts = [r for r in Fake.state["requests"] if r[:2] == ("POST", "/eventsub/subscriptions")
                 and r[3]["type"] == "channel.raid"]
        self.assertEqual(len(posts), 3)
        self.assertIn("event channel.raid: Twitch said 503: Internal Server Error", self.logs())
        self.assertIn("event channel.raid: working again", self.logs())
        self.assertEqual(len(self.ws.conns), 1)

    def test_subscription_retries_wait_longer_each_time(self):
        events = twitch.EventSub()  # not started: sync() by hand
        events.session = Fake.state["session"] = "sess-by-hand"
        Fake.state["fail_subs"] = {"channel.cheer": 10}
        twitch.plugin.watching = {"last_cheer:me"}
        wanted = twitch.wanted_subscriptions()
        key = next(iter(wanted))
        clock = [100.0]
        waits = []
        with mock.patch.object(twitch, "EVENTSUB_RETRY_FIRST", 15.0), mock.patch.object(twitch, "EVENTSUB_RETRY_MAX", 60.0), \
                mock.patch.object(twitch.time, "monotonic", lambda: clock[0]):
            for _ in range(5):
                events.sync(wanted)
                at, wait = events.failed[key]
                waits.append(wait)
                self.assertFalse(events.retry_due(wanted))
                events.sync(wanted)  # not due yet: not sent
                clock[0] = at
                self.assertTrue(events.retry_due(wanted))
            clock[0] -= 1
            self.assertEqual(events.next_retry(wanted), clock[0] + 1)  # every one refused: wait for it
            self.assertIsNone(events.next_retry(dict(wanted, other=None)))  # one not refused: no wait
            # A session with nothing subscribed is lost (Twitch closes those after 10 s):
            # the retry stays where it was.
            events.drop()
            self.assertFalse(events.retry_due(wanted))
            self.assertEqual(events.next_retry(wanted), clock[0] + 1)
            # A session with a working subscription is lost: due at once on the next one,
            # and the wait stays where it got to.
            events.subscribed = {("channel.raid", "1", "{}"): "sub-1"}
            events.drop()
            self.assertTrue(events.retry_due(wanted))
            self.assertIsNone(events.next_retry(wanted))
            self.assertEqual(events.failed[key][1], 60.0)
        self.assertEqual(waits, [15.0, 30.0, 60.0, 60.0, 60.0])
        posts = [r for r in Fake.state["requests"] if r[:2] == ("POST", "/eventsub/subscriptions")]
        self.assertEqual(len(posts), 5)
        events.sync({})  # no key wants it any more: forgotten
        self.assertEqual(events.failed, {})

    def test_failed_subscriptions_are_tried_again_on_a_new_connection(self):
        self.ws.keepalive = 0.3
        Fake.state["fail_subs"] = {"channel.raid": 1}
        with mock.patch.object(twitch, "KEEPALIVE_GRACE", 0.1), mock.patch.object(twitch, "EVENTSUB_RETRY_FIRST", 60):
            self.watch("last_raid:me", "last_cheer:me")
            wait_for(lambda: len(self.subs("sess-1")) == 1, message="the cheer subscription")
            # No keepalives: a new connection, where the raid is tried again at once
            # rather than after the minute its backoff would wait.
            wait_for(lambda: {s["type"] for s in self.subs("sess-2")} == {"channel.raid", "channel.cheer"},
                     timeout=4, message="both on the new session")

    def raid_posts(self):
        return [p for p in Fake.state["sub_posts"] if p[0] == "channel.raid"]

    def test_refused_subscriptions_wait_when_twitch_closes_unused_sessions(self):
        # Twitch closes a session with no subscription 10 s after its welcome (4003). When
        # it refuses every subscription, that mustn't make the retry due at once: the
        # plugin used to reconnect and POST again every few seconds, for ever.
        self.ws.close_unused = 0.1
        Fake.state["fail_subs"] = {"channel.raid": 100}
        with mock.patch.object(twitch, "EVENTSUB_RETRY_FIRST", 0.4):
            self.watch("last_raid:me")
            wait_for(lambda: len(self.raid_posts()) == 3, timeout=5, message="three tries")
            times = [t for _, _, t in self.raid_posts()]
            self.assertGreaterEqual(times[1] - times[0], 0.35)  # 0.4 s
            self.assertGreaterEqual(times[2] - times[1], 0.75)  # then 0.8 s
            self.assertEqual(len({session for _, session, _ in self.raid_posts()}), 3)  # a new session each time
            self.assertEqual(len(self.ws.conns), 3)  # and none opened in between
        self.assertEqual(self.logs().count("event channel.raid: Twitch said 503: Internal Server Error"), 1)

    def test_a_session_with_every_subscription_refused_is_closed(self):
        Fake.state["fail_subs"] = {"channel.raid": 100}
        with mock.patch.object(twitch, "EVENTSUB_RETRY_FIRST", 60):
            self.watch("last_raid:me")
            # Nothing on it would ever arrive, and Twitch would close it: closed at once.
            wait_for(lambda: self.ws.conns and self.ws.conns[0].closed_by_client, message="closed by the plugin")
            time.sleep(0.3)
            self.assertEqual((len(self.ws.conns), len(self.raid_posts())), (1, 1))  # waiting its minute
            # Keys change: everything is tried again at once.
            self.watch("last_raid:me", "last_cheer:me")
            wait_for(lambda: len(self.subs("sess-2")) == 1, message="the cheer subscription")
            self.assertEqual(len(self.raid_posts()), 2)
            time.sleep(0.3)
            self.assertEqual(len(self.ws.conns), 2)  # this one has a subscription: kept
            self.assertFalse(self.ws.conns[1].closed_by_client)

    def test_revocation_and_failed_subscriptions(self):
        Fake.state["scopes"].discard("bits:read")
        self.signed_in(scopes=set(ALL_SCOPES) - {"bits:read"})
        self.watch("last_cheer:me", "last_raid:me", "last_follower:otherstreamer")
        wait_for(lambda: len(self.subs()) == 2, message="subscriptions")
        types = sorted(s["type"] for s in self.subs())
        self.assertEqual(types, ["channel.follow", "channel.raid"])  # no cheer without bits:read
        sub = next(s for s in self.subs() if s["type"] == "channel.raid")
        self.ws.current.send(self.ws.message("revocation", {"subscription": dict(sub, status="authorization_revoked")},
                                             subscription_type="channel.raid", subscription_version="1"))
        wait_for(lambda: any("authorization_revoked" in m for m in self.logs()), message="revocation logged")

    def test_events_turned_off_means_no_connection(self):
        twitch.plugin.settings["group_events"] = False
        self.watch("last_cheer:me")
        time.sleep(0.3)
        self.assertEqual(self.ws.conns, [])
        twitch.safe_poll()
        self.assertEqual(self.values()["last_cheer:me"], "–")


class ManifestTest(unittest.TestCase):
    def test_actions_and_fields(self):
        m = manifest()
        self.assertEqual({a["id"] for a in m["actions"]}, set(twitch.plugin._actions))
        for action in m["actions"]:
            for field in action.get("fields", []):
                if field["type"] == "select":
                    # The editor shows the first option when there's no default but saves
                    # nothing, so every select says which one it means.
                    self.assertIn(field.get("default"), [o["value"] for o in field["options"]],
                                  f"{action['id']}.{field['id']}")
        ban = next(f for a in m["actions"] if a["id"] == "ban" for f in a["fields"] if f["id"] == "duration")
        self.assertEqual(ban["default"], str(twitch.BAN_DEFAULT))
        forever = next(o for o in ban["options"] if o["value"] == str(twitch.BAN_FOREVER))
        self.assertEqual(forever["label"], "Ban (no end)")

    def test_readme_matches_the_plugin(self):
        with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "README.md"), encoding="utf-8") as f:
            text = f.read()
        # The scopes table lists exactly the scopes the sign-in can ask for, and the count
        # under it is right.
        table = text.split("| Group | Scopes | For |", 1)[1].split("\n\n", 1)[0]
        self.assertEqual(set(re.findall(r"`([a-z_]+(?::[a-z_]+)+)`", table)), ALL_SCOPES)
        flat = " ".join(text.split())
        self.assertIn(f"asks for these {len(ALL_SCOPES)} scopes; with none ticked, only for the "
                      f"{len(twitch.BASE_SCOPES)} in the Always rows", flat)
        # Any version it names is the manifest's (it used to lag behind).
        for version in re.findall(r"\bversion (\d+\.\d+\.\d+)", flat):
            self.assertEqual(version, manifest()["version"])
        # Every table row is a row: text right after a row ends the table, and rows right
        # after text are part of that paragraph.
        lines = text.split("\n")
        for i, line in enumerate(lines[1:], 1):
            before = lines[i - 1]
            if line and before and line.startswith("|") != before.startswith("|"):
                self.fail(f"README.md line {i + 1} breaks a table: {line[:60]!r}")


if __name__ == "__main__":
    unittest.main()
