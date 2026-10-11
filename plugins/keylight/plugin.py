"""Lights with Elgato's local HTTP API (Key Light, Key Light Air, Ring Light, Light Strip).

Each light answers on port 9123: GET/PUT /elgato/lights with
{"numberOfLights": 1, "lights": [{"on": 1, "brightness": 20, "temperature": 213}]},
where temperature is 143–344 (kelvin = 1,000,000 / value), and GET
/elgato/accessory-info for its name. A Light Strip has hue and saturation instead of a
temperature. The lights are listed on the Plugins page; nothing is discovered.
Standard library only.
"""

import concurrent.futures
import json
import os
import sys
import threading
import urllib.error
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()

PORT = 9123
TIMEOUT = 4
POLL = 5
UNKNOWN = "–"
TEMP_MIN, TEMP_MAX = 143, 344  # the API's range: about 7000 K to 2900 K
KELVIN_MIN, KELVIN_MAX = 2900, 7000
VALUES = ("on", "brightness", "temperature")

_lock = threading.Lock()
_names = {}  # address -> display name from accessory-info
_published = {}
_last_error = {}
_pool = concurrent.futures.ThreadPoolExecutor(max_workers=8)


class LightError(Exception):
    pass


# --- which lights --------------------------------------------------------------------


def addresses():
    """The lights from the settings, as "host:port"."""
    raw = (plugin.settings or {}).get("lights") or ""
    out = []
    for part in raw.replace(";", ",").replace("\n", ",").split(","):
        address = normalise(part)
        if address and address not in out:
            out.append(address)
    return out


def normalise(text):
    a = (text or "").strip()
    for prefix in ("http://", "https://"):
        if a.lower().startswith(prefix):
            a = a[len(prefix):]
    a = a.split("/")[0]
    if not a:
        return ""
    if a.startswith("["):  # [IPv6]:port
        return a if "]:" in a else f"{a}:{PORT}"
    if a.count(":") == 1:
        return a
    if a.count(":") > 1:  # bare IPv6
        return f"[{a}]:{PORT}"
    return f"{a}:{PORT}"


def host_of(address):
    return address.rsplit(":", 1)[0].strip("[]")


def pick(name):
    """The addresses an action or value means: all for blank/"all", else one light."""
    wanted = (name or "").strip()
    known = addresses()
    if not wanted or wanted.lower() == "all":
        if not known:
            raise LightError("add your lights' addresses on the Plugins page")
        return known
    low = wanted.lower()
    target = normalise(wanted).lower()
    for address in known:
        if address.lower() == target or host_of(address).lower() == low:
            return [address]
    for address in known:
        if (_display_name(address) or "").lower() == low:
            return [address]
    if any(c in wanted for c in ".:"):
        return [normalise(wanted)]  # an address that isn't in the list; use it anyway
    raise LightError(f"no light called “{wanted}”; add its address on the Plugins page")


def _display_name(address):
    with _lock:
        if address in _names:
            return _names[address]
    try:
        info = request("GET", address, "/elgato/accessory-info")
        name = str(info.get("displayName") or info.get("productName") or "")
    except LightError:
        return None  # try again next time
    with _lock:
        _names[address] = name
    return name


# --- talking to a light ----------------------------------------------------------------


def request(method, address, path, body=None):
    url = f"http://{address}{path}"
    data = json.dumps(body).encode("utf-8") if body is not None else None
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Accept", "application/json")
    if data is not None:
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
            raw = resp.read()
    except urllib.error.HTTPError as e:
        raise LightError(f"the light at {address} said {e.code}") from None
    except urllib.error.URLError as e:
        raise LightError(f"couldn't reach the light at {address}: {e.reason}") from None
    except OSError as e:
        raise LightError(f"couldn't reach the light at {address}: {e}") from None
    try:
        answer = json.loads(raw.decode("utf-8") or "{}")
    except ValueError:
        raise LightError(f"{address} didn't answer like an Elgato light") from None
    if not isinstance(answer, dict):
        raise LightError(f"{address} didn't answer like an Elgato light")
    return answer


