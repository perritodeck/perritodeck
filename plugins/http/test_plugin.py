import http.server
import json
import os
import sys
import threading
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as http_plugin  # noqa: E402


class Handler(http.server.BaseHTTPRequestHandler):
    seen = []

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        Handler.seen.append((self.command, self.path, self.headers.get("X-Test"), self.headers.get("Content-Type"), body))
        self.send_response(204 if self.path != "/missing" else 404)
        self.end_headers()

    do_GET = do_POST

    def log_message(self, *args):
        pass


class HttpPluginTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
        cls.base = f"http://127.0.0.1:{cls.server.server_port}"
        http_plugin.plugin._send = lambda m: None  # no host here

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()

    def test_post_with_json_and_headers(self):
        warn = http_plugin.request({"url": self.base + "/hook", "method": "POST", "body": '{"on": true}', "headers": "X-Test: yes"})
        self.assertIsNone(warn)
        method, path, header, ctype, body = Handler.seen[-1]
        self.assertEqual((method, path, header, ctype), ("POST", "/hook", "yes", "application/json"))
        self.assertEqual(json.loads(body), {"on": True})

    def test_slider_value(self):
        self.assertIsNone(http_plugin.send_value({"url": self.base + "/dim?level={value}", "method": "GET", "value": 42}))
        self.assertEqual(Handler.seen[-1][:2], ("GET", "/dim?level=42"))

    def test_errors(self):
        self.assertIn("404", http_plugin.request({"url": self.base + "/missing", "method": "GET"}))
        with self.assertRaises(ValueError):
            http_plugin.request({"url": "ftp://x"})
        with self.assertRaises(RuntimeError):
            http_plugin.request({"url": "http://127.0.0.1:9/", "method": "GET"})


if __name__ == "__main__":
    unittest.main()
