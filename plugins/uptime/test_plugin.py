import http.server
import os
import shutil
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as uptime  # noqa: E402


class Handler(http.server.BaseHTTPRequestHandler):
    seen = []

    def do_GET(self):
        Handler.seen.append((self.path, self.headers.get("User-Agent")))
        code = {"/ok": 200, "/moved": 302, "/broken": 503, "/login": 401}.get(self.path, 404)
        self.send_response(code)
        if code == 302:
            self.send_header("Location", "/broken")
        self.send_header("Content-Length", "0")
        self.end_headers()

    def log_message(self, *args):
        pass


def closed_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


class UptimeTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
        cls.port = cls.server.server_port
        cls.base = f"http://127.0.0.1:{cls.port}"

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()

    def setUp(self):
        self.sent = []
        uptime.plugin._send = self.sent.append
        uptime.plugin.settings = {}
        uptime.plugin.watching = set()
        uptime._results.clear()
        uptime._check_soon.clear()
        uptime._last_poll = 0.0

    def published(self):
        return {m["id"]: m["value"] for m in self.sent if m.get("event") == "value"}

    def test_split_host_port(self):
        self.assertEqual(uptime.split_host_port("nas:22"), ("nas", 22))
        self.assertEqual(uptime.split_host_port("[fd00::5]:8006"), ("fd00::5", 8006))
        self.assertEqual(uptime.split_host_port("fd00::5"), ("fd00::5", None))
        self.assertEqual(uptime.split_host_port("nas"), ("nas", None))
        with self.assertRaises(ValueError):
            uptime.split_host_port("nas:http")
        with self.assertRaises(ValueError):
            uptime.split_host_port("nas:70000")

    def test_http_checks(self):
        ok = uptime.check(self.base + "/ok")
        self.assertEqual((ok.up, ok.status), (True, "200"))
        self.assertRegex(ok.values()["ms"], r"^\d+ ms$")
        self.assertEqual(Handler.seen[-1], ("/ok", "PerritoDeck uptime"))
        moved = uptime.check(self.base + "/moved")  # 3xx counts as up; not followed
        self.assertEqual((moved.up, moved.status), (True, "302"))
        self.assertEqual(Handler.seen[-1][0], "/moved")
        broken = uptime.check(self.base + "/broken")
        self.assertEqual(broken.values(), {"up": "off", "ms": "–", "status": "503"})
        gone = uptime.check(f"http://127.0.0.1:{closed_port()}/")
        self.assertEqual(gone.values(), {"up": "off", "ms": "–", "status": "down"})

    def test_tcp_checks(self):
        self.assertEqual(uptime.check(f"127.0.0.1:{self.port}").values()["status"], "up")
        self.assertEqual(uptime.check(f"127.0.0.1:{closed_port()}").values()["up"], "off")
        self.assertEqual(uptime.check("nas:notaport").status, "down")
        self.assertTrue(any("can't check" in m.get("message", "") for m in self.sent))

    def test_bare_host_tries_443_then_80(self):
        calls = []

        def fake(host, port):
            calls.append((host, port))
            return uptime.Result(port == 80, "up" if port == 80 else "down", 5)

        with mock.patch.object(uptime, "check_tcp", fake):
            self.assertTrue(uptime.check("printer.lan").up)
        self.assertEqual(calls, [("printer.lan", 443), ("printer.lan", 80)])

    def test_poll_publishes_only_watched_and_changes(self):
        ok, bad = self.base + "/ok", f"127.0.0.1:{closed_port()}"
        uptime.plugin.watching = {f"up:{ok}", f"status:{ok}", f"up:{bad}", f"ms:{bad}"}
        uptime.poll(now=100.0)
        self.assertEqual(
            self.published(),
            {f"up:{ok}": "on", f"status:{ok}": "200", f"up:{bad}": "off", f"ms:{bad}": "–"},
        )
        self.sent.clear()
        n = len(Handler.seen)
        uptime.poll(now=105.0)  # not due yet (every 10 s)
        self.assertEqual(len(Handler.seen), n)
        uptime.poll(now=111.0)  # due: checked again, but nothing changed so nothing is sent
        self.assertEqual(len(Handler.seen), n + 1)
        self.assertEqual(self.published(), {})

    def test_interval_setting(self):
        self.assertEqual(uptime.interval(), 10)
        uptime.plugin.settings = {"interval": "30"}
        self.assertEqual(uptime.interval(), 30)
        uptime.plugin.settings = {"interval": 0.5}
        self.assertEqual(uptime.interval(), 2)
        uptime.plugin.settings = {"interval": "soon"}
        self.assertEqual(uptime.interval(), 10)

    def test_watch_changes(self):
        ok = self.base + "/ok"
        uptime.plugin.watching = {f"up:{ok}"}
        uptime.watching(sorted(uptime.plugin.watching))
        self.assertTrue(uptime._check_soon.is_set())  # new target: check soon
        uptime.poll(now=1.0)
        self.assertFalse(uptime._check_soon.is_set())
        self.sent.clear()
        # Another key shows the same target's status: known, so published straight away.
        uptime.plugin.watching = {f"up:{ok}", f"status:{ok}"}
        uptime.watching(sorted(uptime.plugin.watching))
        self.assertEqual(self.published()[f"status:{ok}"], "200")
        self.assertFalse(uptime._check_soon.is_set())
        uptime.plugin.watching = set()
        uptime.watching([])
        self.assertEqual(uptime._results, {})

    def test_check_now_action(self):
        self.assertIn("nothing to check", uptime.check_now({}))
        self.assertIsNone(uptime.check_now({"target": self.base + "/ok"}))
        self.assertEqual(uptime.check_now({"target": self.base + "/broken"}), self.base + "/broken (503) is down")
        bad = f"127.0.0.1:{closed_port()}"
        uptime.plugin.watching = {f"up:{bad}", f"up:{self.base}/login"}
        self.assertEqual(uptime.check_now({}), f"{bad}, {self.base}/login (401) are down")
        self.assertEqual(self.published()[f"up:{bad}"], "off")

    @unittest.skipUnless(shutil.which("openssl"), "needs openssl to make a test certificate")
    def test_self_signed_certificate_needs_the_setting(self):
        with tempfile.TemporaryDirectory() as d:
            cert, key = os.path.join(d, "c.pem"), os.path.join(d, "k.pem")
            subprocess.run(
                ["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1", "-subj", "/CN=localhost",
                 "-keyout", key, "-out", cert],
                check=True, capture_output=True,
            )
            server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
            context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            context.load_cert_chain(cert, key)
            server.socket = context.wrap_socket(server.socket, server_side=True)
            threading.Thread(target=server.serve_forever, daemon=True).start()
            try:
                url = f"https://127.0.0.1:{server.server_port}/ok"
                self.assertEqual(uptime.check(url).status, "down")
                self.assertTrue(any("certificate" in m.get("message", "") for m in self.sent))
                uptime.plugin.settings = {"insecure": True}
                self.assertEqual(uptime.check(url).status, "200")
            finally:
                server.shutdown()


if __name__ == "__main__":
    unittest.main()
