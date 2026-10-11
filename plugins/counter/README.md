# Counters

Count things with a key press: deaths on stream, laps, cups of coffee. Each counter has a
name, and counts are kept in `counters-state.json` in the PerritoDeck config folder
(`%APPDATA%\PerritoDeck` on Windows, `~/Library/Application Support/PerritoDeck` on macOS,
`$XDG_CONFIG_HOME/PerritoDeck` or `~/.config/PerritoDeck` on Linux).

## Setup

Nothing to set up. For OBS, fill in **Text file folder for OBS** on the Plugins page: each
counter is then written to `<folder>/<name>.txt` whenever it changes (characters that
can't be in a file name become `_`). In OBS, add a *Text* source, tick *Read from file*
and pick that file.

## Actions

| Action | Fields |
|---|---|
| Count up | name, step (1) |
| Count down | name, step (1), don't go below zero (on) |
| Reset to zero | name |
| Set to a number | name, number |

## Values

| Title | Shows |
|---|---|
| `{plugin:dev.perritodeck.counter:count:deaths}` | `7` (a counter nobody has pressed yet shows `0`) |

## Not tested yet

OBS reading the text file (the tests check the file's contents).
