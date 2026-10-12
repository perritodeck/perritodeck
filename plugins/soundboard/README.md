# Soundboard

Play sound files from a key: an air horn, a jingle, applause. Sounds play on the
computer running PerritoDeck, through players it already has, so there's nothing to
install on Windows or macOS.

## Setup

Nothing to set up. Optionally, on the Plugins page:

- **Output device (optional, Linux only)**: the PipeWire or PulseAudio device to play
  on instead of the default, e.g. `alsa_output.usb-Generic_USB_Audio-00.analog-stereo`
  (`pactl list short sinks` or `wpctl status` shows the names; PipeWire also takes a
  node id). Only `pw-play` and `paplay` use it; Windows and macOS always play on the
  default device.

## Actions

| Action | Fields |
|---|---|
| Play a sound | Sound file (a full path; quotes from "Copy as path" are fine, and so are `~` and `%VARS%`/`$VARS`); volume 0–100 (default 100); when it's already playing: let it finish, stop it, start it again, or play it again on top |
| Stop all sounds | – |

"Stop it" makes the key a play/stop toggle. At most 32 sounds play at once.

## Values

| Title | Shows |
|---|---|
| `{plugin:dev.perritodeck.soundboard:playing:C:\Sounds\airhorn.mp3}` | `on` while that file plays, `off` otherwise, so a toggle can follow it |

Write the file in the title the same way as in the key's action (the comparison
ignores case on Windows).

## How it plays

| OS | Player | Formats | Volume | Output device |
|---|---|---|---|---|
| Windows | In the plugin itself, through winmm's MCI `mpegvideo` device (`mciSendStringW`) | WAV, MP3, WMA, AAC/M4A | yes | default only |
| Windows (fallback) | Windows PowerShell 5.1 with WPF's `MediaPlayer`, if MCI can't play the file | WAV, MP3, WMA, AAC/M4A | yes | default only |
| Windows (last resort) | `System.Media.SoundPlayer`, if `MediaPlayer` fails | WAV | no | default only |
| macOS | `afplay -v` | WAV, MP3, AAC/M4A, AIFF, CAF… | yes | default only |
| Linux | `pw-play`, then `paplay`, then `aplay`, then `ffplay -nodisp -autoexit` | WAV, FLAC, OGG (MP3 with libsndfile 1.1+); `aplay` only WAV; `ffplay` almost anything | yes, except `aplay` | `pw-play --target`, `paplay --device` |

On Linux the first installed player starts; if it can't play the file (it exits with
an error, e.g. an MP3 on an older libsndfile), the next one tries, and the log says
why. Each sound is its own child process, started without a console window on
Windows, and stopping a sound ends that process. On Windows the first try plays
inside the plugin instead (no PowerShell start, so no second of delay), each sound with
its own MCI alias; when MCI can't open a file, the PowerShell player tries it.

## Not tested yet

Real playback on every OS. The tests use stand-in child processes to check starting,
stopping, the four modes, falling back to the next player and the `playing` value,
and check the exact command lines for each OS. In particular the Windows MCI player (tested
against a stand-in for `mciSendStringW`) and the PowerShell script (MediaPlayer driven
by pumping the WPF dispatcher) have only been read, not run;
macOS `afplay` and Linux `pw-play`/`paplay` options are from their manuals.
