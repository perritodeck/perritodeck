import copy
import http.server
import json
import os
import stat
import sys
import tempfile
import threading
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as hue  # noqa: E402

USER = "1028d66426293e821ecfd9ef1a0731df"

# Shapes as documented for the Hue API v1 (/api/<username>/lights, /groups, /scenes).
LIGHTS = {
    "1": {"state": {"on": True, "bri": 254, "hue": 8417, "sat": 140, "xy": [0.4573, 0.41], "ct": 366,
                    "colormode": "ct", "reachable": True},
          "type": "Extended color light", "name": "Desk lamp", "modelid": "LCT015",
          "capabilities": {"control": {"colorgamuttype": "C",
                                       "colorgamut": [[0.6915, 0.3083], [0.17, 0.7], [0.1532, 0.0475]],
                                       "ct": {"min": 153, "max": 500}}}},
    "2": {"state": {"on": False, "bri": 127, "ct": 300, "colormode": "ct", "reachable": True},
          "type": "Color temperature light", "name": "Sofa", "modelid": "LTW001",
          "capabilities": {"control": {"ct": {"min": 153, "max": 454}}}},
    "3": {"state": {"on": True, "bri": 1, "reachable": False},
          "type": "Dimmable light", "name": "Porch", "modelid": "LWB010", "capabilities": {"control": {}}},
}
GROUPS = {
    "1": {"name": "Living room", "lights": ["1", "2"], "type": "Room", "class": "Living room",
          "state": {"all_on": False, "any_on": True}, "action": {"on": True, "bri": 254}},
    "2": {"name": "Outside", "lights": ["3"], "type": "Room", "class": "Garden",
          "state": {"all_on": False, "any_on": False}, "action": {"on": False, "bri": 1}},
}
SCENES = {
    "aBc123": {"name": "Relax", "type": "GroupScene", "group": "1", "lights": ["1", "2"], "lastupdated": "2026-01-01T10:00:00"},
    "dEf456": {"name": "Relax", "type": "GroupScene", "group": "2", "lights": ["3"], "lastupdated": "2026-02-01T10:00:00"},
    "gHi789": {"name": "Movie", "type": "LightScene", "lights": ["1"], "lastupdated": "2026-03-01T10:00:00"},
}


class Bridge(http.server.BaseHTTPRequestHandler):
    calls = []
    lights = {}
    groups = {}
    link_pressed = False

    def body(self):
        n = int(self.headers.get("Content-Length") or 0)
        return json.loads(self.rfile.read(n)) if n else None

    def do_POST(self):
        body = self.body()
        Bridge.calls.append(("POST", self.path, body))
        if self.path == "/api":
            if not Bridge.link_pressed:
                self.reply([{"error": {"type": 101, "address": "", "description": "link button not pressed"}}])
            else:
                self.reply([{"success": {"username": USER}}])
        else:
            self.reply([], 404)

    def do_GET(self):
        Bridge.calls.append(("GET", self.path, None))
        parts = self.path.strip("/").split("/")
        if parts[:2] != ["api", USER]:
            self.reply([{"error": {"type": 1, "address": "/" + "/".join(parts[2:]), "description": "unauthorized user"}}])
        elif parts[2:] == ["lights"]:
            self.reply(Bridge.lights)
        elif parts[2:] == ["groups"]:
            self.reply(Bridge.groups)
        elif parts[2:] == ["scenes"]:
            self.reply(SCENES)
        else:
            self.reply([{"error": {"type": 3, "address": self.path, "description": "resource not available"}}])

    def do_PUT(self):
        body = self.body()
        Bridge.calls.append(("PUT", self.path, body))
        parts = self.path.strip("/").split("/")
        if parts[:2] != ["api", USER]:
            self.reply([{"error": {"type": 1, "address": "/", "description": "unauthorized user"}}])
            return
        if parts[2] == "lights" and parts[3] in Bridge.lights:
            Bridge.lights[parts[3]]["state"].update({k: v for k, v in body.items() if k in ("on", "bri", "xy", "ct")})
        elif parts[2] == "groups" and (parts[3] in Bridge.groups or parts[3] == "0"):
            pass
        else:
            self.reply([{"error": {"type": 3, "address": self.path, "description": "resource not available"}}])
            return
        base = "/" + "/".join(parts[2:])
        self.reply([{"success": {f"{base}/{k}": v}} for k, v in body.items()])

    def reply(self, body, code=200):
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


