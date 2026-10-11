"""Clocks: the time and date anywhere, countdowns, days until a date, the week number.

Values only; there's nothing to press. Every value is worked out from the computer's
clock, so nothing goes over the network. Time zones use the IANA names ("Asia/Tokyo")
through zoneinfo (Python 3.9+). Windows has no time zone database of its own: there,
zone names need Python's "tzdata" package, and fixed offsets ("UTC+5:30") always work.
Standard library only.
"""

import datetime
import os
import re
import sys
import threading

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

try:
    import zoneinfo
except ImportError:  # Python 3.8
    zoneinfo = None

plugin = Plugin()

UNKNOWN = "–"
TICK = 0.5  # seconds; values are only sent when their text changes
FIXED = ("time", "date", "week")  # always published for the computer's own time zone
ARG_VALUES = ("time", "date", "week", "countdown", "days_until")
LOCAL_TZ = None  # None: the computer's time zone (tests set a fixed one)

DAYS = ("Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun")
MONTHS = ("Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec")

OFFSET = re.compile(r"(?:UTC|GMT)?\s*([+-])\s*(\d{1,2})(?:(?::|h)?(\d{2}))?", re.IGNORECASE)
YEARLY = re.compile(r"(?:--)?(\d{1,2})-(\d{1,2})")

_lock = threading.Lock()
_published = {}  # full value id -> text last sent
_hinted = set()  # values we've already logged a problem with
_zones = {}  # time zone text -> (tzinfo or None, error or None)


def now():
    """The current time, timezone-aware (tests replace this)."""
    return datetime.datetime.now(datetime.timezone.utc)


def local(moment):
    return moment.astimezone(LOCAL_TZ) if LOCAL_TZ is not None else moment.astimezone()


# --- time zones ------------------------------------------------------------------


def zone(name):
    """A tzinfo for "Europe/Lisbon", "UTC", "UTC+5:30", "-03:00"; None for local time.

    Raises LookupError with a short reason when the zone can't be used."""
    text = (name or "").strip()
    if not text or text.lower() == "local":
        return None
    if text.upper() in ("UTC", "GMT", "Z"):
        return datetime.timezone.utc
    match = OFFSET.fullmatch(text)
    if match:
        sign, hours, minutes = match.group(1), int(match.group(2)), int(match.group(3) or 0)
        if hours > 14 or minutes >= 60:
            raise LookupError(f"“{text}” isn't a valid UTC offset")
        delta = datetime.timedelta(hours=hours, minutes=minutes)
        return datetime.timezone(-delta if sign == "-" else delta)
    if zoneinfo is None:
        raise LookupError("time zone names need Python 3.9 or newer; use an offset like UTC+5:30")
    try:
        return zoneinfo.ZoneInfo(text)
    except Exception:  # ZoneInfoNotFoundError, or ValueError for odd keys
        pass
    if not zoneinfo.available_timezones():
        raise LookupError(
            f"no time zone database to look up “{text}”: install Python's tzdata package "
            "(py -m pip install tzdata) or use an offset like UTC+5:30"
        )
    raise LookupError(f"unknown time zone “{text}”; use a name like Europe/Lisbon or an offset like UTC+5:30")


def cached_zone(name):
    """zone(), remembering answers (and failures) so a bad name isn't looked up twice a second."""
    key = (name or "").strip()
    with _lock:
        known = _zones.get(key)
    if known is None:
        try:
            known = (zone(key), None)
        except LookupError as e:
            known = (None, str(e))
        with _lock:
            _zones[key] = known
    if known[1]:
        raise LookupError(known[1])
    return known[0]


def zoned(name, moment):
    tz = cached_zone(name)
    return local(moment) if tz is None else moment.astimezone(tz)


# --- formatting ------------------------------------------------------------------


def format_time(moment, settings):
    if settings.get("format") == "12h":
        hour = moment.hour % 12 or 12
        return f"{hour}:{moment.minute:02d} {'AM' if moment.hour < 12 else 'PM'}"
    return f"{moment.hour:02d}:{moment.minute:02d}"


def format_date(moment, settings):
    style = settings.get("date_format") or "short"
    d, m, y = moment.day, moment.month, moment.year
    if style == "iso":
        return f"{y:04d}-{m:02d}-{d:02d}"
    if style == "us":
        return f"{m}/{d}/{y}"
    if style == "eu":
        return f"{d:02d}/{m:02d}/{y}"
    if style == "long":
        return f"{d} {MONTHS[m - 1]} {y}"
    return f"{DAYS[moment.weekday()]} {d} {MONTHS[m - 1]}"


