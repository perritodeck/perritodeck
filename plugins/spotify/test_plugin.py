import base64
import hashlib
import http.server
import json
import os
import shutil
import stat
import sys
import tempfile
import threading
import time
import unittest
import urllib.parse
import urllib.request
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as spotify  # noqa: E402

CLIENT_ID = "3f8a1c0e9b7d4e2a8c6f5b1d0e9a7c3b"
TRACK = "spotify:track:4uLU6hMCjMI75M1A2tKUQC"


def s256(verifier):
    return base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).decode().rstrip("=")


def player_state(**changes):
    state = {
        "device": {"id": "dev1", "name": "Desk speaker", "type": "Speaker", "volume_percent": 40,
                   "supports_volume": True, "is_active": True},
        "shuffle_state": False,
        "repeat_state": "off",
        "is_playing": True,
        "progress_ms": 83000,
        "currently_playing_type": "track",
        "item": {"type": "track", "uri": TRACK, "name": "Never Gonna Give You Up", "duration_ms": 225000,
                 "artists": [{"name": "Rick Astley"}, {"name": "Guest"}], "album": {"name": "Whenever You Need Somebody"}},
    }
    state.update(changes)
    return state


class Fake(http.server.BaseHTTPRequestHandler):
    """Spotify's accounts service (/api/token) and Web API (/v1/…) in one server."""
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

    def error(self, code, message, reason=None):
        err = {"status": code, "message": message}
        if reason:
            err["reason"] = reason
        self.send_json(code, {"error": err})

    def handle_any(self):
        st = Fake.state
        length = self.headers.get("Content-Length")
        raw = self.rfile.read(int(length)) if length else b""
        url = urllib.parse.urlparse(self.path)
        query = dict(urllib.parse.parse_qsl(url.query))
        st["calls"].append((self.command, url.path, query, raw, self.headers.get("Authorization"), length))
        if url.path == "/api/token":
            return self.token(dict(urllib.parse.parse_qsl(raw.decode())))
        if not url.path.startswith("/v1/"):
            return self.send_json(404, {"error": "not found"})
        if self.headers.get("Authorization") != f"Bearer {st['valid_token']}":
            return self.error(401, "The access token expired")
        if st.get("retry_after"):
            return self.send_json(429, None, {"Retry-After": str(st["retry_after"])})
        if self.command in ("PUT", "POST") and length is None:
            return self.send_json(411, {"error": "length required"})
        body = json.loads(raw) if raw else None
        path, cmd = url.path[3:], self.command
        player = st["player"]
        if path == "/me/player" and cmd == "GET":
            return self.send_json(200, player) if player else self.send_json(204)
        if path == "/me/player/devices":
            return self.send_json(200, {"devices": st["devices"]})
        if path == "/me/library/contains":
            return self.send_json(200, [u in st["library"] for u in query["uris"].split(",")])
        if path == "/me/library":
            for u in query["uris"].split(","):
                (st["library"].add if cmd == "PUT" else st["library"].discard)(u)
            return self.send_json(200)
        if path.startswith("/playlists/") and path.endswith("/items") and cmd == "POST":
            st["playlists"].setdefault(path.split("/")[2], []).extend(body["uris"])
            return self.send_json(201, {"snapshot_id": "abc"})
        # Player commands below need Premium and an active device.
        if not st.get("premium", True):
            return self.error(403, "Player command failed: Premium required", "PREMIUM_REQUIRED")
        if not player and path != "/me/player":
            return self.error(404, "Player command failed: No active device found", "NO_ACTIVE_DEVICE")
        if path == "/me/player/pause":
            player["is_playing"] = False
        elif path == "/me/player/play":
            player["is_playing"] = True
            st["played"] = (body, query)
        elif path == "/me/player/shuffle":
            player["shuffle_state"] = query["state"] == "true"
        elif path == "/me/player/repeat":
            player["repeat_state"] = query["state"]
        elif path == "/me/player/volume":
            player["device"]["volume_percent"] = int(query["volume_percent"])
        elif path in ("/me/player/next", "/me/player/previous"):
            pass
        elif path == "/me/player" and cmd == "PUT":
            st["transferred"] = body
        else:
            return self.send_json(404, {"error": {"status": 404, "message": "Service not found"}})
        self.send_json(204)

    def token(self, form):
        st = Fake.state
        if form.get("client_id") != CLIENT_ID:
            return self.send_json(400, {"error": "invalid_client", "error_description": "Invalid client"})
        if form.get("grant_type") == "authorization_code":
            ok = (form.get("code") == st.get("code") and form.get("redirect_uri") == st.get("redirect_uri")
                  and s256(form.get("code_verifier", "")) == st.get("challenge"))
            if not ok:
                return self.send_json(400, {"error": "invalid_grant", "error_description": "code_verifier was incorrect"})
            st["valid_token"], st["refresh_token"] = "access-1", "refresh-1"
            return self.send_json(200, {"access_token": "access-1", "token_type": "Bearer", "expires_in": 3600,
                                        "refresh_token": "refresh-1", "scope": "user-read-playback-state"})
        if form.get("grant_type") == "refresh_token":
            if form.get("refresh_token") != st.get("refresh_token"):
                return self.send_json(400, {"error": "invalid_grant", "error_description": "Refresh token revoked"})
            st["refreshes"] += 1
            st["valid_token"] = f"access-r{st['refreshes']}"
            answer = {"access_token": st["valid_token"], "token_type": "Bearer", "expires_in": 3600, "scope": "x"}
            if st.get("rotate"):
                st["refresh_token"] = answer["refresh_token"] = f"refresh-r{st['refreshes']}"
            return self.send_json(200, answer)
        self.send_json(400, {"error": "unsupported_grant_type"})

    do_GET = do_POST = do_PUT = do_DELETE = handle_any

    def log_message(self, *args):
        pass


