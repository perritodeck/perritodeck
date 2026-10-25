import base64
import hashlib
import json
import os
import socket
import socketserver
import struct
import sys
import threading
import time
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as truenas  # noqa: E402

KEY = "1-SecretKeyMaterial"
USER = "truenas_admin"


def ws_frame(opcode, payload):
    header = bytes([0x80 | opcode])
    n = len(payload)
    if n < 126:
        header += bytes([n])
    elif n < 65536:
        header += bytes([126]) + struct.pack("!H", n)
    else:
        header += bytes([127]) + struct.pack("!Q", n)
    return header + payload


class FakeTrueNAS(socketserver.BaseRequestHandler):
    """A WebSocket JSON-RPC server shaped like TrueNAS's /api/current."""

    state = {}

    def read(self, n):
        while len(self.buf) < n:
            chunk = self.request.recv(65536)
            if not chunk:
                raise ConnectionError
            self.buf += chunk
        data, self.buf = self.buf[:n], self.buf[n:]
        return data

    def send(self, message, opcode=0x1):
        payload = message if isinstance(message, bytes) else json.dumps(message).encode()
        with self.lock:
            try:
                self.request.sendall(ws_frame(opcode, payload))
            except OSError:
                pass  # the client hung up before a late (job) answer

    def handle(self):
        st = FakeTrueNAS.state
        st["connections"] += 1
        self.buf = b""
        self.lock = threading.Lock()
        self.authed = False
        while b"\r\n\r\n" not in self.buf:
            self.buf += self.request.recv(4096)
        head, self.buf = self.buf.split(b"\r\n\r\n", 1)
        lines = head.decode().split("\r\n")
        st["paths"].append(lines[0])
        headers = {k.lower(): v.strip() for k, _, v in (line.partition(":") for line in lines[1:])}
        accept = base64.b64encode(hashlib.sha1((headers["sec-websocket-key"] + truenas.WS_GUID).encode()).digest())
        self.request.sendall(
            b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
            b"Sec-WebSocket-Accept: " + accept + b"\r\n\r\n"
        )
        try:
            while True:
                b0, b1 = self.read(2)
                assert b1 & 0x80, "client frames must be masked"
                n = b1 & 0x7F
                if n == 126:
                    n = struct.unpack("!H", self.read(2))[0]
                elif n == 127:
                    n = struct.unpack("!Q", self.read(8))[0]
                mask = self.read(4)
                payload = truenas.masked(self.read(n), mask)
                opcode = b0 & 0x0F
                if opcode == 0x8:
                    return
                if opcode == 0xA:
                    st["pongs"].append(payload)
                    continue
                request = json.loads(payload)
                threading.Thread(target=self.answer, args=(request,), daemon=True).start()
        except (ConnectionError, OSError):
            pass

    def answer(self, request):
        st = FakeTrueNAS.state
        method, params, rid = request["method"], request.get("params") or [], request["id"]
        st["calls"].append((method, params))
        assert request["jsonrpc"] == "2.0"

        def ok(result):
            self.send({"jsonrpc": "2.0", "id": rid, "result": result})

        def err(code, message, errname=None, reason=None, extra=None):
            data = {"error": 22, "errname": errname, "reason": reason, "extra": extra}
            self.send({"jsonrpc": "2.0", "id": rid, "error": {"code": code, "message": message, "data": data}})

        if method == "auth.login_ex":
            st["logins"] += 1
            p = params[0]
            good = p["mechanism"] == "API_KEY_PLAIN" and p["username"] == USER and p["api_key"] == KEY
            self.authed = good
            return ok({"response_type": "SUCCESS" if good else "AUTH_ERR"})
        if method == "auth.login_with_api_key":
            st["logins"] += 1
            self.authed = params == [KEY]
            return ok(self.authed)
        if not self.authed:
            return err(-32001, "Method call error", "ENOTAUTHENTICATED", "Not authenticated")
        if method == "core.subscribe":
            ok("sub-1")
            time.sleep(0.02)
            self.send({"jsonrpc": "2.0", "method": "collection_update", "params": {
                "msg": "added", "collection": "reporting.realtime",
                "fields": {"cpu": {"cpu": {"usage": 12.6, "temp": 41.2}, "cpu0": {"usage": 10, "temp": None}},
                           "memory": {}, "disks": {}, "interfaces": {}}}})
            return None
        if method == "alert.list":
            return ok(st["alerts"])
        if method == "alert.dismiss":
            time.sleep(st.get("dismiss_delay", 0))
            for a in st["alerts"]:
                if a["uuid"] == params[0]:
                    a["dismissed"] = True
            return ok(None)
        if method == "pool.query":
            self.send(b"keepalive", opcode=0x9)  # a ping in between
            return ok(st["pools"])
        if method == "app.query":
            filters = params[0] if params else []
            apps = st["apps"]
            for name, op, value in filters:
                apps = [a for a in apps if (a[name] in value if op == "in" else a[name] == value)]
            return ok(apps)
        if method in ("app.start", "app.stop"):
            app = next((a for a in st["apps"] if a["name"] == params[0]), None)
            if app is None:
                return err(-32001, "Method call error", "ENOENT", f"App {params[0]!r} not found")
            time.sleep(st.get("job_delay", 0))
            app["state"] = "RUNNING" if method == "app.start" else "STOPPED"
            return ok(None)
        if method == "pool.scrub.scrub":
            if params[0] not in [p["name"] for p in st["pools"]]:
                return err(-32001, "Method call error", "EINVAL", "x", [["pool.scrub.scrub.name", "Pool not found", 22]])
            time.sleep(st.get("job_delay", 0))
            return ok(None)
        if method in ("cloudsync.sync", "replication.run"):
            time.sleep(st.get("job_delay", 0))
            return ok(None)
        err(-32601, "Method not found")

    def finish(self):
        pass


