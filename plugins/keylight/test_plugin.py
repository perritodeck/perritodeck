import http.server
import json
import os
import sys
import threading
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as keylight  # noqa: E402


def make_light(state, info):
    """A fake light speaking Elgato's documented /elgato/lights and /elgato/accessory-info."""

    class Light(http.server.BaseHTTPRequestHandler):
        calls = []

        def do_GET(self):
            Light.calls.append(("GET", self.path, None))
            if self.path == "/elgato/lights":
                self.reply({"numberOfLights": 1, "lights": [Light.state]})
            elif self.path == "/elgato/accessory-info":
                self.reply(info)
            else:
                self.reply({}, 404)

        def do_PUT(self):
            n = int(self.headers.get("Content-Length") or 0)
            body = json.loads(self.rfile.read(n))
            Light.calls.append(("PUT", self.path, body))
            if self.path != "/elgato/lights" or self.headers.get("Content-Type") != "application/json":
                self.reply({}, 400)
                return
            for key, value in body["lights"][0].items():
                if key in Light.state:
                    Light.state[key] = value
            self.reply({"numberOfLights": 1, "lights": [Light.state]})

        def reply(self, body, code=200):
            data = json.dumps(body).encode()
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def log_message(self, *args):
            pass

    Light.initial = dict(state)
    Light.state = dict(state)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Light)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, Light


KEY_INFO = {"productName": "Elgato Key Light", "hardwareBoardType": 53, "firmwareBuildNumber": 218,
            "firmwareVersion": "1.0.3", "serialNumber": "BW33J1A00000", "displayName": "Left key"}
STRIP_INFO = {"productName": "Elgato Light Strip", "hardwareBoardType": 70, "firmwareVersion": "1.0.4",
              "serialNumber": "EW52J1A00000", "displayName": "Shelf"}


class KeyLightTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.left, cls.Left = make_light({"on": 1, "brightness": 20, "temperature": 213}, KEY_INFO)
        cls.right, cls.Right = make_light({"on": 0, "brightness": 50, "temperature": 300},
                                          dict(KEY_INFO, displayName="Right key"))
        cls.strip, cls.Strip = make_light({"on": 1, "hue": 40.0, "saturation": 77.0, "brightness": 60}, STRIP_INFO)
        cls.a = f"127.0.0.1:{cls.left.server_port}"
        cls.b = f"127.0.0.1:{cls.right.server_port}"
        cls.c = f"127.0.0.1:{cls.strip.server_port}"

    @classmethod
    def tearDownClass(cls):
        for s in (cls.left, cls.right, cls.strip):
            s.shutdown()

    def setUp(self):
        for light in (self.Left, self.Right, self.Strip):
            light.state = dict(light.initial)
            light.calls = []
        keylight.plugin.settings = {"lights": f"{self.a}, {self.b}"}
        keylight.plugin.watching = set()
        keylight._names.clear()
        keylight._published.clear()
        keylight._last_error.clear()
        self.sent = []
        keylight.plugin._send = self.sent.append

    def puts(self, light):
        return [body["lights"][0] for method, _, body in light.calls if method == "PUT"]

    def values(self):
        return {m["id"]: m["value"] for m in self.sent if m.get("event") == "value"}

    # --- settings and names ---

    def test_addresses(self):
        keylight.plugin.settings = {"lights": "192.168.1.50, key-left.local:9124;http://10.0.0.7/ ,, fe80::1, 192.168.1.50"}
        self.assertEqual(keylight.addresses(),
                         ["192.168.1.50:9123", "key-left.local:9124", "10.0.0.7:9123", "[fe80::1]:9123"])

    def test_pick(self):
        self.assertEqual(keylight.pick(""), [self.a, self.b])
        self.assertEqual(keylight.pick("all"), [self.a, self.b])
        self.assertEqual(keylight.pick(self.b), [self.b])
        self.assertEqual(keylight.pick("right key"), [self.b])  # its name in Control Center
        self.assertEqual(keylight.pick("10.0.0.9"), ["10.0.0.9:9123"])  # not listed, used as is
        with self.assertRaisesRegex(keylight.LightError, "no light called “Ceiling”"):
            keylight.pick("Ceiling")
        keylight.plugin.settings = {}
        with self.assertRaisesRegex(keylight.LightError, "Plugins page"):
            keylight.pick("")

    # --- actions ---

    def test_toggle_all_and_one(self):
        keylight.power({})  # left is on: everything goes off
        self.assertEqual(self.puts(self.Left), [{"on": 0}])
        self.assertEqual(self.puts(self.Right), [{"on": 0}])
        keylight.power({"light": self.b, "mode": "toggle"})
        self.assertEqual(self.Right.state["on"], 1)
        keylight.power({"light": self.a, "mode": "on"})
        keylight.power({"light": self.a, "mode": "off"})
        self.assertEqual(self.puts(self.Left)[-2:], [{"on": 1}, {"on": 0}])

    def test_put_body_shape(self):
        keylight.power({"light": self.a, "mode": "on"})
        put = [body for method, _, body in self.Left.calls if method == "PUT"][0]
        self.assertEqual(put, {"numberOfLights": 1, "lights": [{"on": 1}]})

    def test_brightness_slider(self):
        keylight.brightness({"light": self.a, "value": 75})
        keylight.brightness({"light": self.a, "value": 0})
        self.assertEqual(self.puts(self.Left), [{"on": 1, "brightness": 75}, {"on": 0}])

    def test_brightness_up_down(self):
        keylight.brightness_up({"light": self.a, "step": 15})
        self.assertEqual(self.Left.state["brightness"], 35)
        keylight.brightness_down({"light": self.a, "step": 50})
        self.assertEqual(self.Left.state["brightness"], 1)
        self.assertEqual(keylight.brightness_down({"light": self.b}), "it's already off")
        keylight.brightness_up({"light": self.b})  # off: comes on at 10 %
        self.assertEqual(self.puts(self.Right)[-1], {"on": 1, "brightness": 10})

    def test_temperature_slider(self):
        keylight.temperature({"light": self.a, "value": 0})
        keylight.temperature({"light": self.a, "value": 100})
        keylight.temperature({"light": self.a, "value": 50})
        self.assertEqual(self.puts(self.Left), [
            {"on": 1, "temperature": 344},  # 2900 K
            {"on": 1, "temperature": 143},  # 7000 K
            {"on": 1, "temperature": 202},  # 4950 K
        ])

    def test_warmer_cooler(self):
        keylight.warmer({"light": self.a})  # 213 is ~4695 K, so 4395 K
        self.assertEqual(self.Left.state["temperature"], 228)
        keylight.cooler({"light": self.a, "step": 10000})
        self.assertEqual(self.Left.state["temperature"], 143)
        keylight.warmer({"light": self.a, "step": 10000})
        self.assertEqual(self.Left.state["temperature"], 344)

    def test_light_strip_has_no_temperature(self):
        keylight.plugin.settings = {"lights": self.c}
        self.assertIn("colours", keylight.temperature({"value": 30}))
        self.assertEqual(self.puts(self.Strip), [])
        keylight.brightness({"value": 30})
        self.assertEqual(self.puts(self.Strip), [{"on": 1, "brightness": 30}])

    def test_unreachable(self):
        keylight.plugin.settings = {"lights": f"{self.a}, 127.0.0.1:1"}
        with self.assertRaisesRegex(keylight.LightError, "couldn't reach the light at 127.0.0.1:1"):
            keylight.power({"mode": "on"})
        self.assertEqual(self.Left.state["on"], 1)  # the reachable one still did it

    def test_bad_numbers(self):
        with self.assertRaisesRegex(keylight.LightError, "isn't a number"):
            keylight.brightness_up({"light": self.a, "step": "lots"})

    # --- values ---

    def test_values(self):
        keylight.plugin.settings = {"lights": f"{self.a}, {self.b}, {self.c}"}
        keylight.plugin.watching = {f"on:{self.a}", f"brightness:{self.a}", f"temperature:{self.a}",
                                    f"brightness:{self.b}", f"temperature:{self.c}", "on:all",
                                    "on:Left key", "on:Ceiling"}
        keylight.poll()
        self.assertEqual(self.values(), {
            f"on:{self.a}": "on", f"brightness:{self.a}": "20", f"temperature:{self.a}": "4700 K",
            f"brightness:{self.b}": "0", f"temperature:{self.c}": "–",
            "on:all": "on", "on:Left key": "on", "on:Ceiling": "–",
        })
        self.sent.clear()
        keylight.poll()
        self.assertEqual(self.values(), {})

    def test_values_update_after_action(self):
        keylight.plugin.watching = {f"on:{self.b}", f"brightness:{self.b}"}
        keylight.poll()
        self.sent.clear()
        keylight.brightness({"light": self.b, "value": 40})
        self.assertEqual(self.values(), {f"on:{self.b}": "on", f"brightness:{self.b}": "40"})

    def test_unreachable_value_logs_once(self):
        keylight.plugin.settings = {"lights": "127.0.0.1:1"}
        keylight.plugin.watching = {"on:127.0.0.1:1"}
        keylight.poll()
        keylight.poll()
        self.assertEqual(self.values(), {"on:127.0.0.1:1": "–"})
        self.assertEqual(len([m for m in self.sent if m.get("event") == "log"]), 1)

    def test_nothing_watched_no_requests(self):
        keylight.poll()
        self.assertEqual(self.Left.calls + self.Right.calls, [])

    def test_conversions(self):
        self.assertEqual(keylight.kelvin_to_api(2900), 344)
        self.assertEqual(keylight.kelvin_to_api(7000), 143)
        self.assertEqual(keylight.kelvin_to_api(20000), 143)
        self.assertEqual(keylight.api_to_kelvin(143), 7000)
        self.assertEqual(keylight.api_to_kelvin(344), 2900)

    def test_manifest(self):
        with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "perritodeck-plugin.json"), encoding="utf-8") as f:
            manifest = json.load(f)
        self.assertEqual(manifest["id"], "dev.perritodeck.keylight")
        self.assertEqual({a["id"] for a in manifest["actions"]}, set(keylight.plugin._actions))
        self.assertEqual({v["id"] for v in manifest["values"]}, set(keylight.VALUES))


if __name__ == "__main__":
    unittest.main()
