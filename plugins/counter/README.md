# Counters

Count things with a key press: deaths on stream, laps, cups of coffee. Each counter has a
name, and counts are kept in `counters-state.json` in the plugin's data folder, which the
host makes for each plugin in its settings folder (`perritodeck-host paths` shows the settings folder).
Counts saved by older versions in the PerritoDeck config folder (`%APPDATA%\PerritoDeck`,
`~/Library/Application Support/PerritoDeck` or `~/.config/PerritoDeck`) are moved over the
first time the plugin starts.

## Setup

Nothing to set up. For OBS, fill in **Text file folder for OBS** on the Plugins page: each
counter is then written to `<folder>/<name>.txt` whenever it changes (characters that
can't be in a file name become `_`). Each file is replaced in one step, so OBS never
reads a half-written number. In OBS, add a *Text* source, tick *Read from file*
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
