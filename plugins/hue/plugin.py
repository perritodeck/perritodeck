"""Philips Hue lights through the Hue Bridge on the local network (API v1).

Pairing: press the round link button on the bridge, then the "Pair with the bridge"
key within 30 seconds. The bridge hands out a username (an app key) that's kept in the
plugin's data folder, never in the settings or the log.

Lights and rooms are named the way the Hue app names them. Rooms, zones and other
groups are matched first, then single lights; leaving the name blank means every light.
Standard library only.
"""

import json
import os
import re
import socket
import ssl
import sys
import threading
import time
import urllib.error
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()

DISCOVERY_URL = "https://discovery.meethue.com/"
TIMEOUT = 5
POLL = 5
CACHE_AGE = 10  # seconds a list of lights and rooms is trusted for actions
UNKNOWN = "–"
APP_NAME = "perritodeck"
STORE = "hue.json"

# Colour gamuts from the Hue developer docs ("Color conversion formulas RGB to XY").
GAMUTS = {
    "A": ((0.704, 0.296), (0.2151, 0.7106), (0.138, 0.08)),
    "B": ((0.675, 0.322), (0.409, 0.518), (0.167, 0.04)),
    "C": ((0.6915, 0.3083), (0.17, 0.7), (0.1532, 0.0475)),
}
HEX = re.compile(r"#?([0-9a-fA-F]{6}|[0-9a-fA-F]{3})")

_lock = threading.Lock()
_state = {"lights": None, "groups": None, "time": 0.0}
_store = None  # {"username": …, "discovered": ip}
_published = {}


class HueError(Exception):
    pass


# --- settings and the stored username -----------------------------------------


def store_path():
    return os.path.join(plugin.data_dir, STORE)


def load_store():
    global _store
    if _store is None:
        try:
            with open(store_path(), encoding="utf-8") as f:
                data = json.load(f)
            _store = data if isinstance(data, dict) else {}
        except (OSError, ValueError):
            _store = {}
    return _store


def save_store(**changes):
    data = dict(load_store())
    data.update(changes)
    path = store_path()
    tmp = path + ".tmp"
    # Owner-only from the start: the bridge key gives full control of the lights.
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        json.dump(data, f)
    try:
        os.chmod(tmp, 0o600)  # a .tmp left over from before keeps its old mode
    except OSError:
        pass
    os.replace(tmp, path)
    global _store
    _store = data


def discover():
    """The bridge's address from Philips' discovery service (asked once, then cached)."""
    request = urllib.request.Request(DISCOVERY_URL, headers={"User-Agent": "PerritoDeck", "Accept": "application/json"})
    try:
        with urllib.request.urlopen(request, timeout=TIMEOUT) as resp:
            found = json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        if e.code == 429:
            raise HueError("the bridge finder is busy; enter the bridge's address on the Plugins page") from None
        raise HueError(f"couldn't find the bridge (discovery said {e.code}); enter its address on the Plugins page") from None
    except (urllib.error.URLError, OSError, ValueError) as e:
        reason = getattr(e, "reason", e)
        raise HueError(f"couldn't find the bridge ({reason}); enter its address on the Plugins page") from None
    if not isinstance(found, list) or not found or not isinstance(found[0], dict) or not found[0].get("internalipaddress"):
        raise HueError("no Hue Bridge found on this network; enter its address on the Plugins page")
    address = str(found[0]["internalipaddress"])
    if len(found) > 1:
        plugin.log(f"found {len(found)} Hue Bridges; using {address}. Enter another on the Plugins page.", "warn")
    save_store(discovered=address)
    return address


def bridge(rediscover=False):
    """The bridge's base URL, e.g. http://192.168.1.2."""
    s = plugin.settings or {}
    address = (s.get("bridge") or "").strip().rstrip("/")
    if not address:
        address = None if rediscover else load_store().get("discovered")
        address = address or discover()
    scheme = "https" if s.get("https") else "http"
    if "://" in address:
        return address
    return f"{scheme}://{address}"


