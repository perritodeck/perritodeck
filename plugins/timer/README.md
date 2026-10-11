# Timers

Countdown timers, stopwatches and a Pomodoro timer on your keys. Timers and stopwatches
have names, so you can run several at once ("tea", "pizza", "stream break").

They keep counting while PerritoDeck or the plugin restarts: the state is saved in
`timer-state.json` in the PerritoDeck config folder (`%APPDATA%\PerritoDeck` on Windows,
`~/Library/Application Support/PerritoDeck` on macOS, `$XDG_CONFIG_HOME/PerritoDeck` or
`~/.config/PerritoDeck` on Linux).

## Setup

Nothing to set up. On the Plugins page you can turn on **Play a sound when a timer or
Pomodoro part ends** (Windows: the system "asterisk" sound; macOS: `afplay` with the Glass
sound; Linux: `paplay`, `pw-play` or `canberra-gtk-play`, whichever exists). Every finish is
also written to the host's log.

## Actions

| Action | Fields |
|---|---|
| Start a countdown | name, minutes, seconds (empty: reuse the last length) |
| Start or pause a countdown | name, minutes, seconds; starts again once finished |
| Pause a countdown | name |
| Reset a countdown | name; stops it at its full length |
| Add or take away time | name, minutes, seconds (negative takes time away) |
| Start or stop a stopwatch | name |
| Stopwatch lap | name; remembers the time since the last lap |
| Reset a stopwatch | name (keeps running if it was running) |
| Start or pause the Pomodoro | work / short break / long break minutes (25/5/15), rounds before a long break (4), start the next part by itself (on) |
| Skip to the next Pomodoro part | |
| Reset the Pomodoro | |

## Values

| Title | Shows |
|---|---|
| `{plugin:dev.perritodeck.timer:remaining:tea}` | `4:59`, or `1:02:05` from an hour up; `–` for an unknown timer |
| `{plugin:dev.perritodeck.timer:running:tea}` | `on`/`off` (a countdown or stopwatch with that name runs) |
| `{plugin:dev.perritodeck.timer:done:tea}` | `on` once the countdown finished, until it's reset or started; good for a toggle that lights up |
| `{plugin:dev.perritodeck.timer:progress:tea}` | 0–100, how far the countdown is (a slider can follow it) |
| `{plugin:dev.perritodeck.timer:stopwatch:run}` | `12:34` |
| `{plugin:dev.perritodeck.timer:lap:run}` | the last lap, `–` before the first |
| `{plugin:dev.perritodeck.timer:pomodoro_phase}` | `Ready`, `Work`, `Short break`, `Long break` |
| `{plugin:dev.perritodeck.timer:pomodoro_remaining}` | `24:59` |
| `{plugin:dev.perritodeck.timer:pomodoro_round}` | `2/4` |
| `{plugin:dev.perritodeck.timer:pomodoro_running}` | `on`/`off` |
| `{plugin:dev.perritodeck.timer:pomodoro_progress}` | 0–100 for the current part |

Values tick once a second only while something runs.

## Not tested yet

The end-of-timer sounds on real Windows, macOS and Linux desktops (the tests only check
which command would run).
