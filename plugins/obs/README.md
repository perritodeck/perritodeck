# OBS Studio

Control OBS Studio from PerritoDeck and show its status on keys: scenes, Studio Mode,
transitions, streaming, recording, the replay buffer, the virtual camera, audio,
sources, filters, screenshots, media, browser and text sources, projectors, profiles
and scene collections. Plugin id: `dev.perritodeck.obs`.

It talks to OBS over **obs-websocket 5**, which is built into **OBS 28 and newer**.
It keeps one connection open, reconnects by itself (retrying every few seconds, up to
every 10 s) and updates values the moment OBS reports a change. Only the statistics
and timecodes are polled, every 2 seconds while connected.

Python 3.8+ standard library only; works on Windows, macOS and Linux.

## Setup

1. In OBS, open **Tools → WebSocket Server Settings** and tick **Enable WebSocket server**.
2. Click **Show Connect Info** to see the port and the password.
3. In PerritoDeck, open **Plugins → OBS Studio** and fill in:
   - **OBS address**: `ws://127.0.0.1:4455` when OBS runs on this computer
     (the default). For OBS on another computer use its address, e.g.
     `ws://192.168.1.20:4455`, and allow port 4455 through that computer's firewall.
   - **Password**: the one from *Show Connect Info* (leave it empty if you turned
     authentication off).
   - **Skip certificate check**: only for a `wss://` address behind a proxy with a
     self-signed certificate. OBS itself only speaks `ws://`.

Changing the settings reconnects straight away. The value `connected` shows whether
the plugin is talking to OBS.

## Actions

