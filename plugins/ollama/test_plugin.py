import http.server
import json
import os
import subprocess
import sys
import threading
import time
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as ollama  # noqa: E402


class Fake(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    state = {}

    def send_json(self, code, data):
        body = json.dumps(data).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        Fake.state["calls"].append(("GET", self.path, None))
        if self.path == "/api/ps":
            return self.send_json(200, {"models": Fake.state["loaded"]})
        self.send_json(404, {"error": "not found"})

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length") or 0)) or b"{}")
        st = Fake.state
        st["calls"].append(("POST", self.path, body))
        model = body.get("model")
        if model not in st["known"]:
            return self.send_json(404, {"error": f"model '{model}' not found"})
        if self.path == "/api/generate":
            time.sleep(st.get("delay", 0))
            if body.get("keep_alive") == 0:
                st["loaded"] = [m for m in st["loaded"] if m["model"] != model]
                return self.send_json(200, {"model": model, "response": "", "done": True, "done_reason": "unload"})
            if not body.get("prompt"):
                st["loaded"].append(entry(model, 2_000_000_000))
                return self.send_json(200, {"model": model, "response": "", "done": True, "done_reason": "load"})
            return self.send_json(200, {"model": model, "response": st["answer"], "done": True, "done_reason": "stop"})
        if self.path == "/api/pull":
            lines = [
                {"status": "pulling manifest"},
                {"status": "pulling abc", "digest": "abc", "total": 200, "completed": 50},
                {"status": "pulling abc", "digest": "abc", "total": 200, "completed": 200},
                {"status": "verifying sha256 digest"},
                {"status": "success"},
            ]
            data = b"".join(json.dumps(x).encode() + b"\n" for x in lines)
            self.send_response(200)
            self.send_header("Content-Type", "application/x-ndjson")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            return
        self.send_json(404, {"error": "not found"})

    def log_message(self, *args):
        pass


def entry(model, vram):
    return {"name": model, "model": model, "size": vram, "size_vram": vram, "digest": "x",
            "details": {"family": "llama"}, "expires_at": "2026-10-11T12:00:00Z"}


class OllamaTest(unittest.TestCase):
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
            "calls": [],
            "known": {"llama3.2:latest", "llama3.2", "qwen3:8b"},
            "loaded": [entry("qwen3:8b", 5_500_000_000)],
            "answer": "Monday again,\n coffee   cools —\n the inbox grows.",
        }
        self.sent = []
        ollama.plugin._send = self.sent.append
        ollama.plugin.settings = {"url": self.base}
        ollama.plugin.watching = set()
        self.copied = []
        patcher = mock.patch.object(ollama, "copy_to_clipboard", self.copied.append)
        patcher.start()
        self.addCleanup(patcher.stop)

    def values(self):
        return {m["id"]: m["value"] for m in self.sent if m.get("event") == "value"}

    def wait_for(self, check, seconds=2.0):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if check():
                return True
            time.sleep(0.01)
        return False

    def test_values(self):
        Fake.state["loaded"].append(entry("llama3.2:latest", 2_000_000_000))
        ollama.plugin.watching = {"model_loaded:llama3.2", "model_loaded:mistral", "model_loaded:qwen3:8b"}
        ollama.safe_poll()
        v = self.values()
        self.assertEqual(v["loaded"], "qwen3:8b, llama3.2")
        self.assertEqual(v["loaded_count"], "2")
        self.assertEqual(v["vram"], "7.5")
        self.assertEqual(v["model_loaded:llama3.2"], "on")
        self.assertEqual(v["model_loaded:qwen3:8b"], "on")
        self.assertEqual(v["model_loaded:mistral"], "off")

    def test_nothing_loaded_and_unreachable(self):
        Fake.state["loaded"] = []
        ollama.safe_poll()
        self.assertEqual(self.values()["loaded"], "none")
        ollama.plugin.settings = {"url": "127.0.0.1:9"}
        ollama.plugin.watching = {"model_loaded:x"}
        ollama.safe_poll()
        v = self.values()
        self.assertEqual((v["loaded"], v["vram"], v["model_loaded:x"]), ("–", "–", "–"))

    def test_default_url(self):
        ollama.plugin.settings = {}
        self.assertEqual(ollama.base_url(), "http://127.0.0.1:11434")

    def test_load_and_unload(self):
        self.assertIsNone(ollama.load({"model": "llama3.2", "keep_alive": "-1"}))
        self.assertIn(("POST", "/api/generate", {"model": "llama3.2", "keep_alive": -1}), Fake.state["calls"])
        self.assertIsNone(ollama.load({"model": "llama3.2", "keep_alive": "30m"}))
        self.assertIn(("POST", "/api/generate", {"model": "llama3.2", "keep_alive": "30m"}), Fake.state["calls"])
        self.assertIsNone(ollama.unload({"model": "qwen3:8b"}))
        self.assertIn(("POST", "/api/generate", {"model": "qwen3:8b", "keep_alive": 0}), Fake.state["calls"])

    def test_slow_load_answers_early(self):
        Fake.state["delay"] = 0.5
        with mock.patch.object(ollama, "WAIT_FOR_ACTION", 0.05):
            start = time.monotonic()
            self.assertIsNone(ollama.load({"model": "llama3.2"}))
            self.assertLess(time.monotonic() - start, 0.4)
            self.assertIn("still loading", ollama.load({"model": "llama3.2"}))
            self.assertTrue(self.wait_for(lambda: "load:llama3.2" not in ollama._busy))

    def test_unload_all(self):
        Fake.state["loaded"].append(entry("llama3.2:latest", 1))
        self.assertIsNone(ollama.unload_all({}))
        self.assertEqual(Fake.state["loaded"], [])
        self.assertEqual(ollama.unload_all({}), "no model is loaded")

    def test_errors(self):
        with self.assertRaisesRegex(RuntimeError, "choose a model"):
            ollama.load({})
        with self.assertRaisesRegex(RuntimeError, "model 'nope' not found"):
            ollama.unload({"model": "nope"})
        with self.assertRaisesRegex(RuntimeError, "model 'nope' not found"):
            ollama.load({"model": "nope"})  # fails within the wait: reported on the key
        with self.assertRaisesRegex(RuntimeError, "write a question"):
            ollama.ask({"model": "qwen3:8b", "prompt": " "})
        ollama.plugin.settings = {"url": "http://127.0.0.1:9"}
        with self.assertRaisesRegex(RuntimeError, "couldn't reach Ollama at http://127.0.0.1:9"):
            ollama.unload_all({})

    def test_pull_reports_progress(self):
        self.assertIsNone(ollama.pull({"model": "llama3.2"}))
        self.assertTrue(self.wait_for(lambda: "pull:llama3.2" not in ollama._busy))
        progress = [m["value"] for m in self.sent if m.get("id") == "pull_status"]
        self.assertIn("llama3.2 25%", progress)
        self.assertEqual(progress[-1], "llama3.2 ready")
        call = [c for c in Fake.state["calls"] if c[1] == "/api/pull"][0]
        self.assertEqual(call[2], {"model": "llama3.2", "stream": True})

    def test_ask_copies_and_publishes(self):
        self.assertIsNone(ollama.ask({"model": "qwen3:8b", "prompt": "haiku"}))
        self.assertEqual(self.copied, [Fake.state["answer"]])
        self.assertEqual(self.values()["last_answer"], "Monday again, coffee cools — the inbox grows.")
        body = [c for c in Fake.state["calls"] if c[1] == "/api/generate"][0][2]
        self.assertEqual(body, {"model": "qwen3:8b", "prompt": "haiku", "stream": False})
        self.assertIsNone(ollama.ask({"model": "qwen3:8b", "prompt": "again", "copy": False}))
        self.assertEqual(len(self.copied), 1)

    def test_slow_answer_is_published_later(self):
        Fake.state["delay"] = 0.3
        with mock.patch.object(ollama, "WAIT_FOR_ACTION", 0.02):
            self.assertIsNone(ollama.ask({"model": "qwen3:8b", "prompt": "haiku"}))
            self.assertEqual(self.values()["last_answer"], "thinking…")
            self.assertEqual(ollama.ask({"model": "qwen3:8b", "prompt": "x"}), "still answering the last question")
            self.assertTrue(self.wait_for(lambda: self.copied))
        self.assertTrue(self.wait_for(lambda: self.values()["last_answer"].startswith("Monday")))

    def test_one_line(self):
        self.assertEqual(ollama.one_line("a\nb  c"), "a b c")
        long = ollama.one_line("x" * 200)
        self.assertEqual(len(long), ollama.ANSWER_LENGTH)
        self.assertTrue(long.endswith("…"))

    def test_same_model(self):
        self.assertTrue(ollama.same_model("llama3.2", "llama3.2:latest"))
        self.assertTrue(ollama.same_model("me/model", "me/model:latest"))
        self.assertFalse(ollama.same_model("llama3.2", "llama3.2:1b"))


