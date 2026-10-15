# Audio devices

Mute the microphone, change the volume, switch between speakers and a headset, and
set the volume of a single app, on the computer running PerritoDeck. Keys and
sliders can follow the current state (a mic key that shows when you're muted).

## Setup

- **Windows**: nothing to install (it uses Windows' own Core Audio).
- **Linux**: PipeWire with WirePlumber (`wpctl`), or PulseAudio's `pactl`
  (package `pulseaudio-utils`; on PipeWire systems it comes with `pipewire-pulse`).
  Per-app volume prefers `pactl`, so install it on PipeWire too if you can.
- **macOS**: volume and mute work out of the box. Switching devices needs the free
  `SwitchAudioSource` tool: `brew install switchaudio-osx`. Without it those keys
  show a warning saying so.

## Actions

| Action | Fields |
|---|---|
| Mute the microphone | mute or unmute, mute, unmute |
| Mute the sound | mute or unmute, mute, unmute |
| Volume up / Volume down | by how much, out of 100 (default 5) |
| Volume (slider) | – |
| Switch speakers or headphones | a device name, or several separated by commas to move to the next one on each press |
| Switch microphone | the same, for microphones |
| Volume of an app (slider) | the app's program name, e.g. `Spotify`, `Discord`, `firefox` |
| Mute an app | the app's program name; mute or unmute, mute, unmute |

Device names don't have to be complete: `headphones` finds "Headphones (G435
Wireless)". A name that matches exactly wins over one that only contains it. When
several names are given and the current device is one of them, the key moves to the
next; otherwise it picks the first. Names it can't find are shown as a warning.

An app only has a volume while it's playing sound; until then its keys warn "isn't
playing any sound right now". On Windows the name is the program file (`Spotify` or
`Spotify.exe`); on Linux it's the process name or the app name PulseAudio/PipeWire
shows (`pactl list sink-inputs`, or the Streams list in `wpctl status`).

## Values

| Title | Shows |
|---|---|
| `{plugin:dev.perritodeck.audio:mic_muted}` | `on` / `off` |
| `{plugin:dev.perritodeck.audio:output_muted}` | `on` / `off` |
| `{plugin:dev.perritodeck.audio:volume}` | `0`–`100`, for a slider to follow |
| `{plugin:dev.perritodeck.audio:output_device}` | `Speakers (Realtek(R) Audio)` |
| `{plugin:dev.perritodeck.audio:input_device}` | `Yeti Stereo Microphone` |
| `{plugin:dev.perritodeck.audio:app_volume:Spotify}` | `0`–`100`, or `–` when the app isn't playing |

Values are read every 2 seconds and sent only when they change; `–` means it can't be
read (no microphone, say). App volumes are only read for apps a key shows. The other
values are always read, because the host only says which values with an argument
are in use.

## Per-OS details and limits

| | Windows | macOS | Linux |
|---|---|---|---|
| Volume and mute | Core Audio (`IAudioEndpointVolume`) on the default device | `osascript` (`get volume settings`, `set volume`) | `wpctl` on `@DEFAULT_AUDIO_SINK@`, or `pactl` on `@DEFAULT_SINK@` |
| Microphone mute | the default input's mute | input volume set to 0, and the old level restored on unmute (remembered across restarts) | `wpctl`/`pactl` mute of the default source |
| Switch devices | `IPolicyConfig` (see below), for all three roles: console, multimedia and communications | `SwitchAudioSource` (optional) | `wpctl set-default <id>` or `pactl set-default-sink/source` |
| App volume | audio sessions (`IAudioSessionManager2`, `ISimpleAudioVolume`) on every active output | not possible: warns | `pactl` sink inputs, or `wpctl` streams |

- **Windows `IPolicyConfig`** is undocumented: Microsoft never published it, but it's
  what the Sound control panel uses, its GUID and layout have stayed the same since
  Windows 7, and many tools rely on it. A future Windows update could break device
  switching; everything else uses documented interfaces. The COM calls are made with
  `ctypes` in `win_audio.py`.
- **macOS**: some outputs (HDMI, many USB DACs) have no volume macOS can change; the
  volume keys warn and the values show `–`. Volume up/down doesn't unmute.
- **Linux**: volumes above 100 % (PipeWire/PulseAudio allow boosting) show as 100.
  `pactl get-default-sink` needs PulseAudio 15 or later (or pipewire-pulse).

## What's tested and what isn't

- **Run on Windows 11** (2026-10-12, in the desktop app with a tablet): the volume and
  the microphone mute, through Core Audio, set from keys and put back afterwards. Not
  tried on Windows yet: muting the sound, switching devices (`IPolicyConfig`), and an
  app's volume or mute.
- **Linux parsing** is checked against real output: `wpctl status` from two real
  PipeWire 1.2 systems and a real `pactl list sink-inputs`, as posted publicly. The
  `pactl list sinks`/`sources` samples and the macOS and `SwitchAudioSource` outputs
  are written from their documented formats.
- **Tests only**, not on real hardware: the Linux and macOS command lines (checked
  against a fake command runner), and the rest of the Windows Core Audio code. Its
  tests check the GUIDs against their byte layout, the vtable indexes against the
  Windows headers' method order, and drive every COM call through fake objects with
  real vtables of ctypes callbacks (so method indexes, argument types, out-pointers and
  releases are exercised).