class Server(socketserver.ThreadingTCPServer):
    daemon_threads = True
    allow_reuse_address = True


def app(name, state):
    return {"name": name, "id": name, "state": state, "version": "1.0.0", "upgrade_available": False}


class TrueNASTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = Server(("127.0.0.1", 0), FakeTrueNAS)
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
        cls.base = f"http://127.0.0.1:{cls.server.server_address[1]}"

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()

    def setUp(self):
        FakeTrueNAS.state = {
            "connections": 0, "logins": 0, "calls": [], "paths": [], "pongs": [],
            "alerts": [
                {"uuid": "a1", "level": "WARNING", "dismissed": False, "formatted": "Disk is hot"},
                {"uuid": "a2", "level": "CRITICAL", "dismissed": True, "formatted": "old"},
                {"uuid": "a3", "level": "INFO", "dismissed": False, "formatted": "Update available"},
            ],
            "pools": [
                {"name": "tank", "status": "ONLINE", "healthy": True, "size": 1000, "allocated": 423, "free": 577,
                 "padding": "x" * 70000},  # a message over 64 KiB
                {"name": "scratch", "status": "DEGRADED", "healthy": False, "size": None, "allocated": None},
            ],
            "apps": [app("jellyfin", "RUNNING"), app("immich", "STOPPED")],
        }
        self.sent = []
        truenas.plugin._send = self.sent.append
        truenas.plugin.watching = set()
        truenas.plugin.settings = {"url": self.base, "username": USER, "api_key": KEY}
        truenas.disconnect()
        truenas._realtime["published_at"] = 0.0
        patches = [mock.patch.object(truenas, "ALLOW_PLAIN_WS", True),
                   mock.patch.object(truenas, "refresh_apps_soon", lambda: None)]
        for p in patches:
            p.start()
            self.addCleanup(p.stop)
        self.addCleanup(truenas.disconnect)

    def values(self):
        return {m["id"]: m["value"] for m in self.sent if m.get("event") == "value"}

    def calls(self, method):
        return [p for m, p in FakeTrueNAS.state["calls"] if m == method]

    def wait_for(self, check, seconds=2.0):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if check():
                return True
            time.sleep(0.01)
        return False

    def test_values_and_one_login(self):
        truenas.plugin.watching = {
            "pool:tank", "pool_ok:tank", "pool_used:tank", "pool:scratch", "pool_ok:scratch", "pool_used:scratch",
            "pool:missing", "app:jellyfin", "app_running:jellyfin", "app:immich", "app_running:immich", "app:nope",
        }
        truenas.safe_poll()
        truenas.safe_poll()
        v = self.values()
        self.assertEqual((v["pool:tank"], v["pool_ok:tank"], v["pool_used:tank"]), ("ONLINE", "on", "42"))
        self.assertEqual((v["pool:scratch"], v["pool_ok:scratch"], v["pool_used:scratch"]), ("DEGRADED", "off", "–"))
        self.assertEqual(v["pool:missing"], "–")
        self.assertEqual((v["alerts"], v["alert_level"]), ("2", "WARNING"))
        self.assertEqual((v["app:jellyfin"], v["app_running:jellyfin"]), ("running", "on"))
        self.assertEqual((v["app:immich"], v["app_running:immich"]), ("stopped", "off"))
        self.assertEqual(v["app:nope"], "–")
        self.assertEqual(self.calls("app.query")[-1], [[["name", "in", ["immich", "jellyfin", "nope"]]]])
        # One connection and one login for everything, on /api/current.
        self.assertEqual((FakeTrueNAS.state["connections"], FakeTrueNAS.state["logins"]), (1, 1))
        self.assertTrue(FakeTrueNAS.state["paths"][0].startswith("GET /api/current HTTP/1.1"))
        self.assertEqual(self.calls("auth.login_ex")[0], [{"mechanism": "API_KEY_PLAIN", "username": USER, "api_key": KEY}])
        self.assertEqual(FakeTrueNAS.state["pongs"], [b"keepalive", b"keepalive"])

    def test_realtime_cpu_and_temp(self):
        truenas.safe_poll()
        self.assertTrue(self.wait_for(lambda: "cpu" in self.values() and self.values()["cpu"] != "–"))
        self.assertEqual((self.values()["cpu"], self.values()["temp"]), ("13", "41"))
        self.assertEqual(self.calls("core.subscribe"), [["reporting.realtime"]])

    def test_no_alerts(self):
        for a in FakeTrueNAS.state["alerts"]:
            a["dismissed"] = True
        truenas.safe_poll(apps=False)
        self.assertEqual((self.values()["alerts"], self.values()["alert_level"]), ("0", "none"))
        self.assertEqual(truenas.dismiss_alerts({}), "there are no alerts to dismiss")

    def test_dismiss_alerts(self):
        self.assertIsNone(truenas.dismiss_alerts({}))
        self.assertEqual(self.calls("alert.dismiss"), [["a1"], ["a3"]])
        self.assertEqual(self.values()["alerts"], "0")

    def test_app_actions(self):
        self.assertIsNone(truenas.stop_app({"app": "jellyfin"}))
        self.assertIsNone(truenas.toggle_app({"app": "jellyfin"}))  # stopped -> start
        self.assertIsNone(truenas.toggle_app({"app": "immich"}))  # stopped -> start
        self.assertIsNone(truenas.toggle_app({"app": "immich"}))  # running -> stop
        self.assertEqual(self.calls("app.stop"), [["jellyfin"], ["immich"]])
        self.assertEqual(self.calls("app.start"), [["jellyfin"], ["immich"]])
        self.assertIsNone(truenas.restart_app({"app": "jellyfin"}))
        self.assertEqual(self.calls("app.stop")[-1], ["jellyfin"])
        self.assertEqual(self.calls("app.start")[-1], ["jellyfin"])

    def test_slow_jobs_answer_early(self):
        FakeTrueNAS.state["job_delay"] = 0.5
        # Connected and logged in first, so only the actions are timed: on a slow runner
        # (macOS) logging in took most of the margin the three actions had together.
        truenas.client()
        with mock.patch.object(truenas, "JOB_WAIT", 0.05):
            actions = [
                ("start_app", truenas.start_app, {"app": "immich"}),
                ("scrub", truenas.scrub, {"pool": "tank"}),
                ("run_task", truenas.run_task, {"kind": "replication", "task_id": "3"}),
            ]
            for name, action, settings in actions:
                # Each answers after JOB_WAIT; one that waited for its job would take 0.5 s.
                start = time.monotonic()
                self.assertIsNone(action(settings))
                self.assertLess(time.monotonic() - start, 0.45, name)
            # The connection is still fine afterwards, and late answers are dropped.
            truenas.safe_poll(apps=False)
            self.assertEqual(self.values()["alerts"], "2")
            self.assertEqual(FakeTrueNAS.state["connections"], 1)
        self.assertEqual(self.calls("pool.scrub.scrub"), [["tank", "START"]])
        self.assertEqual(self.calls("replication.run"), [[3]])

    def test_run_cloud_sync(self):
        self.assertIsNone(truenas.run_task({"kind": "cloudsync", "task_id": 7}))
        self.assertEqual(self.calls("cloudsync.sync"), [[7]])
        with self.assertRaisesRegex(RuntimeError, "task's number"):
            truenas.run_task({"kind": "cloudsync", "task_id": "x"})

    def test_errors(self):
        with self.assertRaisesRegex(RuntimeError, "App 'ghost' not found"):
            truenas.start_app({"app": "ghost"})
        with self.assertRaisesRegex(RuntimeError, "no app called ghost"):
            truenas.toggle_app({"app": "ghost"})
        with self.assertRaisesRegex(RuntimeError, "Pool not found"):
            truenas.scrub({"pool": "nope"})
        with self.assertRaisesRegex(RuntimeError, "app's name"):
            truenas.start_app({})

    def test_bad_key_backs_off(self):
        truenas.plugin.settings["api_key"] = "1-wrong"
        with self.assertRaisesRegex(RuntimeError, "refused the API key"):
            truenas.start_app({"app": "immich"})
        with self.assertRaisesRegex(RuntimeError, "refused the API key"):
            truenas.start_app({"app": "immich"})
        self.assertEqual(FakeTrueNAS.state["logins"], 1)  # no second login attempt yet
        truenas.plugin.watching = {"pool:tank"}
        truenas.safe_poll()
        self.assertEqual(self.values()["pool:tank"], "–")
        self.assertFalse(any("wrong" in json.dumps(m) or KEY in json.dumps(m) for m in self.sent))
        # New settings: try again at once.
        truenas.plugin.settings["api_key"] = KEY
        truenas.settings_changed(truenas.plugin.settings)
        self.assertIsNone(truenas.start_app({"app": "immich"}))

    def test_legacy_login_without_user_name(self):
        del truenas.plugin.settings["username"]
        truenas.safe_poll(apps=False)
        self.assertEqual(self.calls("auth.login_with_api_key"), [[KEY]])
        self.assertEqual(self.values()["alerts"], "2")

    def test_reconnects_after_the_connection_drops(self):
        truenas.safe_poll(apps=False)
        truenas._conn["conn"].ws.sock.close()
        self.assertTrue(self.wait_for(lambda: not truenas._conn["conn"].alive))
        truenas.safe_poll(apps=False)
        self.assertEqual(FakeTrueNAS.state["connections"], 2)
        self.assertEqual(self.values()["alerts"], "2")

    def test_settings(self):
        with mock.patch.object(truenas, "ALLOW_PLAIN_WS", False):
            truenas.plugin.settings = {"url": "http://nas.local", "api_key": "k"}
            with self.assertRaisesRegex(RuntimeError, "https://"):
                truenas.config()
            truenas.plugin.settings = {"url": "nas.local:8443/", "api_key": "k", "insecure": True}
            self.assertEqual(truenas.config(), ("wss://nas.local:8443/api/current", "", "k", True))
            truenas.plugin.settings = {"url": "https://nas.local"}
            with self.assertRaisesRegex(RuntimeError, "API key"):
                truenas.config()

    def test_unreachable(self):
        truenas.plugin.settings["url"] = "http://127.0.0.1:9"
        with self.assertRaisesRegex(RuntimeError, "couldn't reach TrueNAS at http://127.0.0.1:9"):
            truenas.scrub({"pool": "tank"})
        truenas.plugin.watching = {"app:jellyfin"}
        truenas.safe_poll()
        v = self.values()
        self.assertEqual((v["app:jellyfin"], v["alerts"], v["cpu"]), ("–", "–", "–"))

    def test_rpc_error_text(self):
        self.assertEqual(
            truenas.describe_rpc_error("x", {"code": -32001, "message": "Method call error",
                                             "data": {"errname": "EINVAL", "reason": "[EINVAL] bad\ntrace"}}),
            "TrueNAS couldn't do x: [EINVAL] bad")
        self.assertIn("isn't allowed", truenas.describe_rpc_error("x", {"data": {"errname": "EACCES", "reason": "no"}}))

    def test_dismiss_alerts_answers_in_time(self):
        FakeTrueNAS.state["alerts"] += [
            {"uuid": f"b{i}", "level": "INFO", "dismissed": False, "formatted": "x"} for i in range(3)
        ]
        FakeTrueNAS.state["dismiss_delay"] = 0.15
        with mock.patch.object(truenas, "ACTION_DEADLINE", 0.3):
            answer = truenas.dismiss_alerts({})
        self.assertRegex(answer, r"^still dismissing: \d of 5 done…$")
        self.assertTrue(self.wait_for(lambda: len(self.calls("alert.dismiss")) == 5, 5))

    def test_new_settings_never_wait_for_a_connection_attempt(self):
        real = truenas.connect
        entered = threading.Event()

        def slow_connect(*args):
            entered.set()
            time.sleep(0.6)
            return real(*args)

        outcome = {}

        def press():
            try:
                outcome["result"] = truenas.client()
            except Exception as e:
                outcome["error"] = e

        with mock.patch.object(truenas, "connect", slow_connect):
            thread = threading.Thread(target=press)
            thread.start()
            self.assertTrue(entered.wait(2))
            began = time.monotonic()
            truenas.settings_changed(truenas.plugin.settings)
            self.assertLess(time.monotonic() - began, 0.2)
            # Polls and other presses don't wait on the state lock either.
            with truenas._state_lock:
                pass
            thread.join(5)
        # The attempt started before the change is thrown away, not used.
        self.assertIn("settings changed", str(outcome.get("error")))
        self.assertIsNone(truenas._conn["conn"])
        self.assertTrue(self.wait_for(lambda: FakeTrueNAS.state["connections"] == 1))
        self.assertIsNotNone(truenas.client())

    def test_masking_round_trips(self):
        data = os.urandom(1001)
        self.assertEqual(truenas.masked(truenas.masked(data, b"abcd"), b"abcd"), data)
        self.assertEqual(truenas.masked(b"", b"abcd"), b"")