def username():
    name = load_store().get("username")
    if not name:
        raise HueError("not paired yet: press the bridge's link button, then the Pair key")
    return name


def tls_context():
    if not (plugin.settings or {}).get("insecure"):
        return ssl.create_default_context()
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    return ctx


# --- talking to the bridge ---------------------------------------------------------


def call(method, path, body=None, base=None):
    """Call the bridge; returns the decoded JSON. v1 errors come back as HueError."""
    base = base or bridge()
    url = base + path
    data = json.dumps(body).encode("utf-8") if body is not None else None
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Accept", "application/json")
    req.add_header("User-Agent", "PerritoDeck")
    if data is not None:
        req.add_header("Content-Type", "application/json")
    context = tls_context() if url.startswith("https://") else None
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT, context=context) as resp:
            raw = resp.read()
    except urllib.error.HTTPError as e:
        raise HueError(f"the bridge said {e.code}") from None
    except urllib.error.URLError as e:
        raise HueError(describe_unreachable(base, e.reason)) from None
    except (OSError, ssl.SSLError) as e:
        raise HueError(describe_unreachable(base, e)) from None
    try:
        answer = json.loads(raw.decode("utf-8") or "null")
    except ValueError:
        raise HueError(f"{base} didn't answer like a Hue Bridge") from None
    errors = [item["error"] for item in answer if isinstance(item, dict) and "error" in item] if isinstance(answer, list) else []
    if errors:
        raise HueError(describe_error(errors[0]))
    return answer


def describe_unreachable(base, reason):
    text = str(reason)
    if isinstance(reason, ssl.SSLCertVerificationError) or "CERTIFICATE_VERIFY_FAILED" in text:
        return (f"the bridge's certificate isn't trusted ({base}); turn on “Skip certificate check” "
                "on the Plugins page or turn off HTTPS")
    if isinstance(reason, socket.timeout) or "timed out" in text:
        return f"the bridge at {base} didn't answer in time"
    return f"couldn't reach the bridge at {base}: {text}"


def describe_error(error):
    kind = error.get("type")
    if kind == 1:
        return "the bridge doesn't know this app any more: press its link button, then the Pair key"
    if kind == 101:
        return "press the round link button on the bridge first, then this key within 30 seconds"
    if kind == 201:
        return "that light is off, so it can't change colour or brightness"
    if kind == 3:
        return "the bridge says that light, room or scene doesn't exist any more"
    return f"the bridge said: {error.get('description') or 'error ' + str(kind)}"


def api(method, path, body=None):
    return call(method, f"/api/{username()}{path}", body)


def refresh(force=False):
    """Lights and groups, from the bridge if the copy we have is older than CACHE_AGE."""
    with _lock:
        fresh = _state["lights"] is not None and time.monotonic() - _state["time"] < CACHE_AGE
        if fresh and not force:
            return _state["lights"], _state["groups"]
    lights = api("GET", "/lights")
    groups = api("GET", "/groups")
    if not isinstance(lights, dict) or not isinstance(groups, dict):
        raise HueError("the bridge sent an unexpected answer")
    with _lock:
        _state.update(lights=lights, groups=groups, time=time.monotonic())
    return lights, groups


def forget():
    with _lock:
        _state.update(lights=None, groups=None, time=0.0)


# --- finding lights and rooms --------------------------------------------------------


class Target:
    """A light ("lights", id) or a group ("groups", id); group 0 is every light."""

    def __init__(self, kind, ident, name, info):
        self.kind, self.id, self.name, self.info = kind, ident, name, info or {}

    @property
    def state_path(self):
        return f"/lights/{self.id}/state" if self.kind == "lights" else f"/groups/{self.id}/action"

    def is_on(self):
        if self.kind == "lights":
            return bool((self.info.get("state") or {}).get("on"))
        state = self.info.get("state") or {}
        if "any_on" in state:
            return bool(state["any_on"])
        return bool((self.info.get("action") or {}).get("on"))