class Discovery(http.server.BaseHTTPRequestHandler):
    answer = []
    count = 0

    def do_GET(self):
        Discovery.count += 1
        data = json.dumps(Discovery.answer).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


def serve(handler):
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


class HueTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.bridge = serve(Bridge)
        cls.discovery = serve(Discovery)
        cls.address = f"127.0.0.1:{cls.bridge.server_port}"
        hue.DISCOVERY_URL = f"http://127.0.0.1:{cls.discovery.server_port}/"

    @classmethod
    def tearDownClass(cls):
        cls.bridge.shutdown()
        cls.discovery.shutdown()

    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        hue.plugin._data_dir = self.dir.name
        hue._store = None
        hue.forget()
        hue._published.clear()
        hue._last_error[0] = None
        self.sent = []
        hue.plugin._send = self.sent.append
        hue.plugin.settings = {"bridge": self.address}
        hue.plugin.watching = set()
        Bridge.calls = []
        Bridge.lights = copy.deepcopy(LIGHTS)
        Bridge.groups = copy.deepcopy(GROUPS)
        Bridge.link_pressed = False
        Discovery.answer = [{"id": "001788fffe6a1b2c", "internalipaddress": "127.0.0.1", "port": 443}]
        Discovery.count = 0
        poll_soon = mock.patch.object(hue, "poll_soon")
        poll_soon.start()
        self.addCleanup(poll_soon.stop)

    def tearDown(self):
        self.dir.cleanup()

    def paired(self):
        hue.save_store(username=USER)

    def puts(self):
        return [(path.replace(f"/api/{USER}", ""), body) for method, path, body in Bridge.calls if method == "PUT"]

    def values(self):
        return {m["id"]: m["value"] for m in self.sent if m.get("event") == "value"}

    # --- pairing ---

    def test_pair_needs_link_button(self):
        with self.assertRaisesRegex(hue.HueError, "link button"):
            hue.pair({})
        Bridge.link_pressed = True
        self.assertIsNone(hue.pair({}))
        self.assertEqual(Bridge.calls[-1][1], "/api")
        devicetype = Bridge.calls[-1][2]["devicetype"]
        self.assertTrue(devicetype.startswith("perritodeck#"))
        self.assertLessEqual(len(devicetype.split("#")[1]), 19)
        with open(os.path.join(self.dir.name, "hue.json"), encoding="utf-8") as f:
            self.assertEqual(json.load(f)["username"], USER)
        self.assertNotIn(USER, json.dumps(self.sent))  # never logged

    @unittest.skipUnless(os.name == "posix", "file modes are a Unix thing")
    def test_the_key_file_is_owner_only(self):
        # Even a .tmp left behind with a wide mode ends up owner-only.
        tmp = os.path.join(self.dir.name, "hue.json.tmp")
        with open(tmp, "w", encoding="utf-8") as f:
            f.write("{}")
        os.chmod(tmp, 0o644)
        self.paired()
        self.assertEqual(stat.S_IMODE(os.stat(os.path.join(self.dir.name, "hue.json")).st_mode), 0o600)

    def test_not_paired(self):
        with self.assertRaisesRegex(hue.HueError, "not paired"):
            hue.power({"light": "Sofa"})

    def test_forgotten_username(self):
        hue.save_store(username="stale")
        with self.assertRaisesRegex(hue.HueError, "Pair key"):
            hue.power({"light": "Sofa"})

    def test_discovery_once_and_cached(self):
        hue.plugin.settings = {}
        Discovery.answer = [{"id": "001788fffe6a1b2c", "internalipaddress": self.address, "port": 443}]
        self.paired()
        hue.power({"light": "Sofa", "mode": "on"})
        hue.forget()
        hue.power({"light": "Sofa", "mode": "off"})
        self.assertEqual(Discovery.count, 1)
        hue._store = None  # a restart reads the cached address from disk
        hue.forget()
        hue.power({"light": "Sofa", "mode": "on"})
        self.assertEqual(Discovery.count, 1)

    def test_discovery_finds_nothing(self):
        hue.plugin.settings = {}
        Discovery.answer = []
        self.paired()
        with self.assertRaisesRegex(hue.HueError, "no Hue Bridge found"):
            hue.power({"light": "Sofa"})

    def test_unreachable_bridge(self):
        hue.plugin.settings = {"bridge": "127.0.0.1:1"}
        self.paired()
        with self.assertRaisesRegex(hue.HueError, "couldn't reach the bridge at http://127.0.0.1:1"):
            hue.power({"light": "Sofa"})

    def test_https_setting(self):
        hue.plugin.settings = {"bridge": "192.168.1.20", "https": True}
        self.assertEqual(hue.bridge(), "https://192.168.1.20")
        self.assertEqual(hue.tls_context().verify_mode, hue.ssl.CERT_REQUIRED)
        hue.plugin.settings["insecure"] = True
        self.assertEqual(hue.tls_context().verify_mode, hue.ssl.CERT_NONE)
        err = hue.describe_unreachable("https://x", hue.ssl.SSLCertVerificationError("CERTIFICATE_VERIFY_FAILED"))
        self.assertIn("Skip certificate check", err)

    # --- actions ---

    def test_toggle_light_and_room(self):
        self.paired()
        hue.power({"light": "sofa"})  # off -> on, matched without case
        hue.power({"light": "Living room"})  # any_on -> off
        hue.power({"light": "Outside", "mode": "on"})
        hue.power({"light": "", "mode": "off"})
        self.assertEqual(self.puts(), [
            ("/lights/2/state", {"on": True}),
            ("/groups/1/action", {"on": False}),
            ("/groups/2/action", {"on": True}),
            ("/groups/0/action", {"on": False}),
        ])

    def test_unknown_light(self):
        self.paired()
        with self.assertRaisesRegex(hue.HueError, "no light or room called “Kitchen”"):
            hue.power({"light": "Kitchen"})

    def test_brightness_slider(self):
        self.paired()
        hue.brightness({"light": "Desk lamp", "value": 50})
        hue.brightness({"light": "Desk lamp", "value": 0})
        hue.brightness({"light": "Living room", "value": 100})
        self.assertEqual(self.puts(), [
            ("/lights/1/state", {"on": True, "bri": 127}),
            ("/lights/1/state", {"on": False}),
            ("/groups/1/action", {"on": True, "bri": 254}),
        ])
        with self.assertRaisesRegex(hue.HueError, "0 to 100"):
            hue.brightness({"light": "Desk lamp", "value": "lots"})

    def test_brightness_up_down(self):
        self.paired()
        hue.brightness_up({"light": "Sofa", "step": 20})
        hue.brightness_down({"light": "Desk lamp"})
        self.assertEqual(self.puts(), [
            ("/lights/2/state", {"on": True, "bri_inc": 51}),
            ("/lights/1/state", {"bri_inc": -25}),
        ])
        hue.forget()
        Bridge.lights["2"]["state"]["on"] = False
        self.assertEqual(hue.brightness_down({"light": "Sofa"}), "it's already off")

    def test_colour(self):
        self.paired()
        hue.color({"light": "Desk lamp", "color": "#ff0000"})
        path, body = self.puts()[-1]
        self.assertEqual(path, "/lights/1/state")
        self.assertTrue(body["on"])
        # pure red is outside gamut C and pulled onto its red corner
        self.assertAlmostEqual(body["xy"][0], 0.6915, places=2)
        self.assertAlmostEqual(body["xy"][1], 0.3083, places=2)
        hue.color({"light": "Living room", "color": "fff"})
        _, body = self.puts()[-1]
        self.assertAlmostEqual(body["xy"][0], 0.3227, places=3)  # D65 white per the Hue formula
        self.assertAlmostEqual(body["xy"][1], 0.329, places=3)
        with self.assertRaisesRegex(hue.HueError, "isn't a colour"):
            hue.color({"light": "Desk lamp", "color": "orange"})
        with self.assertRaisesRegex(hue.HueError, "black"):
            hue.color({"light": "Desk lamp", "color": "#000000"})

    def test_gamut_math(self):
        gamut = hue.GAMUTS["C"]
        self.assertTrue(hue.in_gamut((0.3, 0.3), gamut))
        self.assertFalse(hue.in_gamut((0.0, 0.0), gamut))
        x, y = hue.into_gamut((0.8, 0.2), gamut)
        cx, cy = sum(p[0] for p in gamut) / 3, sum(p[1] for p in gamut) / 3
        self.assertTrue(hue.in_gamut((x + (cx - x) * 1e-6, y + (cy - y) * 1e-6), gamut))  # on the edge
        self.assertLess(abs(x - 0.6915) + abs(y - 0.3083), 0.15)

    def test_temperature(self):
        self.paired()
        hue.temperature({"light": "Sofa", "kelvin": 2700})
        hue.temperature({"light": "Sofa", "kelvin": 1000})  # clamped to the light's warmest (454)
        hue.temperature({"light": "Living room", "kelvin": 6500})
        self.assertEqual(self.puts(), [
            ("/lights/2/state", {"on": True, "ct": 370}),
            ("/lights/2/state", {"on": True, "ct": 454}),
            ("/groups/1/action", {"on": True, "ct": 154}),
        ])
        with self.assertRaisesRegex(hue.HueError, "kelvin"):
            hue.temperature({"light": "Sofa", "kelvin": -5})

    def test_scene_in_room(self):
        self.paired()
        self.assertIsNone(hue.scene({"room": "Living room", "scene": "relax"}))
        self.assertEqual(self.puts(), [("/groups/1/action", {"scene": "aBc123"})])

    def test_scene_light_scene_and_ambiguous(self):
        self.paired()
        hue.scene({"room": "Living room", "scene": "Movie"})  # LightScene: lights overlap the room
        self.assertEqual(self.puts()[-1], ("/groups/1/action", {"scene": "gHi789"}))
        hue.scene({"scene": "Movie"})
        self.assertEqual(self.puts()[-1], ("/groups/0/action", {"scene": "gHi789"}))
        warning = hue.scene({"scene": "Relax"})
        self.assertIn("2 scenes", warning)
        self.assertEqual(self.puts()[-1], ("/groups/2/action", {"scene": "dEf456"}))  # newest

    def test_scene_errors(self):
        self.paired()
        with self.assertRaisesRegex(hue.HueError, "no scene called “Movie” in Outside"):
            hue.scene({"room": "Outside", "scene": "Movie"})
        with self.assertRaisesRegex(hue.HueError, "no room called"):
            hue.scene({"room": "Attic", "scene": "Relax"})
        with self.assertRaisesRegex(hue.HueError, "scene's name"):
            hue.scene({"room": "Attic"})

    # --- values ---

    def test_values_for_watched_only(self):
        self.paired()
        hue.plugin.watching = {"on:Desk lamp", "brightness:Desk lamp", "on:Sofa", "brightness:Sofa",
                               "on:Living room", "brightness:Living room", "on:Porch", "on:Outside",
                               "on:all", "brightness:all", "on:Nowhere"}
        hue.poll()
        self.assertEqual(self.values(), {
            "on:Desk lamp": "on", "brightness:Desk lamp": "100",
            "on:Sofa": "off", "brightness:Sofa": "0",
            "on:Living room": "on", "brightness:Living room": "100",
            "on:Porch": "–", "on:Outside": "off",
            "on:all": "on", "brightness:all": "100",
            "on:Nowhere": "–",
        })
        self.sent.clear()
        hue.poll()
        self.assertEqual(self.values(), {})  # nothing changed, nothing sent

    def test_no_requests_when_nothing_watched(self):
        self.paired()
        hue.poll()
        self.assertEqual(Bridge.calls, [])

    def test_values_follow_changes_and_errors(self):
        self.paired()
        hue.plugin.watching = {"on:Sofa", "brightness:Sofa"}
        hue.poll()
        hue.brightness({"light": "Sofa", "value": 50})
        self.sent.clear()
        hue.poll()
        self.assertEqual(self.values(), {"on:Sofa": "on", "brightness:Sofa": "50"})
        hue.plugin.settings = {"bridge": "127.0.0.1:1"}
        self.sent.clear()
        hue.poll()
        self.assertEqual(self.values(), {"on:Sofa": "–", "brightness:Sofa": "–"})
        logs = [m for m in self.sent if m.get("event") == "log"]
        self.assertEqual(len(logs), 1)

    def test_watch_change_forgets(self):
        self.paired()
        hue.plugin.watching = {"on:Sofa"}
        hue.poll()
        hue.watching([])
        self.assertEqual(hue._published, {})

    def test_manifest(self):
        with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "perritodeck-plugin.json"), encoding="utf-8") as f:
            manifest = json.load(f)
        self.assertEqual(manifest["id"], "dev.perritodeck.hue")
        actions = {a["id"] for a in manifest["actions"]}
        self.assertEqual(actions, set(hue.plugin._actions))


if __name__ == "__main__":
    unittest.main()