class FrameRulesTest(unittest.TestCase):
    """The client refuses frames a server must never send (RFC 6455 5.1, 5.4, 5.5)."""

    def received(self, *frames):
        a, b = socket.socketpair()
        self.addCleanup(a.close)
        self.addCleanup(b.close)
        self.peer = b
        ws = truenas.WebSocket.over(a)
        b.sendall(b"".join(frames))
        return ws.recv()

    def refused(self, code, *frames):
        with self.assertRaises(truenas.WebSocketClosed) as caught:
            self.received(*frames)
        self.assertEqual(caught.exception.code, code)
        # The server is told why with a close frame (masked, as from a client).
        self.peer.settimeout(1)
        head = self.peer.recv(2)
        self.assertEqual(head[0], 0x80 | truenas.OP_CLOSE)

    def test_fragments_with_a_ping_between(self):
        rest = "llo ☕".encode()
        frames = [bytes([0x01, 2]) + b"he", ws_frame(0x9, b"p"), bytes([0x80, len(rest)]) + rest]
        self.assertEqual(self.received(*frames), "hello ☕")

    def test_masked_server_frames_are_refused(self):
        self.refused(1002, bytes([0x81, 0x80 | 2]) + b"\0\0\0\0" + b"hi")

    def test_control_frame_rules(self):
        self.refused(1002, ws_frame(0x9, b"x" * 126))
        self.refused(1002, bytes([0x09, 1]) + b"x")  # a ping without FIN
        self.refused(1002, ws_frame(0xB, b""))

    def test_fragment_ordering(self):
        self.refused(1002, ws_frame(0x0, b"x"))
        self.refused(1002, bytes([0x01, 1]) + b"a", ws_frame(0x1, b"b"))
        self.refused(1002, bytes([0x80 | 0x40 | 0x1, 1]) + b"x")
        self.refused(1007, ws_frame(0x1, b"\xff"))

    def test_size_limit(self):
        self.assertLessEqual(truenas.MAX_MESSAGE, 8 << 20)
        self.refused(1009, bytes([0x81, 127]) + struct.pack("!Q", truenas.MAX_MESSAGE + 1))
        with mock.patch.object(truenas, "MAX_MESSAGE", 10):
            self.refused(1009, bytes([0x01, 6]) + b"x" * 6, bytes([0x80, 6]) + b"x" * 6)

    def test_close_wakes_a_blocked_reader(self):
        a, b = socket.socketpair()
        self.addCleanup(b.close)
        ws = truenas.WebSocket.over(a)
        failed = threading.Event()

        def reader():
            try:
                ws.recv()
            except (ConnectionError, OSError):
                failed.set()

        threading.Thread(target=reader, daemon=True).start()
        time.sleep(0.05)
        ws.close()
        self.assertTrue(failed.wait(2))


if __name__ == "__main__":
    unittest.main()