def find(name, lights, groups):
    wanted = (name or "").strip()
    if not wanted or wanted.lower() in ("all", "all lights"):
        return Target("groups", "0", "all lights", None)
    low = wanted.lower()
    for ident, group in sorted(groups.items(), key=lambda kv: kv[0]):
        if str(group.get("name", "")).strip().lower() == low:
            return Target("groups", ident, group.get("name"), group)
    for ident, light in sorted(lights.items(), key=lambda kv: kv[0]):
        if str(light.get("name", "")).strip().lower() == low:
            return Target("lights", ident, light.get("name"), light)
    return None


def target(name):
    lights, groups = refresh()
    found = find(name, lights, groups)
    if found is None:  # maybe it was just renamed or added
        lights, groups = refresh(force=True)
        found = find(name, lights, groups)
    if found is None:
        raise HueError(f"no light or room called “{name.strip()}”")
    if found.kind == "groups" and found.id == "0":
        found.info = {"state": {"any_on": any((l.get("state") or {}).get("on") for l in lights.values())}}
    return found


def put(t, body):
    api("PUT", t.state_path, body)
    forget()  # the next poll reads the new state
    poll_soon()


# --- colour --------------------------------------------------------------------------


def parse_hex(text):
    match = HEX.fullmatch((text or "").strip())
    if not match:
        raise HueError(f"“{text}” isn't a colour like #ff8800")
    h = match.group(1)
    if len(h) == 3:
        h = "".join(c * 2 for c in h)
    return tuple(int(h[i:i + 2], 16) / 255 for i in (0, 2, 4))


def gamma(c):
    return ((c + 0.055) / 1.055) ** 2.4 if c > 0.04045 else c / 12.92


def rgb_to_xy(r, g, b, gamut=None):
    """CIE xy for an sRGB colour (0–1 each), as in the Hue developer docs, pulled into gamut."""
    r, g, b = gamma(r), gamma(g), gamma(b)
    x = r * 0.664511 + g * 0.154324 + b * 0.162028
    y = r * 0.283881 + g * 0.668433 + b * 0.047685
    z = r * 0.000088 + g * 0.072310 + b * 0.986039
    total = x + y + z
    if total == 0:
        raise HueError("black can't be shown; use “Turn off” instead")
    point = (x / total, y / total)
    if gamut:
        point = into_gamut(point, gamut)
    return [round(point[0], 4), round(point[1], 4)]


def _cross(a, b):
    return a[0] * b[1] - a[1] * b[0]


def in_gamut(p, gamut):
    red, green, blue = gamut
    v1 = (green[0] - red[0], green[1] - red[1])
    v2 = (blue[0] - red[0], blue[1] - red[1])
    q = (p[0] - red[0], p[1] - red[1])
    d = _cross(v1, v2)
    s = _cross(q, v2) / d
    t = _cross(v1, q) / d
    return s >= 0 and t >= 0 and s + t <= 1


def _closest_on_line(a, b, p):
    ab = (b[0] - a[0], b[1] - a[1])
    ap = (p[0] - a[0], p[1] - a[1])
    t = (ap[0] * ab[0] + ap[1] * ab[1]) / (ab[0] ** 2 + ab[1] ** 2)
    t = min(1.0, max(0.0, t))
    return (a[0] + ab[0] * t, a[1] + ab[1] * t)


def into_gamut(p, gamut):
    if in_gamut(p, gamut):
        return p
    red, green, blue = gamut
    candidates = [_closest_on_line(red, green, p), _closest_on_line(blue, red, p), _closest_on_line(green, blue, p)]
    return min(candidates, key=lambda c: (c[0] - p[0]) ** 2 + (c[1] - p[1]) ** 2)


def gamut_of(light):
    control = ((light or {}).get("capabilities") or {}).get("control") or {}
    points = control.get("colorgamut")
    if isinstance(points, list) and len(points) == 3:
        try:
            return tuple((float(p[0]), float(p[1])) for p in points)
        except (TypeError, ValueError, IndexError):
            pass
    return GAMUTS.get(control.get("colorgamuttype"))


