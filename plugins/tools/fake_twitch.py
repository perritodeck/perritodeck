"""The fake Twitch from plugins/twitch/test_plugin.py (a fake id.twitch.tv and Helix in one
HTTP server, and a fake EventSub WebSocket server), started the way its tests start them,
for the tools' dry runs (--dry-run-fake) and for test_tools.py.

The fake's starting state comes from test_plugin.Base.setUp, so it stays what the plugin's
own tests use. On top of test_plugin's fake this adds only what the tools need: DELETE
/channel_points/custom_rewards (the cleanup script), GET /channels/followers with user_id
(does V follow T?) and accounts' creation dates (the harness's account check), a sign-in
approved by another account (state["signed_in_as"]) or denied (state["deny"]), keepalives on the EventSub
connection (the plugin's tests send those by hand), and hooks that stand in for a person:
approving a sign-in, going live.
"""

import http.server
import os
import sys
import threading

from twitch_common import TWITCH_DIR

if TWITCH_DIR not in sys.path:
    sys.path.insert(0, TWITCH_DIR)
import test_plugin  # noqa: E402  (plugins/twitch/test_plugin.py: the fakes)

CLIENT_ID = test_plugin.CLIENT_ID
ME = test_plugin.ME  # the account that signs in to the fake
OTHER = test_plugin.OTHER
VIEWER = test_plugin.VIEWER
USERS = test_plugin.USERS
CREATED = "2015-06-01T12:00:00Z"  # every fake account's creation date


def with_created(user):
    return dict({"created_at": CREATED}, **user)


class ToolsFake(test_plugin.Fake):
    """test_plugin's fake id.twitch.tv and Helix, plus deleting app-made rewards, the
    follow check, a sign-in approved by another account and one denied."""

    def oauth(self, path, form):
        st = test_plugin.Fake.state
        who = st.get("signed_in_as")
        if path == "/oauth2/validate" and who and self.headers.get("Authorization") == f"OAuth {st['access']}":
            return self.send_json(200, {"client_id": CLIENT_ID, "login": who["login"], "scopes": sorted(st["scopes"]),
                                        "user_id": who["id"], "expires_in": 5000})
        if path == "/oauth2/token" and form.get("grant_type") == test_plugin.twitch.DEVICE_GRANT and st.get("deny"):
            return self.send_json(400, {"status": 400, "message": "access_denied"})  # the person pressed Deny
        return super().oauth(path, form)

    def helix(self, cmd, path, query, one, body):
        st = test_plugin.Fake.state
        if path == "/users":
            if "login" not in query and st.get("signed_in_as"):  # the token belongs to someone else
                return 200, {"data": [with_created(st["signed_in_as"])]}
            code, answer = super().helix(cmd, path, query, one, body)
            if code == 200:
                answer = dict(answer, data=[with_created(u) for u in answer.get("data") or []])
            return code, answer
        if path == "/channels/followers" and "user_id" in one:
            user = next((u for u in USERS.values() if u["id"] == one["user_id"]), None)
            follows = (user is not None and one.get("broadcaster_id") == ME["id"]
                       and user["id"] in st.get("follows_me", set()) and "moderator:read:followers" in st["scopes"])
            data = [{"user_id": user["id"], "user_login": user["login"], "user_name": user["display_name"],
                     "followed_at": "2026-10-01T12:00:00Z"}] if follows else []
            return 200, {"data": data, "total": st["followers"].get(one.get("broadcaster_id"), 0), "pagination": {}}
        if path == "/channel_points/custom_rewards" and cmd == "DELETE":
            if "channel:manage:redemptions" not in st["scopes"]:
                return 401, {"error": "Unauthorized", "status": 401, "message": "Missing scope: channel:manage:redemptions"}
            reward = next((r for r in st["rewards"] if r["id"] == one.get("id")), None)
            if reward is None:
                return 404, {"error": "Not Found", "status": 404, "message": "custom reward not found"}
            if not reward["mine"]:
                return 403, {"error": "Forbidden", "status": 403,
                             "message": "The ID in the Client-Id header must match the client ID used to create the custom reward."}
            st["rewards"].remove(reward)
            st.setdefault("deleted_rewards", []).append(reward["id"])
            return 204, None
        return super().helix(cmd, path, query, one, body)