def format_week(moment):
    return str(moment.isocalendar()[1])


def parse_moment(text):
    """An aware datetime for "2026-12-24T18:00", "2026-12-24 18:00+01:00" or "2026-12-24".

    Without an offset it's the computer's local time."""
    raw = (text or "").strip()
    if not raw:
        raise ValueError("enter a date and time, e.g. 2026-12-24T18:00")
    if raw.endswith(("Z", "z")):
        raw = raw[:-1] + "+00:00"
    try:
        moment = datetime.datetime.fromisoformat(raw)
    except ValueError:
        raise ValueError(f"“{text}” isn't a date and time like 2026-12-24T18:00") from None
    if moment.tzinfo is None:
        moment = moment.replace(tzinfo=LOCAL_TZ) if LOCAL_TZ is not None else moment.astimezone()
    return moment


def format_countdown(target, moment):
    seconds = int((target - moment).total_seconds() + 0.999)  # 4:59.2 left shows 5:00
    if seconds <= 0:
        return "now"
    if seconds < 3600:
        return f"{seconds // 60}:{seconds % 60:02d}"
    minutes = seconds // 60
    if minutes < 24 * 60:
        return f"{minutes // 60}h {minutes % 60}m"
    hours = minutes // 60
    return f"{hours // 24}d {hours % 24}h"


def days_until(text, moment):
    """Whole days from today (local) to a date: "2026-12-24", or "12-24" every year."""
    raw = (text or "").strip()
    today = local(moment).date()
    yearly = YEARLY.fullmatch(raw)
    if yearly:
        month, day = int(yearly.group(1)), int(yearly.group(2))
        for year in range(today.year, today.year + 9):  # 9 years covers 29 February
            try:
                target = datetime.date(year, month, day)
            except ValueError:
                if month > 12 or day > 31 or (month, day) != (2, 29):
                    raise ValueError(f"“{text}” isn't a month and day like 12-24") from None
                continue
            if target >= today:
                return (target - today).days
        raise ValueError(f"“{text}” isn't a month and day like 12-24")
    try:
        target = datetime.date.fromisoformat(raw[:10])
    except ValueError:
        raise ValueError(f"“{text}” isn't a date like 2026-12-24 (or 12-24 for every year)") from None
    return (target - today).days


# --- values ----------------------------------------------------------------------


def compute(value_id, arg, moment, settings):
    if value_id == "time":
        return format_time(zoned(arg, moment), settings)
    if value_id == "date":
        return format_date(zoned(arg, moment), settings)
    if value_id == "week":
        return format_week(zoned(arg, moment))
    if value_id == "countdown":
        return format_countdown(parse_moment(arg), moment)
    if value_id == "days_until":
        return str(days_until(arg, moment))
    raise LookupError(f"unknown value {value_id}")


def wanted():
    """(value id, arg or None) for everything to publish right now."""
    items = [(v, None) for v in FIXED]
    for value_id in ARG_VALUES:
        items.extend((value_id, arg) for arg in plugin.watched(value_id))
    return items


def tick(moment=None):
    """Publish every value whose text changed since it was last sent."""
    moment = now() if moment is None else moment
    settings = plugin.settings or {}
    for value_id, arg in wanted():
        full = value_id if arg is None else f"{value_id}:{arg}"
        try:
            text = compute(value_id, arg, moment, settings)
        except (LookupError, ValueError) as e:
            text = UNKNOWN
            with _lock:
                first = full not in _hinted
                _hinted.add(full)
            if first:
                plugin.log(f"{full}: {e}", "warn")
        with _lock:
            if _published.get(full) == text:
                continue
            _published[full] = text
        plugin.set_value(value_id, text, arg=arg)


@plugin.every(TICK)
def poll():
    tick()


@plugin.on_watch
def watching(values):
    keep = set(values) | set(FIXED)
    with _lock:
        for full in [k for k in _published if k not in keep]:
            del _published[full]
        for full in [k for k in _hinted if k not in keep]:
            _hinted.discard(full)
    tick()


@plugin.on_settings
def configure(_settings):
    with _lock:
        _published.clear()  # 12h/24h or the date style may have changed; the next tick resends


if __name__ == "__main__":
    plugin.run()