def read(address):
    """The first light's state: {"on": 0|1, "brightness": …, "temperature": …}."""
    answer = request("GET", address, "/elgato/lights")
    lights = answer.get("lights")
    if not isinstance(lights, list) or not lights or not isinstance(lights[0], dict):
        raise LightError(f"{address} didn't answer like an Elgato light")
    return lights[0]


def write(address, changes, count=1):
    body = {"numberOfLights": count, "lights": [dict(changes) for _ in range(count)]}
    answer = request("PUT", address, "/elgato/lights", body)
    lights = answer.get("lights")
    return lights[0] if isinstance(lights, list) and lights and isinstance(lights[0], dict) else None


def on_all(targets, fn):
    """Run fn(address) for each light at once; raise if any failed, return warnings."""
    futures = {_pool.submit(fn, a): a for a in targets}
    errors, warnings = [], []
    for future, address in futures.items():
        try:
            result = future.result(timeout=TIMEOUT * 2 + 1)
        except concurrent.futures.TimeoutError:
            errors.append(f"the light at {address} didn't answer in time")
            continue
        except Exception as e:
            errors.append(str(e) or type(e).__name__)
            continue
        if isinstance(result, str):
            warnings.append(result)
        if isinstance(result, dict):
            remember(address, result)
    if errors:
        more = f" (and {len(errors) - 1} more)" if len(errors) > 1 else ""
        raise LightError(errors[0] + more)
    return warnings[0] if warnings else None


# --- conversions -------------------------------------------------------------------------


def kelvin_to_api(kelvin):
    return max(TEMP_MIN, min(TEMP_MAX, int(round(1_000_000 / kelvin))))


def api_to_kelvin(value):
    return int(round(1_000_000 / int(value) / 50) * 50)


def slider_to_kelvin(position):
    return KELVIN_MIN + (KELVIN_MAX - KELVIN_MIN) * position / 100


def number(settings, key, default, low, high):
    value = settings.get(key)
    if value in (None, ""):
        value = default
    try:
        value = float(value)
    except (TypeError, ValueError):
        raise LightError(f"“{value}” isn't a number") from None
    return max(low, min(high, value))


# --- values ---------------------------------------------------------------------------


def remember(address, state):
    """Publish a light's state right after an action, for every watched name meaning it."""
    for value_id in VALUES:
        for arg in plugin.watched(value_id):
            try:
                targets = pick(arg)
            except LightError:
                continue
            if targets == [address]:
                publish(value_id, arg, describe(value_id, state))


def describe(value_id, state):
    if state is None:
        return UNKNOWN
    if value_id == "on":
        return "on" if state.get("on") else "off"
    if value_id == "brightness":
        return str(int(state.get("brightness", 0))) if state.get("on") else "0"
    if value_id == "temperature":
        t = state.get("temperature")
        return f"{api_to_kelvin(t)} K" if isinstance(t, (int, float)) and t > 0 else UNKNOWN
    return UNKNOWN


def publish(value_id, arg, text):
    key = f"{value_id}:{arg}"
    with _lock:
        if _published.get(key) == text:
            return
        _published[key] = text
    plugin.set_value(value_id, text, arg=arg)