class SpotifyTest(unittest.TestCase):
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
        Fake.state = {"calls": [], "player": player_state(), "library": set(), "playlists": {}, "refreshes": 0,
                      "valid_token": "access-0", "refresh_token": "refresh-0",
                      "devices": [{"id": "dev1", "name": "Desk speaker"}, {"id": "dev2", "name": "Living Room TV"}]}
        self.data = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.data, True)
        spotify.plugin._data_dir = self.data
        self.sent = []
        spotify.plugin._send = self.sent.append
        spotify.plugin.settings = {"client_id": CLIENT_ID, "port": 0}
        for name, value in (("ACCOUNTS_URL", self.base), ("API_URL", self.base + "/v1"),
                            ("poll_soon", lambda: None)):
            patcher = mock.patch.object(spotify, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        spotify._backoff["until"] = 0.0
        spotify._published.clear()
        spotify._reported.clear()
        spotify._state.update(player=None, liked={}, next=0.0)

    def signed_in(self, expires_in=3600):
        spotify.save_tokens({"client_id": CLIENT_ID, "access_token": "access-0", "refresh_token": "refresh-0",
                             "expires_at": time.time() + expires_in, "scope": ""})

    def values(self):
        return {m["id"]: m["value"] for m in self.sent if m.get("event") == "value"}

    def api_calls(self, method=None):
        return [(c[0], c[1][3:], c[2]) for c in Fake.state["calls"]
                if c[1].startswith("/v1/") and (method is None or c[0] == method)]

    # --- PKCE and sign-in ----------------------------------------------------------

    def test_pkce_rfc7636_test_vector(self):
        # RFC 7636 appendix B.
        self.assertEqual(spotify.challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
                         "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
        verifier = spotify.make_verifier()
        self.assertTrue(43 <= len(verifier) <= 128)
        self.assertRegex(verifier, r"^[A-Za-z0-9\-._~]+$")
        self.assertNotEqual(verifier, spotify.make_verifier())

    def browser(self, deny=False, wrong_state=False):
        """Plays the user's browser: reads the authorize address, 'approves', and
        follows the redirect back to the plugin's one-shot server."""
        opened = []

        def open_url(url):
            parsed = urllib.parse.urlparse(url)
            q = dict(urllib.parse.parse_qsl(parsed.query))
            opened.append((parsed, q))
            st = Fake.state
            st["challenge"], st["redirect_uri"], st["code"] = q["code_challenge"], q["redirect_uri"], "the-code"
            back = {"error": "access_denied"} if deny else {"code": "the-code"}
            back["state"] = "forged" if wrong_state else q["state"]

            def follow():
                try:
                    with urllib.request.urlopen(q["redirect_uri"] + "?" + urllib.parse.urlencode(back), timeout=5) as r:
                        opened.append(r.read().decode())
                except urllib.error.HTTPError as e:
                    opened.append(e.read().decode())

            threading.Thread(target=follow, daemon=True).start()
            return True

        return opened, open_url

    def test_sign_in(self):
        opened, open_url = self.browser()
        with mock.patch.object(spotify.webbrowser, "open", open_url):
            self.assertEqual(spotify.sign_in({}), "finish signing in in your web browser")
            attempt = spotify._sign_in["current"]
            self.assertTrue(attempt.done.wait(5))
        self.assertIsNone(attempt.error)
        parsed, q = opened[0]
        self.assertEqual(parsed.path, "/authorize")
        self.assertEqual(q["response_type"], "code")
        self.assertEqual(q["client_id"], CLIENT_ID)
        self.assertEqual(q["code_challenge_method"], "S256")
        self.assertEqual(q["code_challenge"], spotify.challenge_for(attempt.verifier))
        self.assertRegex(q["redirect_uri"], r"^http://127\.0\.0\.1:\d+/callback$")
        self.assertIn("user-modify-playback-state", q["scope"].split(" "))
        self.assertIn("user-library-modify", q["scope"].split(" "))
        tokens = spotify.load_tokens()
        self.assertEqual((tokens["access_token"], tokens["refresh_token"], tokens["client_id"]),
                         ("access-1", "refresh-1", CLIENT_ID))
        if os.name == "posix":
            mode = stat.S_IMODE(os.stat(os.path.join(self.data, "tokens.json")).st_mode)
            self.assertEqual(mode, 0o600)
        # Tokens never reach the log.
        for m in self.sent:
            self.assertNotIn("access-1", json.dumps(m))
            self.assertNotIn("refresh-1", json.dumps(m))
        # The server answered once and closed.
        deadline = time.monotonic() + 3
        while attempt.thread.is_alive() and time.monotonic() < deadline:
            time.sleep(0.02)
        self.assertFalse(attempt.thread.is_alive())

    def test_sign_in_with_a_wrong_verifier_fails(self):
        opened, open_url = self.browser()
        with mock.patch.object(spotify.webbrowser, "open", open_url), \
                mock.patch.object(spotify, "challenge_for", lambda v: s256(v + "x")):
            spotify.sign_in({})
            attempt = spotify._sign_in["current"]
            self.assertTrue(attempt.done.wait(5))
        self.assertIn("invalid_grant", attempt.error)
        self.assertIsNone(spotify.load_tokens())

    def test_sign_in_denied_or_forged(self):
        opened, open_url = self.browser(deny=True)
        with mock.patch.object(spotify.webbrowser, "open", open_url):
            spotify.sign_in({})
            attempt = spotify._sign_in["current"]
            self.assertTrue(attempt.done.wait(5))
        self.assertIn("access_denied", attempt.error)
        opened, open_url = self.browser(wrong_state=True)
        with mock.patch.object(spotify.webbrowser, "open", open_url):
            spotify.sign_in({})
            attempt = spotify._sign_in["current"]
            deadline = time.monotonic() + 3
            while len(opened) < 2 and time.monotonic() < deadline:
                time.sleep(0.02)
        self.assertIn("doesn't match", opened[1])
        self.assertFalse(attempt.done.is_set())  # still waiting for the real answer
        attempt.finish("test over")
        self.assertIsNone(spotify.load_tokens())

    def test_sign_in_needs_a_client_id(self):
        spotify.plugin.settings = {}
        with self.assertRaisesRegex(RuntimeError, "client ID"):
            spotify.sign_in({})

    def test_sign_in_port_in_use(self):
        taken = http.server.HTTPServer(("127.0.0.1", 0), http.server.BaseHTTPRequestHandler)
        self.addCleanup(taken.server_close)
        spotify.plugin.settings["port"] = taken.server_port
        with self.assertRaisesRegex(RuntimeError, "another sign-in port"):
            spotify.sign_in({})

    def test_sign_in_again_on_the_same_port(self):
        # Pressed again while the first attempt still waits for the browser: the new
        # attempt needs the same port, the one registered with Spotify.
        with mock.patch.object(spotify.webbrowser, "open", lambda url: True):
            spotify.sign_in({})
        first = spotify._sign_in["current"]
        spotify.plugin.settings["port"] = first.port
        opened, open_url = self.browser()
        with mock.patch.object(spotify.webbrowser, "open", open_url):
            spotify.sign_in({})
            attempt = spotify._sign_in["current"]
            self.assertTrue(attempt.done.wait(5))
        self.assertEqual(first.error, "replaced by a new sign-in")
        self.assertEqual(attempt.port, first.port)
        self.assertIsNone(attempt.error)
        self.assertEqual(spotify.load_tokens()["access_token"], "access-1")

    # --- tokens ----------------------------------------------------------------------

    def test_refresh_on_401(self):
        self.signed_in()
        Fake.state["valid_token"] = "revoked"  # the saved token no longer works
        Fake.state["refresh_token"] = "refresh-0"
        self.assertIsNone(spotify.next_track({}))
        self.assertEqual(Fake.state["refreshes"], 1)
        tokens = spotify.load_tokens()
        self.assertEqual(tokens["access_token"], "access-r1")
        self.assertEqual(tokens["refresh_token"], "refresh-0")  # kept: no new one was sent
        auths = [c[4] for c in Fake.state["calls"] if c[1] == "/v1/me/player/next"]
        self.assertEqual(auths, ["Bearer access-0", "Bearer access-r1"])

    def test_refresh_before_expiry_and_rotation(self):
        self.signed_in(expires_in=10)  # inside the 60 s margin
        Fake.state["rotate"] = True
        self.assertIsNone(spotify.previous_track({}))
        self.assertEqual(spotify.load_tokens()["refresh_token"], "refresh-r1")
        refresh = next(c for c in Fake.state["calls"] if c[1] == "/api/token")
        form = dict(urllib.parse.parse_qsl(refresh[3].decode()))
        self.assertEqual(form, {"grant_type": "refresh_token", "refresh_token": "refresh-0", "client_id": CLIENT_ID})

    def test_revoked_refresh_token(self):
        self.signed_in(expires_in=0)
        Fake.state["refresh_token"] = "something-else"
        with self.assertRaisesRegex(RuntimeError, "Sign in to Spotify"):
            spotify.next_track({})
        self.assertIsNone(spotify.load_tokens())

    def test_not_signed_in(self):
        with self.assertRaisesRegex(RuntimeError, "not signed in"):
            spotify.next_track({})
        # Tokens from another client ID don't count.
        self.signed_in()
        spotify.plugin.settings["client_id"] = "another-app"
        with self.assertRaisesRegex(RuntimeError, "not signed in"):
            spotify.next_track({})

    def test_sign_out(self):
        self.signed_in()
        spotify.sign_out({})
        self.assertIsNone(spotify.load_tokens())
        self.assertEqual(self.values()["track"], "–")

    # --- actions ---------------------------------------------------------------------

    def test_playback_actions(self):
        self.signed_in()
        player = Fake.state["player"]
        spotify.play_pause({})
        self.assertFalse(player["is_playing"])
        spotify.play_pause({})
        self.assertTrue(player["is_playing"])
        spotify.next_track({})
        spotify.previous_track({})
        spotify.shuffle({})
        self.assertTrue(player["shuffle_state"])
        spotify.shuffle({"mode": "on"})
        self.assertTrue(player["shuffle_state"])
        spotify.shuffle({"mode": "off"})
        self.assertFalse(player["shuffle_state"])
        modes = []
        for _ in range(3):
            spotify.repeat({})
            modes.append(player["repeat_state"])
        self.assertEqual(modes, ["context", "track", "off"])
        spotify.repeat({"mode": "track"})
        self.assertEqual(player["repeat_state"], "track")
        methods = {(m, p) for m, p, q in self.api_calls()}
        self.assertTrue({("PUT", "/me/player/pause"), ("PUT", "/me/player/play"), ("POST", "/me/player/next"),
                         ("POST", "/me/player/previous"), ("PUT", "/me/player/shuffle"),
                         ("PUT", "/me/player/repeat")} <= methods)
        # Every PUT/POST carries a length (Spotify answers 411 otherwise).
        self.assertTrue(all(c[5] is not None for c in Fake.state["calls"] if c[0] in ("PUT", "POST")))

    def test_volume(self):
        self.signed_in()
        device = Fake.state["player"]["device"]
        spotify.volume({"value": 73})
        self.assertEqual(device["volume_percent"], 73)
        spotify.volume({"value": "120"})
        self.assertEqual(device["volume_percent"], 100)
        spotify.volume_down({"step": 25})
        self.assertEqual(device["volume_percent"], 75)
        spotify.volume_up({})
        self.assertEqual(device["volume_percent"], 85)
        device["volume_percent"] = 95
        spotify.volume_up({"step": "10"})
        self.assertEqual(device["volume_percent"], 100)
        device["supports_volume"] = False
        with self.assertRaisesRegex(RuntimeError, "can't change the volume of Desk speaker"):
            spotify.volume_up({})

    def test_like(self):
        self.signed_in()
        spotify.like({})
        self.assertIn(TRACK, Fake.state["library"])
        spotify.like({"mode": "on"})
        self.assertIn(TRACK, Fake.state["library"])
        spotify.like({})
        self.assertNotIn(TRACK, Fake.state["library"])
        lib = [(m, q) for m, p, q in self.api_calls() if p == "/me/library"]
        self.assertEqual(lib, [("PUT", {"uris": TRACK}), ("DELETE", {"uris": TRACK})])
        Fake.state["player"] = None
        with self.assertRaisesRegex(RuntimeError, "nothing is playing"):
            spotify.like({})

    def test_play_links(self):
        self.signed_in()
        spotify.play({"uri": "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=abc123"})
        self.assertEqual(Fake.state["played"], ({"context_uri": "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M"}, {}))
        spotify.play({"uri": "https://open.spotify.com/intl-de/album/1DFixLWuPkv3KT3TnV35m3"})
        self.assertEqual(Fake.state["played"][0], {"context_uri": "spotify:album:1DFixLWuPkv3KT3TnV35m3"})
        spotify.play({"uri": TRACK, "device": "living room"})
        self.assertEqual(Fake.state["played"], ({"uris": [TRACK]}, {"device_id": "dev2"}))
        spotify.play({"uri": "spotify:user:someone:playlist:37i9dQZF1DXcBWIGoYBM5M"})
        self.assertEqual(Fake.state["played"][0], {"context_uri": "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M"})
        with self.assertRaisesRegex(RuntimeError, "isn't a Spotify link"):
            spotify.play({"uri": "https://example.com/playlist"})
        with self.assertRaisesRegex(RuntimeError, "no Spotify device called 'Garage'; devices: Desk speaker, Living Room TV"):
            spotify.play({"uri": TRACK, "device": "Garage"})

    def test_add_to_playlist(self):
        self.signed_in()
        spotify.add_to_playlist({"playlist": "https://open.spotify.com/playlist/3cEYpjA9oz9GiPac4AsH4n"})
        spotify.add_to_playlist({"playlist": "3cEYpjA9oz9GiPac4AsH4n"})
        self.assertEqual(Fake.state["playlists"], {"3cEYpjA9oz9GiPac4AsH4n": [TRACK, TRACK]})
        with self.assertRaisesRegex(RuntimeError, "not a playlist"):
            spotify.add_to_playlist({"playlist": "spotify:album:1DFixLWuPkv3KT3TnV35m3"})

    def test_transfer(self):
        self.signed_in()
        spotify.transfer({"device": "Living Room TV"})
        self.assertEqual(Fake.state["transferred"], {"device_ids": ["dev2"], "play": True})
        spotify.transfer({"device": "desk", "play": False})
        self.assertEqual(Fake.state["transferred"], {"device_ids": ["dev1"]})
        with self.assertRaisesRegex(RuntimeError, "device's name"):
            spotify.transfer({})

    def test_errors(self):
        self.signed_in()
        Fake.state["premium"] = False
        with self.assertRaisesRegex(RuntimeError, "needs Spotify Premium"):
            spotify.next_track({})
        Fake.state["premium"] = True
        Fake.state["player"] = None
        with self.assertRaisesRegex(RuntimeError, "no Spotify device is active"):
            spotify.next_track({})
        with self.assertRaisesRegex(RuntimeError, "no Spotify device is active"):
            spotify.play_pause({})

    def test_429_backs_off(self):
        self.signed_in()
        Fake.state["retry_after"] = 7
        with self.assertRaisesRegex(RuntimeError, "try again in 7 s"):
            spotify.next_track({})
        Fake.state["retry_after"] = 0
        calls = len(Fake.state["calls"])
        with self.assertRaisesRegex(RuntimeError, "slow down"):
            spotify.next_track({})  # not even sent
        spotify.tick()  # the poll waits too
        self.assertEqual(len(Fake.state["calls"]), calls)
        spotify._backoff["until"] = 0.0
        spotify.tick()
        self.assertGreater(len(Fake.state["calls"]), calls)

    # --- values ----------------------------------------------------------------------

    def test_values(self):
        self.signed_in()
        Fake.state["library"].add(TRACK)
        self.assertEqual(spotify.poll(), spotify.POLL_PLAYING)
        self.assertEqual(self.values(), {
            "track": "Never Gonna Give You Up", "artist": "Rick Astley, Guest", "album": "Whenever You Need Somebody",
            "playing": "on", "shuffle": "off", "repeat": "off", "liked": "on", "volume": "40",
            "progress": "1:23 / 3:45", "device": "Desk speaker"})
        # Only changes are sent again, and the liked check isn't repeated for the same track.
        self.sent.clear()
        Fake.state["player"]["progress_ms"] = 84000
        spotify.poll()
        self.assertEqual(self.values(), {"progress": "1:24 / 3:45"})
        self.assertEqual(len([c for c in self.api_calls() if c[1] == "/me/library/contains"]), 1)
        # Paused: slower polling.
        Fake.state["player"]["is_playing"] = False
        self.assertEqual(spotify.poll(), spotify.POLL_IDLE)
        self.assertEqual(self.values()["playing"], "off")

    def test_values_for_an_episode_and_nothing(self):
        self.signed_in()
        Fake.state["player"]["item"] = {"type": "episode", "uri": "spotify:episode:512ojhOuo1ktJprKbVcKyQ",
                                        "name": "Episode 12", "duration_ms": 3725000, "show": {"name": "A Podcast"}}
        Fake.state["player"]["progress_ms"] = 3601000
        spotify.poll()
        v = self.values()
        self.assertEqual((v["track"], v["artist"], v["album"], v["progress"]),
                         ("Episode 12", "A Podcast", "A Podcast", "1:00:01 / 1:02:05"))
        Fake.state["player"] = None
        spotify.poll()
        v = self.values()
        self.assertEqual((v["track"], v["playing"], v["device"]), ("–", "off", "–"))

    def test_values_when_signed_out_or_failing(self):
        spotify.safe_poll()
        self.assertEqual(self.values()["track"], "–")
        self.assertEqual(Fake.state["calls"], [])  # nothing asked without a sign-in
        self.signed_in()
        spotify.plugin.settings["client_id"] = CLIENT_ID
        spotify._state["next"] = 0.0
        spotify.safe_poll()
        self.assertEqual(self.values()["track"], "Never Gonna Give You Up")
        with mock.patch.object(spotify, "API_URL", "http://127.0.0.1:9/v1"):
            for _ in range(3):
                spotify._state["next"] = 0.0
                spotify.safe_poll()
        self.assertEqual(self.values()["track"], "–")
        logs = [m for m in self.sent if m.get("event") == "log" and "poll" in m["message"]]
        self.assertEqual(len(logs), 1, logs)
        self.assertIn("couldn't reach Spotify", logs[0]["message"])


if __name__ == "__main__":
    unittest.main()