def fresh_state(base):
    """The fake's state as test_plugin's tests start with it (Base.setUp builds it). A
    throwaway test case runs that setUp and its cleanups; the state stays behind."""
    case = test_plugin.Base()  # a TestCase made only for its setUp; it runs no test
    case.base = base
    case.setUp()
    state = test_plugin.Fake.state
    case.doCleanups()
    return state


class FakeTwitch:
    """Both fake servers, on 127.0.0.1 ports of their own."""

    def __init__(self, keepalive_every=4.0):
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), ToolsFake)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.base = f"http://127.0.0.1:{self.server.server_port}"
        self.id_url = self.base
        self.helix_url = self.base + "/helix"
        self.eventsub = test_plugin.FakeEventSub(keepalive=10)
        self.eventsub_url = self.eventsub.url
        self._stop = threading.Event()
        self.reset()
        threading.Thread(target=self._keepalives, args=(keepalive_every,), daemon=True).start()

    @property
    def state(self):
        return test_plugin.Fake.state

    def reset(self):
        test_plugin.Fake.state = fresh_state(self.base)
        self.state["pending_polls"] = 1  # the "person" approves on the plugin's second poll
        self.state["follows_me"] = {VIEWER["id"]}  # the test viewer follows the test broadcaster
        self.state["signed_in_as"] = None  # None: whoever approves is ME

    def approve_next_sign_in(self):
        """Stand in for a person approving the device code: the next sign-in's code is
        approved after one 'authorization_pending', by state["signed_in_as"] (ME unless a
        test set another account)."""
        self.state.update(device_used=False, device_polls=0, pending_polls=1)

    def set_live(self, login, on):
        user = USERS.get((login or "").lower())
        if user is None:
            return
        streams = [s for s in self.state["streams"] if s["user_id"] != user["id"]]
        if on:
            streams.append({"id": "4095" + user["id"], "user_id": user["id"], "user_login": user["login"], "type": "live",
                            "title": "Fake stream", "viewer_count": 3, "started_at": "2026-10-11T18:00:00Z"})
        self.state["streams"] = streams
        self.state["live"] = bool(on) if user is ME else self.state.get("live")

    def end_sign_in(self):
        """Stand in for disconnecting the app on twitch.tv: the tokens stop working."""
        self.state["access"] = self.state["refresh"] = "ended-on-twitch"

    def _keepalives(self, every):
        while not self._stop.wait(every):
            for conn in list(self.eventsub.conns):
                if conn.closed_by_client:
                    continue
                try:
                    conn.send(self.eventsub.message("session_keepalive", {}))
                except OSError:
                    pass

    def requests(self, method=None, path=None):
        """Helix requests the fake answered: (method, path, query, body)."""
        return [r for r in self.state["requests"]
                if (method is None or r[0] == method) and (path is None or r[1] == path)]

    def stop(self):
        self._stop.set()
        self.eventsub.stop()
        self.server.shutdown()
        self.server.server_close()


def plugin_overrides(fake):
    """Arguments for twitch_trace.py run that point the plugin at these fakes."""
    return ["--id-url", fake.id_url, "--helix-url", fake.helix_url, "--eventsub-url", fake.eventsub_url,
            "--no-browser", "--fast-polls", "--set", "MIN_DEVICE_POLL=0.05"]


if __name__ == "__main__":
    fake = FakeTwitch()
    print(f"fake id.twitch.tv and Helix: {fake.base}  fake EventSub: {fake.eventsub_url}", file=sys.stderr)
    print(f"client ID: {CLIENT_ID}; signs in as {ME['login']}. Ctrl+C stops it.", file=sys.stderr)
    try:
        threading.Event().wait()
    except KeyboardInterrupt:
        fake.stop()
    os._exit(0)