def poll():
    args = sorted({arg for value_id in VALUES for arg in plugin.watched(value_id)})
    if not args:
        return
    wanted = {}
    for arg in args:
        try:
            wanted[arg] = pick(arg)
        except LightError:
            wanted[arg] = []
    states = {}
    unique = sorted({a for targets in wanted.values() for a in targets})
    futures = {_pool.submit(read, a): a for a in unique}
    for future, address in futures.items():
        try:
            states[address] = future.result(timeout=TIMEOUT * 2 + 1)
            error = None
        except Exception as e:
            states[address] = None
            error = str(e) or type(e).__name__
        if _last_error.get(address) != error:
            _last_error[address] = error
            if error:
                plugin.log(error, "warn")
    for arg, targets in wanted.items():
        found = [states.get(a) for a in targets]
        for value_id in VALUES:
            if arg not in plugin.watched(value_id):
                continue
            if not targets or any(s is None for s in found):
                text = UNKNOWN
            elif len(found) == 1:
                text = describe(value_id, found[0])
            elif value_id == "on":
                text = "on" if any(s.get("on") for s in found) else "off"
            else:
                texts = {describe(value_id, s) for s in found}
                text = texts.pop() if len(texts) == 1 else UNKNOWN
            publish(value_id, arg, text)


@plugin.every(POLL)
def poll_timer():
    poll()


@plugin.on_watch
def watching(values):
    keep = set(values)
    with _lock:
        for key in [k for k in _published if k not in keep]:
            del _published[key]


@plugin.on_settings
def configure(_settings):
    with _lock:
        _names.clear()
        _published.clear()
    _last_error.clear()


# --- actions ---------------------------------------------------------------------------


@plugin.action("power")
def power(settings):
    targets = pick(settings.get("light"))
    mode = settings.get("mode") or "toggle"
    if mode == "toggle":  # all on unless any is on: then all off
        states = [read(a) for a in targets] if len(targets) == 1 else _read_all(targets)
        on = 0 if any(s.get("on") for s in states) else 1
    else:
        on = 1 if mode == "on" else 0
    return on_all(targets, lambda a: write(a, {"on": on}))


def _read_all(targets):
    futures = [_pool.submit(read, a) for a in targets]
    return [f.result(timeout=TIMEOUT * 2 + 1) for f in futures]


@plugin.action("brightness")
def brightness(settings):
    targets = pick(settings.get("light"))
    value = int(round(number(settings, "value", 50, 0, 100)))
    changes = {"on": 0} if value <= 0 else {"on": 1, "brightness": value}
    return on_all(targets, lambda a: write(a, changes))


def _nudge(settings, sign):
    targets = pick(settings.get("light"))
    by = number(settings, "step", 10, 1, 100)

    def one(address):
        state = read(address)
        current = int(state.get("brightness", 0)) if state.get("on") else 0
        if sign < 0 and not state.get("on"):
            return "it's already off"
        value = max(1, min(100, int(round(current + sign * by))))
        return write(address, {"on": 1, "brightness": value})

    return on_all(targets, one)


@plugin.action("brightness-up")
def brightness_up(settings):
    return _nudge(settings, +1)


@plugin.action("brightness-down")
def brightness_down(settings):
    return _nudge(settings, -1)


def _set_temperature(targets, kelvin_of_state):
    def one(address):
        state = read(address)
        if "temperature" not in state:
            return "that light has colours, not shades of white"
        kelvin = kelvin_of_state(state)
        return write(address, {"on": 1, "temperature": kelvin_to_api(kelvin)})

    return on_all(targets, one)


@plugin.action("temperature")
def temperature(settings):
    targets = pick(settings.get("light"))
    kelvin = slider_to_kelvin(number(settings, "value", 50, 0, 100))
    return _set_temperature(targets, lambda _state: kelvin)


def _shift(settings, sign):
    targets = pick(settings.get("light"))
    by = number(settings, "step", 300, 10, KELVIN_MAX)

    def kelvin(state):
        current = 1_000_000 / max(1, int(state.get("temperature") or 213))
        return max(KELVIN_MIN, min(KELVIN_MAX, current + sign * by))

    return _set_temperature(targets, kelvin)


@plugin.action("warmer")
def warmer(settings):
    return _shift(settings, -1)


@plugin.action("cooler")
def cooler(settings):
    return _shift(settings, +1)


if __name__ == "__main__":
    plugin.run()