class ClipboardTest(unittest.TestCase):
    def run_with(self, platform, which=lambda name: None, env=None):
        with mock.patch.object(ollama.sys, "platform", platform), mock.patch.object(
            ollama.shutil, "which", which
        ), mock.patch.dict(ollama.os.environ, env or {}, clear=True), mock.patch.object(
            ollama.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)
        ) as run:
            ollama.copy_to_clipboard("héllo")
        return run.call_args

    def test_windows_uses_clip_with_utf16(self):
        args = self.run_with("win32")
        self.assertEqual(args.args[0], ["clip"])
        self.assertEqual(args.kwargs["input"], "héllo".encode("utf-16"))
        self.assertTrue(args.kwargs["input"].startswith((b"\xff\xfe", b"\xfe\xff")))

    def test_macos_uses_pbcopy(self):
        args = self.run_with("darwin")
        self.assertEqual(args.args[0], ["pbcopy"])
        self.assertEqual(args.kwargs["input"], "héllo".encode())
        self.assertEqual(args.kwargs["env"]["LANG"], "en_US.UTF-8")

    def test_linux_prefers_wl_copy_on_wayland(self):
        tools = {"wl-copy": "/usr/bin/wl-copy", "xclip": "/usr/bin/xclip"}
        self.assertEqual(self.run_with("linux", tools.get, {"WAYLAND_DISPLAY": "wayland-0"}).args[0], ["wl-copy"])
        self.assertEqual(self.run_with("linux", tools.get, {"DISPLAY": ":0"}).args[0], ["xclip", "-selection", "clipboard"])
        args = self.run_with("linux", tools.get, {"DISPLAY": ":0"})
        self.assertEqual(args.kwargs["stdout"], subprocess.DEVNULL)  # xclip keeps running

    def test_no_tool(self):
        with mock.patch.object(ollama.sys, "platform", "linux"), mock.patch.object(ollama.shutil, "which", lambda n: None):
            with self.assertRaisesRegex(RuntimeError, "no clipboard tool"):
                ollama.copy_to_clipboard("x")


if __name__ == "__main__":
    unittest.main()
