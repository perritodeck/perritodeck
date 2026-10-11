import http.server
import json
import os
import ssl
import sys
import threading
import unittest
import urllib.parse
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as pve  # noqa: E402

TOKEN = "PVEAPIToken=root@pam!deck=11111111-2222-3333-4444-555555555555"


def fresh_resources():
    return [
        {"id": "node/pve1", "type": "node", "node": "pve1", "status": "online",
         "cpu": 0.123, "maxcpu": 8, "mem": 4 * 2**30, "maxmem": 16 * 2**30, "uptime": 100000},
        {"id": "node/pve2", "type": "node", "node": "pve2", "status": "offline"},
        {"id": "qemu/101", "type": "qemu", "vmid": 101, "node": "pve1", "name": "homeassistant",
         "status": "running", "cpu": 0.256, "maxcpu": 2, "mem": 1 * 2**30, "maxmem": 4 * 2**30,
         "uptime": 90061, "template": 0},
        {"id": "lxc/200", "type": "lxc", "vmid": 200, "node": "pve1", "name": "pihole",
         "status": "stopped", "cpu": 0, "maxcpu": 1, "mem": 0, "maxmem": 512 * 2**20, "uptime": 0},
        {"id": "qemu/9000", "type": "qemu", "vmid": 9000, "node": "pve1", "name": "tmpl",
         "status": "stopped", "template": 1},
        {"id": "storage/pve1/local", "type": "storage", "node": "pve1", "storage": "local", "status": "available"},
    ]


class Fake(http.server.BaseHTTPRequestHandler):
    state = {}

    def reply(self, code, data=None, reason=None):
        body = json.dumps({"data": data}).encode()
        self.send_response(code, reason)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def handle_any(self):
        st = Fake.state
        length = int(self.headers.get("Content-Length") or 0)
        form = urllib.parse.parse_qs(self.rfile.read(length).decode()) if length else {}
        st["calls"].append((self.command, self.path, self.headers.get("Authorization"), form))
        if self.headers.get("Authorization") != TOKEN:
            return self.reply(401, reason="authentication failure")
        path = urllib.parse.urlparse(self.path).path
        if path == "/api2/json/cluster/resources" and self.command == "GET":
            return self.reply(200, st["resources"])
        parts = path.split("/")
        # /api2/json/nodes/<node>/<type>/<vmid>/...
        if len(parts) >= 7 and parts[3] == "nodes":
            node, kind, vmid = parts[4], parts[5], int(parts[6])
            guest = next(r for r in st["resources"] if r.get("vmid") == vmid)
            if (node, kind) != (guest["node"], guest["type"]):
                return self.reply(500, reason="wrong node or type")
            if parts[7:] == ["status", "current"]:
                return self.reply(200, {"status": guest["status"], "qmpstatus": st.get("qmp", guest["status"])})
            if parts[7:8] == ["status"] and self.command == "POST":
                if vmid in st.get("forbid", ()):
                    return self.reply(403, reason=f"Permission check failed (/vms/{vmid}, VM.PowerMgmt)")
                command = parts[8]
                guest["status"] = {"start": "running", "resume": "running"}.get(command, "stopped")
                return self.reply(200, f"UPID:{node}:0000:qm{command}:{vmid}:root@pam!deck:")
            if parts[7:] == ["snapshot"] and self.command == "POST":
                return self.reply(200, f"UPID:{node}:0000:qmsnapshot:{vmid}:root@pam!deck:")
        self.reply(501, reason="Method not implemented")

    do_GET = do_POST = handle_any

    def log_message(self, *args):
        pass


