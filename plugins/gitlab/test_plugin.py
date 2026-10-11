import http.server
import json
import os
import sys
import threading
import time
import unittest
import urllib.parse
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as gitlab  # noqa: E402

TOKEN = "glpat-test-token"
PROJECT = "homelab%2Finfra"  # group/project, URL-encoded as the id


def pipeline(pid, status, ref="main"):
    return {"id": pid, "iid": pid, "project_id": 7, "status": status, "ref": ref, "sha": "abc",
            "source": "push", "web_url": f"https://git.example/homelab/infra/-/pipelines/{pid}"}


class Fake(http.server.BaseHTTPRequestHandler):
    state = {}

    def send_json(self, code, data, headers=None):
        body = json.dumps(data).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

    def handle_any(self):
        st = Fake.state
        length = int(self.headers.get("Content-Length") or 0)
        body = json.loads(self.rfile.read(length)) if length else None
        url = urllib.parse.urlparse(self.path)
        query = dict(urllib.parse.parse_qsl(url.query))
        # The raw path keeps %2F: http.server doesn't decode it.
        st["calls"].append((self.command, url.path, query, body, self.headers.get("PRIVATE-TOKEN")))
        if self.headers.get("PRIVATE-TOKEN") != TOKEN:
            return self.send_json(401, {"message": "401 Unauthorized"})
        base = f"/api/v4/projects/{PROJECT}"
        if not url.path.startswith(base):
            return self.send_json(404, {"message": "404 Project Not Found"})
        rest = url.path[len(base):]
        if rest == "" and self.command == "GET":
            return self.send_json(200, {"id": 7, "path_with_namespace": "homelab/infra", "default_branch": "main"})
        if rest == "/pipelines" and self.command == "GET":
            found = [p for p in st["pipelines"]
                     if query.get("ref") in (None, p["ref"])
                     and query.get("status") in (None, p["status"])
                     and (query.get("scope") is None or p["status"] == query["scope"])]
            found.sort(key=lambda p: -p["id"])
            return self.send_json(200, found[: int(query.get("per_page", 20))])
        if rest == "/pipeline" and self.command == "POST":
            if st.get("read_only"):
                return self.send_json(403, {"message": "403 Forbidden"})
            if not body.get("ref"):
                return self.send_json(400, {"message": {"base": ["Reference not found"]}})
            new = pipeline(max(p["id"] for p in st["pipelines"]) + 1, "created", body["ref"])
            st["pipelines"].append(new)
            return self.send_json(201, new)
        parts = rest.split("/")
        if len(parts) == 4 and parts[1] == "pipelines" and self.command == "POST":
            time.sleep(st.get("slow_cancel", 0) if parts[3] == "cancel" else 0)
            p = next(p for p in st["pipelines"] if p["id"] == int(parts[2]))
            p["status"] = {"retry": "pending", "cancel": "canceled"}[parts[3]]
            return self.send_json(201, p)
        if rest in ("/merge_requests", "/issues") and self.command == "GET":
            total = st["counts"][rest[1:]]
            headers = {} if total is None else {"X-Total": str(total), "X-Total-Pages": str(total)}
            return self.send_json(200, [{"iid": 1}] if total != 0 else [], headers)
        self.send_json(404, {"message": "404 Not found"})

    do_GET = do_POST = handle_any

    def log_message(self, *args):
        pass


