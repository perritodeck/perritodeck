"""Weather from Open-Meteo (https://open-meteo.com): free, no account or key.

The value's argument is a place name ("Lisbon", "Paris, France", "Springfield, Illinois")
or coordinates ("38.72,-9.14"). Names are looked up once with Open-Meteo's geocoding
API; each place's forecast is fetched every 10 minutes while a key shows it.
Standard library only.
"""

import concurrent.futures
import json
import os
import re
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()

# Module constants so tests (or a self-hosted Open-Meteo) can point elsewhere.
FORECAST_URL = "https://api.open-meteo.com/v1/forecast"
GEOCODING_URL = "https://geocoding-api.open-meteo.com/v1/search"
TIMEOUT = 8
REFRESH = 600  # seconds between forecasts for a place
RETRY = 120  # seconds before trying again after a failure
NOT_FOUND_RETRY = 3600
ACTION_DEADLINE = 8.0  # the host wants an answer within 10 s; slower places finish later
VALUES = ("temp", "feels", "condition", "icon", "humidity", "wind", "high", "low", "rain_chance")
UNKNOWN = "–"

# WMO weather interpretation codes, as listed in Open-Meteo's forecast API docs.
# code: (short text, emoji by day, emoji by night)
WMO = {
    0: ("Clear", "☀️", "🌙"),
    1: ("Mainly clear", "🌤️", "🌙"),
    2: ("Partly cloudy", "⛅", "☁️"),
    3: ("Cloudy", "☁️", "☁️"),
    45: ("Fog", "🌫️", "🌫️"),
    48: ("Icy fog", "🌫️", "🌫️"),
    51: ("Light drizzle", "🌦️", "🌧️"),
    53: ("Drizzle", "🌦️", "🌧️"),
    55: ("Heavy drizzle", "🌧️", "🌧️"),
    56: ("Freezing drizzle", "🌧️", "🌧️"),
    57: ("Freezing drizzle", "🌧️", "🌧️"),
    61: ("Light rain", "🌦️", "🌧️"),
    63: ("Rain", "🌧️", "🌧️"),
    65: ("Heavy rain", "🌧️", "🌧️"),
    66: ("Freezing rain", "🌧️", "🌧️"),
    67: ("Freezing rain", "🌧️", "🌧️"),
    71: ("Light snow", "🌨️", "🌨️"),
    73: ("Snow", "🌨️", "🌨️"),
    75: ("Heavy snow", "❄️", "❄️"),
    77: ("Snow grains", "🌨️", "🌨️"),
    80: ("Light showers", "🌦️", "🌧️"),
    81: ("Showers", "🌧️", "🌧️"),
    82: ("Heavy showers", "🌧️", "🌧️"),
    85: ("Snow showers", "🌨️", "🌨️"),
    86: ("Heavy snow showers", "🌨️", "🌨️"),
    95: ("Thunderstorm", "⛈️", "⛈️"),
    96: ("Storm with hail", "⛈️", "⛈️"),
    99: ("Storm with hail", "⛈️", "⛈️"),
}

COORDS = re.compile(r"\s*(-?\d{1,3}(?:\.\d+)?)\s*,\s*(-?\d{1,3}(?:\.\d+)?)\s*")

_lock = threading.Lock()
_geo = {}  # lowercased place -> (lat, lon) or (None, time it wasn't found)
_cache = {}  # place -> {"time": monotonic, "values": {...} or None, "error": str or None}
_pool = concurrent.futures.ThreadPoolExecutor(max_workers=6)


def get_json(url, params):
    full = url + "?" + urllib.parse.urlencode(params)
    request = urllib.request.Request(full, headers={"User-Agent": "PerritoDeck weather plugin", "Accept": "application/json"})
    try:
        with urllib.request.urlopen(request, timeout=TIMEOUT) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        try:
            reason = json.loads(e.read().decode("utf-8")).get("reason")
        except (ValueError, AttributeError, OSError):
            reason = None
        raise RuntimeError(f"Open-Meteo answered {e.code}" + (f": {reason}" if reason else "")) from None
    except urllib.error.URLError as e:
        raise RuntimeError(f"couldn't reach Open-Meteo: {e.reason}") from None
    except (OSError, ValueError) as e:
        raise RuntimeError(f"couldn't read Open-Meteo's answer: {e}") from None


def units():
    return "imperial" if plugin.settings.get("units") == "imperial" else "metric"


def locate(place, now=None):
    """(latitude, longitude) for a place name or "lat,lon"."""
    match = COORDS.fullmatch(place)
    if match:
        lat, lon = float(match.group(1)), float(match.group(2))
        if not (-90 <= lat <= 90 and -180 <= lon <= 180):
            raise ValueError(f"“{place}” isn't a valid latitude,longitude")
        return lat, lon
    now = time.monotonic() if now is None else now
    key = place.strip().lower()
    with _lock:
        known = _geo.get(key)
    if known and known[0] is not None:
        return known
    if known and now - known[1] < NOT_FOUND_RETRY:
        raise LookupError(f"couldn't find a place called “{place}”")
    name, _, hint = (part.strip() for part in place.partition(","))
    if not name:
        raise ValueError("enter a place name")
    data = get_json(GEOCODING_URL, {"name": name, "count": 10 if hint else 1, "language": "en", "format": "json"})
    results = data.get("results") or []
    if hint:
        h = hint.lower()
        results = [
            r for r in results
            if h in (str(r.get("country", "")).lower(), str(r.get("country_code", "")).lower(), str(r.get("admin1", "")).lower())
        ]
    if not results:
        with _lock:
            _geo[key] = (None, now)
        raise LookupError(f"couldn't find a place called “{place}”")
    coords = (float(results[0]["latitude"]), float(results[0]["longitude"]))
    with _lock:
        _geo[key] = coords
    return coords