class ProxmoxTest(unittest.TestCase):
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
        Fake.state = {"resources": fresh_resources(), "calls": []}
        self.sent = []
        pve.plugin._send = self.sent.append
        pve.plugin.settings = {
            "url": self.base,
            "token_id": "root@pam!deck",
            "token_secret": "11111111-2222-3333-4444-555555555555",
        }
        pve.plugin.watching = set()
        pve._reported.clear()
        pve.forget()
        # Don't start refresh timers from actions in tests.
        patcher = mock.patch.object(pve, "refresh_soon", pve.forget)
        patcher.start()
        self.addCleanup(patcher.stop)

    def values(self):
        return {m["id"]: m["value"] for m in self.sent if m.get("event") == "value"}

    def posts(self):
        return [c[1] for c in Fake.state["calls"] if c[0] == "POST"]

    def test_auth_header_and_actions_find_the_node(self):
        self.assertIsNone(pve.shutdown({"vmid": "101"}))
        self.assertEqual(self.posts(), ["/api2/json/nodes/pve1/qemu/101/status/shutdown"])
        self.assertTrue(all(c[2] == TOKEN for c in Fake.state["calls"]))
        pve.forget()
        self.assertIsNone(pve.start({"vmid": " 200 "}))
        self.assertEqual(self.posts()[-1], "/api2/json/nodes/pve1/lxc/200/status/start")

    def test_every_power_command(self):
        for fn, command in ((pve.stop, "stop"), (pve.reboot, "reboot"), (pve.suspend, "suspend")):
            Fake.state["resources"] = fresh_resources()
            pve.forget()
            self.assertIsNone(fn({"vmid": "101"}))
            self.assertEqual(self.posts()[-1], f"/api2/json/nodes/pve1/qemu/101/status/{command}")
        Fake.state["resources"] = fresh_resources()
        pve.forget()
        self.assertIsNone(pve.resume({"vmid": "101"}))
        self.assertEqual(self.posts()[-1], "/api2/json/nodes/pve1/qemu/101/status/resume")

    def test_toggle(self):
        self.assertIsNone(pve.toggle({"vmid": "101"}))  # running -> shutdown
        self.assertTrue(self.posts()[-1].endswith("/qemu/101/status/shutdown"))
        pve.forget()
        self.assertIsNone(pve.toggle({"vmid": "101"}))  # now stopped -> start
        self.assertTrue(self.posts()[-1].endswith("/qemu/101/status/start"))

    def test_warnings_for_no_ops(self):
        self.assertIn("already running", pve.start({"vmid": "101"}))
        self.assertIn("already stopped", pve.shutdown({"vmid": "200"}))
        self.assertEqual(self.posts(), [])

    def test_errors(self):
        with self.assertRaisesRegex(RuntimeError, "number like 101"):
            pve.start({"vmid": "abc"})
        with self.assertRaisesRegex(RuntimeError, "no VM or container 555"):
            pve.start({"vmid": "555"})
        with self.assertRaisesRegex(RuntimeError, "only work for VMs"):
            pve.suspend({"vmid": "200"})
        Fake.state["forbid"] = {101}
        with self.assertRaisesRegex(RuntimeError, "isn't allowed.*VM.PowerMgmt"):
            pve.stop({"vmid": "101"})
        pve.plugin.settings["token_secret"] = "wrong"
        pve.forget()
        with self.assertRaisesRegex(RuntimeError, "refused the token"):
            pve.stop({"vmid": "101"})
        pve.plugin.settings = {"url": "http://127.0.0.1:9", "token_id": "a@pam!b", "token_secret": "x"}
        pve.forget()
        with self.assertRaisesRegex(RuntimeError, "couldn't reach Proxmox at http://127.0.0.1:9"):
            pve.stop({"vmid": "101"})

    def test_settings_are_checked(self):
        pve.plugin.settings = {}
        with self.assertRaisesRegex(RuntimeError, "Proxmox address"):
            pve.start({"vmid": "101"})
        pve.plugin.settings = {"url": "pve.local:8006", "token_id": "nobang", "token_secret": "x"}
        with self.assertRaisesRegex(RuntimeError, "user@realm!name"):
            pve.start({"vmid": "101"})
        pve.plugin.settings = {"url": "pve.local:8006/api2/json/", "token_id": "a@pve!b", "token_secret": "x"}
        self.assertEqual(pve.config()[0], "https://pve.local:8006")

    def test_snapshot(self):
        self.assertIsNone(pve.snapshot({"vmid": "101", "name": "before_update"}))
        call = Fake.state["calls"][-1]
        self.assertEqual(call[1], "/api2/json/nodes/pve1/qemu/101/snapshot")
        self.assertEqual(call[3]["snapname"], ["before_update"])
        self.assertIsNone(pve.snapshot({"vmid": "200"}))
        self.assertRegex(Fake.state["calls"][-1][3]["snapname"][0], r"^deck_\d{8}_\d{6}$")
        with self.assertRaisesRegex(RuntimeError, "start with a letter"):
            pve.snapshot({"vmid": "101", "name": "1 bad name"})

    def test_values_publish_only_watched(self):
        pve.plugin.watching = {
            "status:101", "running:101", "cpu:101", "mem:101", "uptime:101", "name:101",
            "status:200", "running:200", "cpu:200", "uptime:200", "status:404",
            "node_cpu:pve1", "node_mem:pve1", "node_status:pve1", "node_cpu:pve2", "node_status:pve2",
        }
        pve.poll()
        v = self.values()
        self.assertEqual(v["status:101"], "running")
        self.assertEqual(v["running:101"], "on")
        self.assertEqual(v["cpu:101"], "26")
        self.assertEqual(v["mem:101"], "25")
        self.assertEqual(v["uptime:101"], "1d 1h")
        self.assertEqual(v["name:101"], "homeassistant")
        self.assertEqual((v["status:200"], v["running:200"], v["cpu:200"], v["uptime:200"]), ("stopped", "off", "0", "–"))
        self.assertEqual(v["status:404"], "–")
        self.assertEqual((v["node_cpu:pve1"], v["node_mem:pve1"], v["node_status:pve1"]), ("12", "25", "online"))
        self.assertEqual((v["node_cpu:pve2"], v["node_status:pve2"]), ("–", "offline"))
        self.assertEqual(v["guests_running"], "1")  # templates don't count
        self.assertNotIn("mem:200", v)
        gets = [c[1] for c in Fake.state["calls"] if c[0] == "GET"]
        # One resources call, plus status/current for the running VM whose status is shown.
        self.assertEqual(gets, ["/api2/json/cluster/resources", "/api2/json/nodes/pve1/qemu/101/status/current"])

    def test_paused_vm(self):
        Fake.state["qmp"] = "paused"
        pve.plugin.watching = {"status:101"}
        pve.poll()
        self.assertEqual(self.values()["status:101"], "paused")

    def test_unreachable_publishes_dashes(self):
        pve.plugin.settings["url"] = "http://127.0.0.1:9"
        pve.plugin.watching = {"status:101", "node_cpu:pve1"}
        pve.poll()
        self.assertEqual(self.values(), {"status:101": "–", "node_cpu:pve1": "–", "guests_running": "–"})
        self.assertTrue(any(m.get("event") == "log" for m in self.sent))
        self.assertFalse(any("5555" in json.dumps(m) for m in self.sent))  # no secret in logs

    def test_a_repeating_error_is_logged_once(self):
        good = pve.plugin.settings["url"]
        pve.plugin.settings["url"] = "http://127.0.0.1:9"
        pve.plugin.watching = {"status:101"}
        for _ in range(3):
            pve.forget()
            pve.poll()
        logs = [m for m in self.sent if m.get("event") == "log"]
        self.assertEqual(len(logs), 1, logs)
        pve.plugin.settings["url"] = good
        pve.forget()
        pve.poll()
        logs = [m for m in self.sent if m.get("event") == "log"]
        self.assertEqual(len(logs), 2)
        self.assertIn("working again", logs[-1]["message"])

    def test_resources_are_cached_for_actions(self):
        pve.resources()
        pve.find_guest(101)
        pve.find_guest(200)
        self.assertEqual(len(Fake.state["calls"]), 1)

    def test_helpers(self):
        self.assertEqual(pve.human_uptime(59), "0m")
        self.assertEqual(pve.human_uptime(3700), "1h 1m")
        self.assertEqual(pve.human_uptime(0), "–")
        self.assertEqual(pve.percent(1, 0), "–")
        ctx = pve.tls_context(True)
        self.assertEqual(ctx.verify_mode, ssl.CERT_NONE)
        self.assertEqual(pve.tls_context(False).verify_mode, ssl.CERT_REQUIRED)
        self.assertIn("Skip certificate check", pve.describe_url_error("https://x", ssl.SSLCertVerificationError("bad")))

    def test_watch_change_polls(self):
        with mock.patch.object(pve, "safe_poll") as poll:
            pve.watch_changed(["status:101"])
            for _ in range(50):
                if poll.called:
                    break
                threading.Event().wait(0.01)
        poll.assert_called()


if __name__ == "__main__":
    unittest.main()