def kelvin_to_mirek(kelvin, light=None):
    try:
        k = float(kelvin)
    except (TypeError, ValueError):
        raise HueError(f"“{kelvin}” isn't a colour temperature like 2700") from None
    if k <= 0:
        raise HueError("the colour temperature is in kelvin, e.g. 2700 (warm) to 6500 (cool)")
    ct = ((light or {}).get("capabilities") or {}).get("control", {}).get("ct") or {}
    low, high = int(ct.get("min", 153)), int(ct.get("max", 500))
    return max(low, min(high, int(round(1_000_000 / k))))


# --- values --------------------------------------------------------------------------


def brightness_pct(bri):
    return int(round(max(0, min(254, int(bri))) * 100 / 254))


def describe(name, lights, groups):
    """(on, brightness) texts for a watched light or room."""
    t = find(name, lights, groups)
    if t is None:
        return UNKNOWN, UNKNOWN
    if t.kind == "lights":
        state = t.info.get("state") or {}
        if state.get("reachable") is False:
            return UNKNOWN, UNKNOWN
        on = bool(state.get("on"))
        bri = state.get("bri")
        return ("on" if on else "off"), (str(brightness_pct(bri)) if on and bri is not None else "0")
    if t.id == "0":
        members = list(lights.values())
    else:
        members = [lights[i] for i in t.info.get("lights") or [] if i in lights]
    states = [m.get("state") or {} for m in members]
    on_states = [s for s in states if s.get("on") and s.get("reachable") is not False]
    if t.id != "0" and "any_on" in (t.info.get("state") or {}):
        any_on = bool(t.info["state"]["any_on"])
    else:
        any_on = bool(on_states)
    if not any_on:
        return "off", "0"
    bris = [s["bri"] for s in on_states if s.get("bri") is not None]
    if bris:
        return "on", str(brightness_pct(sum(bris) / len(bris)))
    bri = (t.info.get("action") or {}).get("bri")
    return "on", str(brightness_pct(bri)) if bri is not None else UNKNOWN


def publish(value_id, arg, text):
    key = f"{value_id}:{arg}"
    with _lock:
        if _published.get(key) == text:
            return
        _published[key] = text
    plugin.set_value(value_id, text, arg=arg)


_poll_now = threading.Event()
_last_error = [None]


def poll_soon():
    _poll_now.set()


def poll():
    names = sorted(set(plugin.watched("on")) | set(plugin.watched("brightness")))
    if not names:
        return
    try:
        lights, groups = refresh(force=True)
        error = None
    except Exception as e:
        lights = groups = None
        error = str(e) or type(e).__name__
    if error != _last_error[0]:
        _last_error[0] = error
        if error:
            plugin.log(f"Hue: {error}", "warn")
    for name in names:
        on, bri = (UNKNOWN, UNKNOWN) if error else describe(name, lights, groups)
        if name in plugin.watched("on"):
            publish("on", name, on)
        if name in plugin.watched("brightness"):
            publish("brightness", name, bri)


@plugin.every(POLL)
def poll_timer():
    poll()


@plugin.on_start
def started():
    def soon():  # after an action, read the new state without waiting for the timer
        while True:
            _poll_now.wait()
            _poll_now.clear()
            time.sleep(0.3)
            try:
                poll()
            except Exception as e:  # never let the helper thread die
                plugin.log(f"Hue: {e}", "warn")

    threading.Thread(target=soon, name="hue-soon", daemon=True).start()


@plugin.on_watch
def watching(values):
    keep = set(values)
    with _lock:
        for key in [k for k in _published if k not in keep]:
            del _published[key]
    poll_soon()


@plugin.on_settings
def configure(_settings):
    forget()
    with _lock:
        _published.clear()


# --- actions ---------------------------------------------------------------------


def step(settings, default=10):
    try:
        value = float(settings.get("step") or default)
    except (TypeError, ValueError):
        value = default
    return max(1, min(100, value))