class GitLabTest(unittest.TestCase):
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
            "pipelines": [pipeline(10, "success"), pipeline(8, "failed", "dev"), pipeline(12, "failed"),
                          pipeline(13, "running"), pipeline(9, "success", "dev")],
            "counts": {"merge_requests": 3, "issues": 12},
        }
        self.sent = []
        gitlab.plugin._send = self.sent.append
        gitlab.plugin.settings = {"url": self.base, "token": TOKEN}
        gitlab.plugin.watching = set()
        gitlab._reported.clear()
        patcher = mock.patch.object(gitlab, "refresh_soon", lambda: None)
        patcher.start()
        self.addCleanup(patcher.stop)

    def values(self):
        return {m["id"]: m["value"] for m in self.sent if m.get("event") == "value"}

    def posts(self):
        return [(c[1], c[3]) for c in Fake.state["calls"] if c[0] == "POST"]

    def test_values(self):
        gitlab.plugin.watching = {
            "pipeline:homelab/infra", "pipeline_running:homelab/infra", "pipeline_ok:homelab/infra",
            "pipeline:homelab/infra@dev", "pipeline_ok:homelab/infra@dev",
            "mrs:homelab/infra", "issues:homelab/infra", "pipeline:nobody/nothing",
        }
        gitlab.poll()
        v = self.values()
        self.assertEqual(v["pipeline:homelab/infra"], "running")
        self.assertEqual(v["pipeline_running:homelab/infra"], "on")
        self.assertEqual(v["pipeline_ok:homelab/infra"], "off")
        self.assertEqual(v["pipeline:homelab/infra@dev"], "success")
        self.assertEqual(v["pipeline_ok:homelab/infra@dev"], "on")
        self.assertNotIn("pipeline_running:homelab/infra@dev", v)  # not watched
        self.assertEqual(v["mrs:homelab/infra"], "3")
        self.assertEqual(v["issues:homelab/infra"], "12")
        self.assertEqual(v["pipeline:nobody/nothing"], "–")
        calls = Fake.state["calls"]
        self.assertTrue(all(c[4] == TOKEN for c in calls))
        # One pipelines call per project/branch, shared by the three pipeline values.
        self.assertEqual(sum(1 for c in calls if c[1].endswith("/pipelines")), 3)
        mr_call = next(c for c in calls if c[1].endswith("/merge_requests"))
        self.assertEqual(mr_call[2], {"state": "opened", "per_page": "1"})
        dev = next(c for c in calls if c[1].endswith("/pipelines") and c[2].get("ref") == "dev")
        self.assertEqual(dev[2], {"per_page": "1", "order_by": "id", "sort": "desc", "ref": "dev"})

    def test_counts_without_x_total(self):
        Fake.state["counts"] = {"merge_requests": None, "issues": 0}
        gitlab.plugin.watching = {"mrs:homelab/infra", "issues:homelab/infra"}
        gitlab.poll()
        self.assertEqual(self.values(), {"mrs:homelab/infra": "10000+", "issues:homelab/infra": "0"})

    def test_no_pipeline_yet(self):
        Fake.state["pipelines"] = [pipeline(1, "success", "other")]
        gitlab.plugin.watching = {"pipeline:homelab/infra@main", "pipeline_ok:homelab/infra@main"}
        gitlab.poll()
        self.assertEqual(self.values(), {"pipeline:homelab/infra@main": "none", "pipeline_ok:homelab/infra@main": "off"})

    def test_run_pipeline(self):
        self.assertIsNone(gitlab.run_pipeline({"project": "homelab/infra", "ref": "dev", "variables": "DEPLOY=yes; ENV = staging"}))
        path, body = self.posts()[-1]
        self.assertEqual(path, f"/api/v4/projects/{PROJECT}/pipeline")
        self.assertEqual(body, {"ref": "dev", "variables": [
            {"key": "DEPLOY", "value": "yes", "variable_type": "env_var"},
            {"key": "ENV", "value": "staging", "variable_type": "env_var"}]})
        # No branch: the project's default branch.
        self.assertIsNone(gitlab.run_pipeline({"project": "homelab/infra"}))
        self.assertEqual(self.posts()[-1][1], {"ref": "main"})
        # Branch in the project field.
        self.assertIsNone(gitlab.run_pipeline({"project": "homelab/infra@release/1.0"}))
        self.assertEqual(self.posts()[-1][1], {"ref": "release/1.0"})

    def test_variables(self):
        self.assertEqual(gitlab.parse_variables("A=1\nB=x=y"), [
            {"key": "A", "value": "1", "variable_type": "env_var"},
            {"key": "B", "value": "x=y", "variable_type": "env_var"}])
        self.assertEqual(gitlab.parse_variables(""), [])
        with self.assertRaisesRegex(RuntimeError, "KEY=VALUE"):
            gitlab.parse_variables("oops")

    def test_retry_and_cancel(self):
        self.assertIsNone(gitlab.retry_pipeline({"project": "homelab/infra"}))
        self.assertEqual(self.posts()[-1][0], f"/api/v4/projects/{PROJECT}/pipelines/12/retry")
        self.assertEqual(gitlab.retry_pipeline({"project": "homelab/infra", "ref": "main"}), "no failed pipeline to retry")
        self.assertIsNone(gitlab.cancel_pipelines({"project": "homelab/infra"}))
        cancelled = [p for p, _ in self.posts() if p.endswith("/cancel")]
        self.assertEqual(cancelled, [f"/api/v4/projects/{PROJECT}/pipelines/{i}/cancel" for i in (12, 13)])
        self.assertEqual(gitlab.cancel_pipelines({"project": "homelab/infra"}), "no pipeline is running")

    def test_cancel_answers_in_time_and_finishes_later(self):
        Fake.state["pipelines"] += [pipeline(20 + i, "running") for i in range(4)]
        Fake.state["slow_cancel"] = 0.15
        with mock.patch.object(gitlab, "ACTION_DEADLINE", 0.25):
            answer = gitlab.cancel_pipelines({"project": "homelab/infra"})
        self.assertRegex(answer, r"^still cancelling: \d of 5 done…$")
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and len([p for p, _ in self.posts() if p.endswith("/cancel")]) < 5:
            time.sleep(0.02)
        self.assertEqual(len([p for p, _ in self.posts() if p.endswith("/cancel")]), 5)

    def test_a_repeating_poll_error_is_logged_once(self):
        gitlab.plugin.watching = {"pipeline:nobody/nothing"}
        for _ in range(3):
            gitlab.poll()
        logs = [m for m in self.sent if m.get("event") == "log"]
        self.assertEqual(len(logs), 1, logs)
        self.assertIn("nobody/nothing", logs[0]["message"])

    def test_open_pipeline(self):
        with mock.patch.object(gitlab.webbrowser, "open", return_value=True) as browser:
            self.assertIsNone(gitlab.open_pipeline({"project": "homelab/infra", "ref": "dev"}))
            browser.assert_called_with("https://git.example/homelab/infra/-/pipelines/9")
            Fake.state["pipelines"] = []
            gitlab.open_pipeline({"project": "homelab/infra"})
            browser.assert_called_with(self.base + "/homelab/infra/-/pipelines")

    def test_errors(self):
        with self.assertRaisesRegex(RuntimeError, "project path"):
            gitlab.run_pipeline({"project": ""})
        with self.assertRaisesRegex(RuntimeError, "can't find that project"):
            gitlab.retry_pipeline({"project": "nobody/nothing"})
        Fake.state["read_only"] = True
        with self.assertRaisesRegex(RuntimeError, "api scope"):
            gitlab.run_pipeline({"project": "homelab/infra", "ref": "main"})
        gitlab.plugin.settings["token"] = "wrong"
        with self.assertRaisesRegex(RuntimeError, "refused the token"):
            gitlab.retry_pipeline({"project": "homelab/infra"})
        gitlab.plugin.settings = {"url": self.base}
        with self.assertRaisesRegex(RuntimeError, "access token"):
            gitlab.retry_pipeline({"project": "homelab/infra"})
        gitlab.plugin.settings = {"url": "http://127.0.0.1:9", "token": TOKEN}
        with self.assertRaisesRegex(RuntimeError, "couldn't reach GitLab at http://127.0.0.1:9"):
            gitlab.retry_pipeline({"project": "homelab/infra"})

    def test_unreachable_values_are_dashes_and_token_is_never_logged(self):
        gitlab.plugin.settings = {"url": "http://127.0.0.1:9", "token": TOKEN}
        gitlab.plugin.watching = {"pipeline:homelab/infra", "mrs:homelab/infra"}
        gitlab.poll()
        self.assertEqual(self.values(), {"pipeline:homelab/infra": "–", "mrs:homelab/infra": "–"})
        self.assertFalse(any(TOKEN in json.dumps(m) for m in self.sent))

    def test_settings(self):
        gitlab.plugin.settings = {"token": "x"}
        self.assertEqual(gitlab.config()[0], "https://gitlab.com")
        self.assertEqual(gitlab.interval(), gitlab.GITLAB_COM_POLL_SECONDS)
        gitlab.plugin.settings = {"url": "git.lan/api/v4", "token": "x", "insecure": True}
        self.assertEqual(gitlab.config(), ("https://git.lan", "x", True))
        self.assertEqual(gitlab.interval(), gitlab.POLL_SECONDS)

    def test_split_target(self):
        self.assertEqual(gitlab.split_target("a/b@feature/x"), ("a/b", "feature/x"))
        self.assertEqual(gitlab.split_target(" a/b/c "), ("a/b/c", None))
        self.assertEqual(gitlab.split_target("https://gitlab.com/a/b/-/pipelines"), ("a/b", None))

    def test_tick_only_polls_when_watched(self):
        with mock.patch.object(gitlab, "safe_poll") as poll:
            gitlab._last_poll["at"] = 0.0
            gitlab.tick()
            poll.assert_not_called()
            gitlab.plugin.watching = {"mrs:a/b"}
            gitlab.tick()
            poll.assert_called_once()


if __name__ == "__main__":
    unittest.main()
