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
import plugin as weather  # noqa: E402

# Shapes as documented by Open-Meteo (geocoding: /v1/search; forecast: /v1/forecast).
GEO = {
    "lisbon": [
        {"id": 2267057, "name": "Lisbon", "latitude": 38.71667, "longitude": -9.13333, "elevation": 45.0,
         "country_code": "PT", "country": "Portugal", "admin1": "Lisbon", "timezone": "Europe/Lisbon"},
    ],
    "paris": [
        {"id": 2988507, "name": "Paris", "latitude": 48.85341, "longitude": 2.3488, "country_code": "FR",
         "country": "France", "admin1": "Île-de-France", "timezone": "Europe/Paris"},
        {"id": 4717560, "name": "Paris", "latitude": 33.66094, "longitude": -95.55551, "country_code": "US",
         "country": "United States", "admin1": "Texas", "timezone": "America/Chicago"},
    ],
}


def forecast(query):
    imperial = query.get("temperature_unit") == ["fahrenheit"]
    return {
        "latitude": float(query["latitude"][0]),
        "longitude": float(query["longitude"][0]),
        "generationtime_ms": 0.1,
        "utc_offset_seconds": 3600,
        "timezone": "Europe/Lisbon",
        "timezone_abbreviation": "WEST",
        "elevation": 45.0,
        "current_units": {"time": "iso8601", "interval": "seconds", "temperature_2m": "°F" if imperial else "°C",
                          "apparent_temperature": "°F" if imperial else "°C", "relative_humidity_2m": "%",
                          "weather_code": "wmo code", "wind_speed_10m": "mp/h" if imperial else "km/h", "is_day": ""},
        "current": {"time": "2026-10-11T14:00", "interval": 900, "temperature_2m": 64.6 if imperial else 18.2,
                    "apparent_temperature": 61.0 if imperial else 16.4, "relative_humidity_2m": 72,
                    "weather_code": 61, "wind_speed_10m": 7.8 if imperial else 12.6, "is_day": 1},
        "daily_units": {"time": "iso8601", "temperature_2m_max": "°C", "temperature_2m_min": "°C",
                        "precipitation_probability_max": "%"},
        "daily": {"time": ["2026-10-11"], "temperature_2m_max": [70.3 if imperial else 21.3],
                  "temperature_2m_min": [55.0 if imperial else 12.8], "precipitation_probability_max": [40]},
    }


class Handler(http.server.BaseHTTPRequestHandler):
    seen = []

    def do_GET(self):
        url = urllib.parse.urlparse(self.path)
        query = urllib.parse.parse_qs(url.query)
        Handler.seen.append((url.path, query, self.headers.get("User-Agent")))
        if url.path == "/v1/search":
            body = {"generationtime_ms": 0.5}
            results = GEO.get(query["name"][0].lower())
            if results:
                body["results"] = results[: int(query["count"][0])]
            self.reply(200, body)
        elif url.path == "/v1/forecast":
            if float(query["latitude"][0]) > 90:
                self.reply(400, {"error": True, "reason": "Latitude must be in range of -90 to 90°."})
            elif query["latitude"][0].startswith("1.0"):
                self.reply(500, {})
            else:
                self.reply(200, forecast(query))
        else:
            self.reply(404, {})

    def reply(self, code, body):
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


class WeatherTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
        base = f"http://127.0.0.1:{cls.server.server_port}"
        weather.FORECAST_URL = base + "/v1/forecast"
        weather.GEOCODING_URL = base + "/v1/search"

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()

    def setUp(self):
        self.sent = []
        weather.plugin._send = self.sent.append
        weather.plugin.settings = {}
        weather.plugin.watching = set()
        weather._cache.clear()
        weather._geo.clear()
        Handler.seen.clear()

    def published(self):
        return {m["id"]: m["value"] for m in self.sent if m.get("event") == "value"}

    def watch(self, place, values=weather.VALUES):
        weather.plugin.watching = {f"{v}:{place}" for v in values}

    def test_values_for_a_place_name(self):
        self.watch("Lisbon")
        weather.poll(now=0)
        self.assertEqual(
            self.published(),
            {"temp:Lisbon": "18°C", "feels:Lisbon": "16°C", "condition:Lisbon": "Light rain", "icon:Lisbon": "🌦️",
             "humidity:Lisbon": "72%", "wind:Lisbon": "13 km/h", "high:Lisbon": "21°C", "low:Lisbon": "13°C",
             "rain_chance:Lisbon": "40%"},
        )
        geo, fc = Handler.seen
        self.assertEqual(geo[0], "/v1/search")
        self.assertEqual(geo[1], {"name": ["Lisbon"], "count": ["1"], "language": ["en"], "format": ["json"]})
        self.assertEqual(fc[0], "/v1/forecast")
        q = fc[1]
        self.assertEqual((q["latitude"], q["longitude"]), (["38.7167"], ["-9.1333"]))
        self.assertEqual(q["current"][0].split(","), ["temperature_2m", "apparent_temperature", "relative_humidity_2m",
                                                      "weather_code", "wind_speed_10m", "is_day"])
        self.assertEqual(q["daily"][0].split(","), ["temperature_2m_max", "temperature_2m_min",
                                                    "precipitation_probability_max"])
        self.assertEqual((q["timezone"], q["forecast_days"], q["wind_speed_unit"]), (["auto"], ["1"], ["kmh"]))
        self.assertNotIn("temperature_unit", q)
        self.assertIn("PerritoDeck", fc[2])

    def test_coordinates_skip_geocoding_and_imperial_units(self):
        weather.plugin.settings = {"units": "imperial"}
        self.watch("38.72, -9.14", ["temp", "wind", "low"])
        weather.poll(now=0)
        self.assertEqual([s[0] for s in Handler.seen], ["/v1/forecast"])
        q = Handler.seen[0][1]
        self.assertEqual((q["temperature_unit"], q["wind_speed_unit"]), (["fahrenheit"], ["mph"]))
        self.assertEqual(self.published(), {"temp:38.72, -9.14": "65°F", "wind:38.72, -9.14": "8 mph",
                                            "low:38.72, -9.14": "55°F"})

    def test_place_with_country_hint(self):
        self.assertEqual(weather.locate("Paris, US"), (33.66094, -95.55551))
        self.assertEqual(Handler.seen[-1][1]["count"], ["10"])
        self.assertEqual(weather.locate("paris, france"), (48.85341, 2.3488))
        self.assertEqual(weather.locate("Paris, Texas"), (33.66094, -95.55551))

    def test_geocoding_is_cached(self):
        weather.locate("Lisbon")
        weather.locate("lisbon ")
        self.assertEqual(len(Handler.seen), 1)

    def test_unknown_place(self):
        self.watch("Atlantis", ["temp"])
        weather.poll(now=0)
        self.assertEqual(self.published(), {"temp:Atlantis": "–"})
        self.assertTrue(any("couldn't find a place called “Atlantis”" in m.get("message", "") for m in self.sent))
        with self.assertRaisesRegex(LookupError, "Atlantis"):
            weather.locate("Atlantis", now=10)
        self.assertEqual(len(Handler.seen), 1)  # not asked again so soon

    def test_api_errors(self):
        with self.assertRaisesRegex(ValueError, "valid latitude"):
            weather.locate("95,10")
        weather.FORECAST_URL, url = weather.FORECAST_URL.replace("/v1/", "/v9/"), weather.FORECAST_URL
        try:
            with self.assertRaisesRegex(RuntimeError, "Open-Meteo answered 404"):
                weather.weather("1,2")
        finally:
            weather.FORECAST_URL = url
        with self.assertRaisesRegex(RuntimeError, "Open-Meteo answered 500"):
            weather.weather("1.0,2")
        saved = weather.GEOCODING_URL
        weather.GEOCODING_URL = "http://127.0.0.1:9/v1/search"
        try:
            with self.assertRaisesRegex(RuntimeError, "couldn't reach Open-Meteo"):
                weather.locate("Oslo")
        finally:
            weather.GEOCODING_URL = saved

    def test_error_reason_from_open_meteo(self):
        with self.assertRaisesRegex(RuntimeError, "400: Latitude must be"):
            weather.get_json(weather.FORECAST_URL, {"latitude": "91", "longitude": "0"})

    def test_poll_schedule_and_retry(self):
        self.watch("Lisbon", ["temp"])
        weather.poll(now=0)
        weather.poll(now=300)
        self.assertEqual(len(Handler.seen), 2)  # geocode + one forecast
        weather.poll(now=600)
        self.assertEqual(len(Handler.seen), 3)
        self.watch("1.0,2", ["temp"])  # this one fails: retried after RETRY, not REFRESH
        weather.watching(sorted(weather.plugin.watching))
        weather.poll(now=601)
        weather.poll(now=650)
        self.assertEqual(len(Handler.seen), 4)
        weather.poll(now=601 + weather.RETRY)
        self.assertEqual(len(Handler.seen), 5)
        self.assertEqual(sum("weather for" in m.get("message", "") for m in self.sent), 1)  # logged once

    def test_watch_and_settings_changes(self):
        self.watch("Lisbon", ["temp"])
        weather.poll(now=0)
        self.sent.clear()
        self.watch("Lisbon", ["temp", "icon"])
        weather.watching(sorted(weather.plugin.watching))  # known place: published right away
        self.assertEqual(self.published(), {"temp:Lisbon": "18°C", "icon:Lisbon": "🌦️"})
        weather.plugin.settings = {"units": "imperial"}
        weather.configure(weather.plugin.settings)
        weather.poll(now=1)
        self.assertEqual(self.published()["temp:Lisbon"], "65°F")
        self.watch("Elsewhere", ["temp"])
        weather.watching(sorted(weather.plugin.watching))
        self.assertNotIn("Lisbon", weather._cache)

    def test_refresh_action(self):
        self.assertIn("no key", weather.refresh({}))
        self.watch("Lisbon", ["temp"])
        self.assertIsNone(weather.refresh({}))
        self.assertEqual(self.published()["temp:Lisbon"], "18°C")
        weather.plugin.watching = {"temp:Lisbon", "temp:Atlantis"}
        with self.assertRaisesRegex(RuntimeError, "Atlantis: couldn't find"):
            weather.refresh({})

    def test_refresh_answers_in_time_and_finishes_later(self):
        real = weather.weather
        release = threading.Event()

        def slow(place):
            if place == "Slowtown":
                release.wait(5)
                return dict.fromkeys(weather.VALUES, "late")
            return real(place)

        weather.plugin.watching = {"temp:Lisbon", "temp:Slowtown"}
        with mock.patch.object(weather, "weather", slow), mock.patch.object(weather, "ACTION_DEADLINE", 0.3):
            began = time.monotonic()
            answer = weather.refresh({})
            self.assertLess(time.monotonic() - began, 2)
            self.assertEqual(answer, "still working on Slowtown…")
            self.assertEqual(self.published()["temp:Lisbon"], "18°C")
            release.set()
            for _ in range(200):
                if self.published().get("temp:Slowtown") == "late":
                    break
                time.sleep(0.01)
        self.assertEqual(self.published()["temp:Slowtown"], "late")

    def test_night_icon_and_unknown_codes(self):
        data = {"current": {"weather_code": 0, "is_day": 0, "temperature_2m": -0.4}, "daily": {}}
        values = weather.describe(data, "metric")
        self.assertEqual((values["condition"], values["icon"], values["temp"]), ("Clear", "🌙", "0°C"))
        self.assertEqual(values["high"], "–")
        self.assertEqual(weather.describe({"current": {"weather_code": 42}}, "metric")["condition"], "–")


if __name__ == "__main__":
    unittest.main()