@plugin.action("pair")
def pair(settings):
    base = bridge()
    host = re.sub(r"[^A-Za-z0-9 _-]", "", socket.gethostname() or "computer")[:19] or "computer"
    try:
        answer = call("POST", "/api", {"devicetype": f"{APP_NAME}#{host}"}, base=base)
    except HueError as e:
        if "couldn't reach" in str(e) and not (plugin.settings or {}).get("bridge") and load_store().get("discovered"):
            base = bridge(rediscover=True)  # the bridge may have a new address
            answer = call("POST", "/api", {"devicetype": f"{APP_NAME}#{host}"}, base=base)
        else:
            raise
    try:
        name = answer[0]["success"]["username"]
    except (TypeError, KeyError, IndexError):
        raise HueError("the bridge didn't hand out a username; try again") from None
    save_store(username=name)
    forget()
    poll_soon()
    plugin.log(f"paired with the Hue Bridge at {base}")
    return None


@plugin.action("power")
def power(settings):
    t = target(settings.get("light", ""))
    mode = settings.get("mode") or "toggle"
    on = (not t.is_on()) if mode == "toggle" else mode == "on"
    put(t, {"on": on})


@plugin.action("brightness")
def brightness(settings):
    t = target(settings.get("light", ""))
    try:
        value = float(settings.get("value"))
    except (TypeError, ValueError):
        raise HueError("brightness is a number from 0 to 100") from None
    value = max(0.0, min(100.0, value))
    if value <= 0:
        put(t, {"on": False})
    else:
        put(t, {"on": True, "bri": max(1, int(round(value * 254 / 100)))})


@plugin.action("brightness-up")
def brightness_up(settings):
    t = target(settings.get("light", ""))
    put(t, {"on": True, "bri_inc": int(round(step(settings) * 254 / 100))})


@plugin.action("brightness-down")
def brightness_down(settings):
    t = target(settings.get("light", ""))
    if not t.is_on():
        return "it's already off"
    put(t, {"bri_inc": -int(round(step(settings) * 254 / 100))})


@plugin.action("color")
def color(settings):
    r, g, b = parse_hex(settings.get("color"))
    t = target(settings.get("light", ""))
    xy = rgb_to_xy(r, g, b, gamut_of(t.info) if t.kind == "lights" else None)
    put(t, {"on": True, "xy": xy})


@plugin.action("temperature")
def temperature(settings):
    t = target(settings.get("light", ""))
    put(t, {"on": True, "ct": kelvin_to_mirek(settings.get("kelvin") or 2700, t.info if t.kind == "lights" else None)})


@plugin.action("scene")
def scene(settings):
    wanted = (settings.get("scene") or "").strip()
    if not wanted:
        raise HueError("enter the scene's name")
    room_name = (settings.get("room") or "").strip()
    lights, groups = refresh()
    room = None
    if room_name:
        room = find(room_name, {}, groups)
        if room is None:
            lights, groups = refresh(force=True)
            room = find(room_name, {}, groups)
        if room is None:
            raise HueError(f"no room called “{room_name}”")
    scenes = api("GET", "/scenes")
    if not isinstance(scenes, dict):
        raise HueError("the bridge sent an unexpected answer")
    matches = []
    for ident, info in scenes.items():
        if str(info.get("name", "")).strip().lower() != wanted.lower():
            continue
        group = info.get("group")
        if room is not None and room.id != "0":
            room_lights = set(room.info.get("lights") or [])
            if group is not None and str(group) != room.id:
                continue
            if group is None and not (set(info.get("lights") or []) & room_lights):
                continue
        matches.append((str(info.get("lastupdated") or ""), ident, info))
    if not matches:
        where = f" in {room.name}" if room is not None else ""
        raise HueError(f"no scene called “{wanted}”{where}")
    matches.sort(reverse=True)  # the most recently changed one if names repeat
    _, ident, info = matches[0]
    group = info.get("group") or (room.id if room is not None else "0")
    api("PUT", f"/groups/{group}/action", {"scene": ident})
    forget()
    poll_soon()
    if len(matches) > 1 and room is None:
        return f"{len(matches)} scenes are called “{wanted}”; enter the room to pick one"
    return None


if __name__ == "__main__":
    plugin.run()