def forecast_params(lat, lon, unit_system):
    params = {
        "latitude": f"{lat:.4f}",
        "longitude": f"{lon:.4f}",
        "current": "temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m,is_day",
        "daily": "temperature_2m_max,temperature_2m_min,precipitation_probability_max",
        "timezone": "auto",
        "forecast_days": 1,
    }
    if unit_system == "imperial":
        params.update(temperature_unit="fahrenheit", wind_speed_unit="mph")
    else:
        params["wind_speed_unit"] = "kmh"
    return params


def describe(data, unit_system):
    """Turn a forecast answer into the plugin's values."""
    current = data.get("current") or {}
    daily = data.get("daily") or {}
    degrees = "°F" if unit_system == "imperial" else "°C"
    speed = "mph" if unit_system == "imperial" else "km/h"

    def first(key):
        values = daily.get(key) or []
        return values[0] if values else None

    def fmt(value, suffix):
        if value is None:
            return UNKNOWN
        return f"{int(round(float(value)))}{suffix}"

    code = current.get("weather_code")
    text, day_icon, night_icon = WMO.get(code, (UNKNOWN, UNKNOWN, UNKNOWN))
    night = current.get("is_day") == 0
    return {
        "temp": fmt(current.get("temperature_2m"), degrees),
        "feels": fmt(current.get("apparent_temperature"), degrees),
        "condition": text,
        "icon": night_icon if night else day_icon,
        "humidity": fmt(current.get("relative_humidity_2m"), "%"),
        "wind": fmt(current.get("wind_speed_10m"), " " + speed),
        "high": fmt(first("temperature_2m_max"), degrees),
        "low": fmt(first("temperature_2m_min"), degrees),
        "rain_chance": fmt(first("precipitation_probability_max"), "%"),
    }


def weather(place):
    lat, lon = locate(place)
    unit_system = units()
    return describe(get_json(FORECAST_URL, forecast_params(lat, lon, unit_system)), unit_system)


def watched_places():
    places = set()
    for value in VALUES:
        places.update(plugin.watched(value))
    return sorted(places)


def publish(place):
    with _lock:
        entry = _cache.get(place) or {}
    values = entry.get("values") or {}
    for value in VALUES:
        if place in plugin.watched(value):
            plugin.set_value(value, values.get(value, UNKNOWN), arg=place)


def update(place, now=None):
    """Fetch one place's weather, cache it and publish it. Returns the error, if any."""
    now = time.monotonic() if now is None else now
    try:
        values, error = weather(place), None
    except Exception as e:
        values, error = None, str(e) or type(e).__name__
    with _lock:
        old = _cache.get(place) or {}
        _cache[place] = {"time": now, "values": values, "error": error}
    if error and error != old.get("error"):
        plugin.log(f"weather for “{place}”: {error}", "warn")
    publish(place)
    return error


def due(place, now):
    with _lock:
        entry = _cache.get(place)
    if entry is None:
        return True
    return now - entry["time"] >= (RETRY if entry["error"] else REFRESH)


@plugin.every(5)
def poll(now=None):
    now = time.monotonic() if now is None else now
    for place in watched_places():
        if due(place, now):
            update(place, now)


@plugin.on_watch
def watching(_values):
    places = set(watched_places())
    with _lock:
        for place in [p for p in _cache if p not in places]:
            del _cache[place]
        known = [p for p in places if p in _cache]
    for place in known:  # new places are fetched by the next poll, within 5 s
        publish(place)


@plugin.on_settings
def configure(_settings):
    with _lock:
        _cache.clear()  # the units may have changed: fetch everything again


@plugin.action("refresh")
def refresh(settings):
    places = watched_places()
    if not places:
        return "no key shows the weather yet"
    futures = {_pool.submit(update, p): p for p in places}
    done, pending = concurrent.futures.wait(futures, timeout=ACTION_DEADLINE)
    errors = {futures[f]: f.result() for f in done if f.result()}
    if pending:  # they keep going and publish when they're done
        waiting = sorted(futures[f] for f in pending)
        more = f" and {len(waiting) - 1} more" if len(waiting) > 1 else ""
        return f"still working on {waiting[0]}{more}…"
    if errors:
        place, error = next(iter(errors.items()))
        more = f" (and {len(errors) - 1} more)" if len(errors) > 1 else ""
        raise RuntimeError(f"{place}: {error}{more}")
    return None


if __name__ == "__main__":
    plugin.run()