| Action | Fields | What it does (obs-websocket request) |
|---|---|---|
| Switch scene | Scene; Switch: *Live, or the preview when Studio Mode is on* / *Always live* / *Always the preview* | `SetCurrentProgramScene` or `SetCurrentPreviewScene` |
| Studio Mode on or off | Turn on or off / Turn on / Turn off | `SetStudioModeEnabled` |
| Transition now (Studio Mode) | Transition (optional), duration in ms (optional) | `TriggerStudioModeTransition`. A transition or duration given here is used for this one transition, then the previous one is put back. |
| Choose the transition | Transition, duration in ms (optional, 50–20000) | `SetCurrentSceneTransition`, `SetCurrentSceneTransitionDuration` |
| Streaming | Start or stop / Start / Stop | `ToggleStream`, `StartStream`, `StopStream` |
| Recording | Start or stop / Start / Stop / Pause or resume / Pause / Resume | `ToggleRecord`, `StartRecord`, `StopRecord`, `ToggleRecordPause`, `PauseRecord`, `ResumeRecord` |
| Split the recording into a new file | | `SplitRecordFile` (OBS 30.2+) |
| Add a chapter marker to the recording | Chapter name (optional) | `CreateRecordChapter` (OBS 30.2+, Hybrid MP4 recordings only) |
| Replay buffer | Start or stop / Start / Stop / Save the replay | `ToggleReplayBuffer`, `StartReplayBuffer`, `StopReplayBuffer`, `SaveReplayBuffer` |
| Virtual camera | Start or stop / Start / Stop | `ToggleVirtualCam`, `StartVirtualCam`, `StopVirtualCam` |
| Mute an audio source | Audio source; Mute or unmute / Mute / Unmute | `ToggleInputMute`, `SetInputMute` |
| Volume of an audio source (**slider**) | Audio source | `SetInputVolume` with `inputVolumeMul = (position / 100)²`, the same curve as PerritoDeck's built-in OBS slider |
| Turn a source's volume up or down | Audio source, step (e.g. `10` or `-10`) | Moves the volume by that many steps on the slider's 0–100 scale |
| Show or hide a source | Scene (blank: the live scene), source; Show or hide / Show / Hide | `GetSceneItemId`, `SetSceneItemEnabled` |
| Turn a filter on or off | Source or scene, filter; Turn on or off / Turn on / Turn off | `SetSourceFilterEnabled` |
| Save a screenshot | Source or scene (blank: what's live), folder (blank: your Pictures folder), PNG/JPEG | `SaveSourceScreenshot`, file name like `Gaming 2026-10-11 20-15-03.png` |
| Press an OBS hotkey | Hotkey name, e.g. `OBSBasic.Screenshot`, `OBSBasic.StartRecording` | `TriggerHotkeyByName` |
| Control a media source | Media source; Play or pause / Play / Pause / Play from the start / Stop / Next / Previous | `TriggerMediaInputAction` |
| Refresh a browser source | Browser source | `PressInputPropertiesButton` with `refreshnocache` |
| Change the text of a text source | Text source, text (`\n` makes a new line) | `SetInputSettings` with `{"text": …}` (Text GDI+ and FreeType 2) |
| Open a projector | Program / Preview / Multiview / A source or scene; screen number (blank: a window) | `OpenVideoMixProjector`, `OpenSourceProjector` |
| Switch profile | Profile | `SetCurrentProfile` |
| Switch scene collection | Scene collection | `SetCurrentSceneCollection` |
| Send a caption on the stream | Caption | `SendStreamCaption` (CEA-608) |
| Send any request (advanced) | Request type, request data as JSON | Any request from the [protocol](https://github.com/obsproject/obs-websocket/blob/master/docs/generated/protocol.md); the answer goes to the value `last_response` |
| Send any request with the slider's value (advanced, **slider**) | Request type, data with `{value}` (0–100) or `{mul}` (the volume multiplier) | e.g. `SetInputVolume` with `{"inputName": "Mic/Aux", "inputVolumeMul": {mul}}`. Mind OBS's ranges: `{value}` is 0–100, which doesn't fit `inputVolumeDb` (-100 to 26) |

"Already streaming", "isn't recording" and similar flash a warning rather than an
error. When OBS refuses a request, the key shows OBS's own explanation, or a plain one
when OBS gives none. A replay buffer action while the replay buffer is turned off in
OBS's settings says so. Names (scenes, sources, filters) must match OBS exactly,
including capital letters.

## Values

Titles show a value with `{plugin:dev.perritodeck.obs:<value>}`, or
`{plugin:dev.perritodeck.obs:<value>:<argument>}` for the ones that take an argument.
On/off values are `on` or `off`, so toggles can follow them; `–` means unknown (OBS
isn't connected, or the name wasn't found). A toggle follows the same name without the
braces (*Choose a value to follow* in the editor fills it in); typed with braces works
too.

| Value | Shows |
|---|---|
| `connected` | `on` while connected to OBS |
| `scene` | The live (program) scene |
| `preview_scene` | The preview scene in Studio Mode (`–` otherwise) |
| `studio_mode`, `streaming`, `recording`, `record_paused`, `replay_buffer`, `virtualcam` | `on` / `off` |
| `transition` | The current transition |
| `profile`, `scene_collection` | The current profile and scene collection |
| `stream_time`, `record_time` | `HH:MM:SS` (`00:00:00` when stopped) |
| `fps` | Frames per second OBS renders |
| `cpu` | OBS's CPU use, in % |
| `memory` | OBS's memory use, in MB |
| `dropped_frames` | Frames OBS skipped because rendering or encoding fell behind (the computer's load), in % of all frames |
| `network_dropped_frames` | Frames the stream dropped because the network couldn't keep up, in % of the frames sent while streaming (`0` when not streaming) |
| `bitrate` | Stream bitrate in kbps (from the bytes sent between two polls) |
| `last_response` | The answer to the last *Send any request* (JSON, up to 500 characters) |
| `scene_active:<scene>` | `on` while that scene is live |
| `scene_preview:<scene>` | `on` while that scene is in the Studio Mode preview |
| `muted:<audio source>` | `on` when muted |
| `volume:<audio source>` | 0–100, the inverse of the volume slider's curve, so a slider can follow it |
| `volume_db:<audio source>` | Volume in dB, e.g. `-6.2` (`-inf` when silent) |
| `item_visible:<scene>/<source>` | `on` when that source is visible in that scene |
| `filter_enabled:<source>/<filter>` | `on` when that filter is on |
| `input_text:<text source>` | The text of a text source (needs OBS 30+ for live updates) |
| `media_playing:<media source>` | `on` while a media source plays |

**Two names in one argument** (`item_visible`, `filter_enabled`): write the scene (or
source) and the source (or filter) separated by `/`, e.g. `Gaming/Webcam`. The first `/`
splits them. If the first name contains a `/`, use `|` instead: `Talk/Show|Webcam`.
Spaces around the separator are ignored.

### Examples

- A recording key: action *Recording → Start or stop*, title
  `REC {plugin:dev.perritodeck.obs:record_time}`, toggle follows
  `plugin:dev.perritodeck.obs:recording`.
- A scene key that lights up when live: *Switch scene → Gaming*, toggle follows
  `plugin:dev.perritodeck.obs:scene_active:Gaming`.
- A mic mute key: *Mute an audio source → Mic/Aux*, toggle follows
  `plugin:dev.perritodeck.obs:muted:Mic/Aux`, title `Mic {plugin:dev.perritodeck.obs:volume_db:Mic/Aux} dB`.
  (Audio source names may contain `/`; arguments for `muted`, `volume`, `volume_db`,
  `input_text` and `media_playing` are the whole name.)
- A music slider: *Volume of an audio source → Desktop Audio*; under *The slider
  follows* choose *Source volume (0–100)* and type `Desktop Audio`. (In a profile file:
  `follow = "volume:Desktop Audio"`.)
- A webcam key: *Show or hide a source → Webcam* (scene blank), toggle follows
  `plugin:dev.perritodeck.obs:item_visible:Gaming/Webcam`.
- A stats key: `{plugin:dev.perritodeck.obs:fps} fps · {plugin:dev.perritodeck.obs:cpu}% · {plugin:dev.perritodeck.obs:bitrate} kbps`.
- A stream health key: `Drops {plugin:dev.perritodeck.obs:network_dropped_frames}%`.
- A blur filter key: *Turn a filter on or off → Webcam / Blur*, toggle follows
  `plugin:dev.perritodeck.obs:filter_enabled:Webcam/Blur`.

## Good to know

- **Screenshots** are written by OBS, so the folder is on the computer that runs OBS.
  With OBS on another computer, fill in a folder on that computer. The default is
  the Pictures folder of this computer (Windows: the Pictures known folder; Linux:
  `XDG_PICTURES_DIR`; macOS: `~/Pictures`).
- **Transition now** only works in Studio Mode; otherwise it warns.
- **Statistics** (`fps`, `cpu`, `memory`, `dropped_frames`, `network_dropped_frames`,
  `bitrate`, timecodes) are polled every 2 s with one batched request while connected.
  The host doesn't say which values without an argument are on screen, so this happens
  whether or not a key shows them.
- If OBS kicks PerritoDeck from its session list, the plugin doesn't reconnect by
  itself (as obs-websocket asks); pressing any OBS key or changing the settings does.
- Stream markers (as on Twitch) aren't offered: obs-websocket 5 has no request for them.
- The built-in OBS integration in PerritoDeck (set up in `integrations.toml`: ten
  actions, a volume slider and the live values `{obs:scene}`, `{obs:recording}` and
  `{obs:streaming}`, read every 2 seconds) works independently of this plugin; you can
  use either or both. This plugin updates the moment OBS reports a change and does much
  more.

## What's tested and what isn't

The tests run against a fake obs-websocket 5 server (`test_plugin.py`): the WebSocket
handshake and framing (masking, 16- and 64-bit lengths, fragments, ping/pong, close
codes), authentication, reconnection, every action's request type and data, events,
batches and polling. Request, event and field names were checked against the
official protocol document. Not yet tried against a real OBS:

- the exact error comments OBS returns (shown as they come);
- `SplitRecordFile` / `CreateRecordChapter` on OBS 30.2+;
- `SaveSourceScreenshot` file paths with non-ASCII folder names;
- `InputSettingsChanged` carrying the full text for Text (GDI+) sources;
- `wss://` through a TLS proxy.
