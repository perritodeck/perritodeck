# Clocks

The time and date here or anywhere in the world, countdowns, days until a date and the
ISO week number. Everything comes from the computer's own clock: nothing goes over the
network, and there's nothing to set up.

This plugin only has values; there are no actions. Put a value in a key's title.

## Settings

On the Plugins page:

- **Time format**: 24-hour (`15:07`) or 12-hour (`3:07 PM`).
- **Date format**: `Sun 11 Oct`, `11 Oct 2026`, `2026-10-11`, `10/11/2026` or `11/10/2026`.

## Values

| Title | Shows |
|---|---|
| `{plugin:dev.perritodeck.clock:time}` | the time here, `15:07` |
| `{plugin:dev.perritodeck.clock:time:Asia/Tokyo}` | the time in Tokyo, `23:07` |
| `{plugin:dev.perritodeck.clock:time:UTC+5:30}` | the time at a fixed offset from UTC |
| `{plugin:dev.perritodeck.clock:date}` | today's date here, `Sun 11 Oct` |
| `{plugin:dev.perritodeck.clock:date:America/New_York}` | today's date in New York |
| `{plugin:dev.perritodeck.clock:week}` | the ISO week number, `41` (also takes a time zone) |
| `{plugin:dev.perritodeck.clock:countdown:2026-12-24T18:00}` | `12d 4h`, `3h 12m`, then `4:59` in the last hour, then `now` |
| `{plugin:dev.perritodeck.clock:days_until:2026-12-24}` | `74`; `0` on the day, negative after it |
| `{plugin:dev.perritodeck.clock:days_until:12-24}` | days until the next 24 December, every year |

Time zones are IANA names (`Europe/Lisbon`, `America/Los_Angeles`, `Australia/Sydney`),
`UTC`, `local`, or a fixed offset (`UTC+5:30`, `GMT-3`, `+09:00`). Fixed offsets don't
follow daylight saving time.

A countdown's date and time is in the computer's time zone unless it ends with an offset
(`2026-12-24T18:00+01:00`) or `Z` for UTC. A date alone (`2026-12-24`) counts down to
midnight.

Values are sent when their text changes: once a minute for clocks, and every second for
countdowns in their last hour. Anything that can't be worked out (a misspelt time zone, a
date in the wrong shape) shows `–`, and the host's log says why.

## Time zones on Windows

Time zone names need Python 3.9 or newer and a time zone database. Linux and macOS have
one; Windows doesn't, so there zone names show `–` until Python's `tzdata` package is
installed (`py -m pip install tzdata`). Fixed offsets like `UTC+5:30` and the local time
always work.

## Not tested yet

Nothing here talks to a service. The tests cover formats, time zones (including a
missing time zone database), countdowns and the watch list with a fake clock; they
haven't been run on Windows without `tzdata` installed, which is mocked.
