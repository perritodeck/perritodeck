# OBS Studio: real-world test results, 2026-10-13

This is the first run of the [OBS test plan](obs.md) against a real OBS. Until now, both
ways PerritoDeck controls OBS, the built-in integration and the OBS plugin
(`dev.perritodeck.obs`), had run only against fake obs-websocket servers.

> **Since this run:** all 22 bugs below were fixed (OBS plugin 1.0.3) and re-checked
> against the same OBS on 2026-10-15. Every re-check passed; the list is in the test plan,
> [§1, Found by the first pass](obs.md#found-by-the-first-pass-2026-10-13).

**In short**
- **Coverage:** 93 of the plan's 110 cases ran. 84 passed (14 of them only in part), 9
  failed, 6 were blocked and 11 were not run.
- **§1 re-checks:** every one that a real OBS could exercise held. The new 604 hint, the
  status-code wording, the backoff, the "don't resend a timed-out toggle" rule, the 4009
  hint and the stale-failure fix all behave as intended.
- **Most important bugs:**
  - In the plugin, a one-off transition can change the user's OBS transition duration for
    good.
  - In the plugin, "pause recording" can show success when nothing paused.
  - In the plugin, the `recording` value drops to off on every pause and flickers during
    every stop.
  - In the built-in integration, live values reach a tablet up to about 3 s after OBS
    changes, over the plan's 2.5 s target.
- **Bugs found:** 22 in all, 7 should-fix and 15 nice-to-have. None blocks a release. See
  [Bugs](#5-bugs-ranked-by-severity).

## 1. Setup

**Software**
- OBS Studio 31.0.3 with obs-websocket 5.5.6 on Windows 11, on this PC. Authentication was
  on and the port was 4455.
- PerritoDeck at main (`71922d3`):
  - the desktop app (release build);
  - the command-line host and fakedeck, built from the same commit;
  - the OBS plugin 1.0.2 from `plugins/obs/`.
- `obs-websocket-compat` (plan §2.1 step 4): OBS doesn't load it. Instead it opens a modal
  "Plugin Load Error" box at every start. While that box is open, OBS ignores a polite
  close, so the harness dismissed it each time.

**Harness**

The harness is scratch tooling outside the repository, as plan §5 says. It ran in four
layers:
- **Independent checker.** A small obs-websocket 5 client using only the standard library,
  written separately from the plugin. Every effect was checked against OBS's own state and
  events, not just against what PerritoDeck said.
- **Layer A.** `plugins/obs/plugin.py` runs as a subprocess speaking plugin protocol 1,
  with no host.
- **Layer B.** A fake tablet. fakedeck was paired with the desktop app as a device of its
  own, with a 5×3 test profile assigned only to it. A harness client using the same
  pairing also announced `sliders`. That got around the plan's "fakedeck has no slider
  command", so D1, D2, D6 and D7 ran without a tablet.
- **Layer C.** The desktop app over DevTools: `test_action`, `get_diagnostics`, and the
  editor in the real window.

**Different from the plan (§2.4)**
- The pass drove the running desktop app and its own data folder, not a scratch
  `PERRITODECK_DATA_DIR`. The maintainer's tablet stayed connected on its own profile and
  was never touched.
- As a result, the app's `integrations.toml` (which holds the real password) was used as
  it was and never edited.
- Cases that need a different `integrations.toml` ran on scratch command-line hosts
  instead. Those hosts used their own data folders and spare ports and never held the real
  password, but they run the same client code as the app. The cases were: no password, a
  wrong password, a typo, address forms and an address that never answers.
- The OBS plugin was not installed in the app. Every plugin case therefore ran at Layer A
  only, and the cases that need the host to run the plugin were blocked.

**Fixtures**

The fixtures were built through obs-websocket requests only. Some names differ from the
plan:

| Plan | This pass |
|---|---|
| Scenes `PD A`, `PD B`, `PD C` | `PD Main`, `PD Second`, `PD BRB` (`PD BRB` is also nested in `PD Second`) |
| `PD Mic` | `PD Tone`: a media source playing a 5 s WAV, used as the audio source for volume and mute |
| `PD CC` | `PD Filter`, a colour filter on `PD Color` |
| `PD Media` | `PD Tone`, plus `PD Slides` (a slideshow of 2 images, manual) |
| Desktop Audio | None: a new scene collection has no global audio sources |
| A group | None: obs-websocket can't create one |
| Not in the plan | `PD A/V`, an audio source with a slash in its name |

The rest matches the plan:
- **Scenes:** `Escena ñ 🎮` and `Talk/Show, live`.
- **Sources:** `PD Text` (GDI+) and `PD Text FT` (FreeType 2); `PD Browser`, a local page
  whose reloads a local server counted.
- **Profile `PD Test`:**
  - Advanced output, recording to a scratch folder;
  - Hybrid MP4, with file splitting set to manual;
  - replay buffer of 20 s;
  - Fade 300 ms.
- **Profile `PD Test2`.**
- **Scene collection `PD Big`:** 41 scenes.

**Safety**
- **Test profile only:** the harness refused to change OBS unless both the scene collection
  and the profile were `PD Test`.
- **No public stream:** it refused to start a stream unless the server was a custom RTMP
  server on 127.0.0.1. That was `rtmp://127.0.0.1:1935/live` with a dummy key, and nothing
  listens there. Five stream starts went to it, and none went live.
- **OBS process:** OBS was only ever closed politely, never killed. It was suspended twice,
  for 7 s each.
- **Password:** never printed. Scans of every harness file, the app's log with detailed
  logs on, Copy diagnostics and the plugin's output found it 0 times.
- **Clean-up afterwards:**
  - OBS was switched back to the maintainer's own profile and scene collection.
  - The `PD Test` and `PD Test2` profiles and the `PD Test` and `PD Big` collections are left
    to remove by hand (obs-websocket can't remove a scene collection).
  - The fakedeck device was revoked, the test profile deleted and the recordings removed.

**Not covered, and why**

| What | Why | Cases |
|---|---|---|
| A stream that goes live | No live target was approved for this pass (a Twitch bandwidth-test key or a local RTMP server needs the maintainer) | B6, C4, the rest of E5, E24 while streaming, F-stats bitrate and network drops, I6 |
| The OBS plugin through the host | The plugin wasn't installed in the app | The host half of every E case, F-follow, D4, K1, K6, the tablet half of D3, the plugin slider in D6, plugin I1 through a tablet |
| Settings only in OBS's window | obs-websocket can't change them, and they would cut every other session | A4, A6 (server off), H6, H7, H8 |
| Killing OBS | Polite closes only, so OBS's configuration and Safe Mode prompt stay out of it | H3 |
| The app's own `integrations.toml` and restarting the app | Editing the file or restarting the app would disturb the maintainer's setup and tablet | The app's halves of H12 (Settings text, Companion section), the host half of H10 |
| A second computer, OBS as administrator, a real tablet by hand | Manual | A9, J4, H11, D8 |
| Auto-switch | It would change the app's own auto-switch settings | L2 |
| Full-screen projectors; screenshots into Pictures | They would cover the screens, and Pictures is cloud-synced on this PC | Parts of E21, E16 |
| Long runs | I4 used 60 s windows instead of 30 minutes; there was no 2-hour soak | I4, I6 |
| Linux and macOS | Windows only this time | All |

## 2. §1 re-checks

| §1 fix | Case | On a real OBS |
|---|---|---|
| Built-in status-code wording | B5, B10 | Holds. OBS sends 500 and 501 without a comment, and the built-in texts are what users see. With the buffer off, OBS sends 604 "Replay buffer is not available.", which gets the "turned off" hint |
| Built-in backoff, no lock held while connecting | H5, I2, I3 | Holds: 5, 10, 20 s, then one attempt every 30 s. `{cpu}` keeps its 2 s rhythm with OBS closed, and presses never wait behind a poll. With an address that never answers, each attempt still freezes the live loop for 5 s (bug 10) |
| Only `Get…`/`Set…` requests are resent | H4, I5 | Holds: a toggle that timed out while OBS hung reached OBS exactly once |
| 4009 wrong-password hint | A2 | Holds on the command-line host: "OBS: OBS rejected the password in integrations.toml" |
| `{obs:scene}` falls back to `sceneName` | C1 | Not exercised: OBS 31.0.3 still sends `currentProgramSceneName` as well |
| Braced follow | C7, F-follow | Holds for the built-in path. The plugin half was blocked |
| A mistake in `integrations.toml` shows in Settings, diagnostics and the log | H12 | Diagnostics, the banner and the log hold on the command-line host; the app's Settings text wasn't run. OBS keys and sliders still say "add it" (bug 7) |
| `perritodeck-host action` reads `integrations.toml` | B (command line) | Holds |
| Separate sentences for a timeout and for OBS closing | H4, H2 | The timeout sentence holds, but reads "OBS: OBS …" (bug 17). The closing sentence couldn't be provoked, because OBS answered every request before it closed |
| A numeric password is never quoted | H12 | Holds: `12345678` appears nowhere |
| A press ignores a poll's failure from before it | H1 | Holds: a press 2 ms after OBS started listening got OBS's own 207 answer, not "can't reach OBS" |
| Saving a profile with a braced follow reloads it once | C7 | Holds: one layout per save |
| The Companion section shows the mistake | H12 | Not run (the app's `integrations.toml` wasn't edited) |
| Plugin slider follow with an argument in the editor | D4 | Blocked (plugin not installed in the app) |
| Plugin slider follow cleared when "Slider sets" changes | K6 | Blocked (plugin not installed in the app) |
| README `request-value` example uses `inputVolumeMul: {mul}` | D5 | Holds: it gives exactly the D1 curve |
| README lists the built-in values | K5 | Holds |
| Plugin 604 "turned off" hint | E9 | Holds: real OBS sends 604 "Replay buffer is not available." for toggle, start, stop and save, and the plugin shows the hint for each |
| `network_dropped_frames` from `GetStreamStatus` | F-stats | Holds while not streaming (0, matching `GetStreamStatus`). Not tested during a stream |

**Known and not being fixed (§1)**
- **`wss://` on the built-in path:** fails with "OBS: can't reach OBS at
  wss://127.0.0.1:4455: URL error: TLS support not compiled in (is the WebSocket server
  enabled?)".
- **The plugin's generic wording:** the only refusal seen without a comment that the plugin
  doesn't word itself was 600, for a missing profile or scene collection. It reads "OBS:
  it couldn't find that" (bug 15). Every other refusal came with OBS's own comment.

## 3. Results by section

**Result values**
- **Pass (partial):** part of the case could not run; the note says which part.
- **Blocked:** the case needed something this pass didn't have.
- **Not run:** a Semi or Manual case left on purpose.

**Bug numbers** refer to [§5](#5-bugs-ranked-by-severity).

### A. Connection and password

| ID | Case | Result | Note |
|---|---|---|---|
| A1 | Right password | Pass | Built-in key ✓ in 27 ms; the plugin's `connected` was `on` 0.2 s after it started. The app's and the plugin's sessions were open on OBS together (L1). OBS's session list wasn't checked by eye |
| A2 | Wrong password | Pass | Built-in (command-line host): "OBS: OBS rejected the password in integrations.toml". Plugin: "OBS rejected the password: check it in this plugin's settings (in OBS: Tools → WebSocket Server Settings)", logged once; it reconnected 0.02 s after the right password arrived. Backoff: H5 |
| A3 | No password | Pass (partial) | Built-in: "OBS: OBS needs a password; add it to integrations.toml". The plugin half wasn't run |
| A4 | Authentication off in OBS | Not run | Semi: a setting in OBS's window |
| A5 | Address forms | Pass (partial) | `ws://localhost:4455` and `ws://[::1]:4455` reach OBS. With no `ws://`, the built-in says "URL error: URL scheme not supported (is the WebSocket server enabled?)" (bug 14). The plugin's normalisation wasn't run |
| A6 | Wrong port; server off | Pass (partial) | Built-in: "…actively refused it. (os error 10061) (is the WebSocket server enabled?)" after 2.06 s. Plugin: "couldn't reach OBS at ws://127.0.0.1:4456: connection refused. Is OBS open, with its WebSocket server on…?". The "server off" half wasn't run (it needs OBS's window) |
| A7 | Plugin settings changed while connected | Pass | Back from a wrong port: `connected` was `on` after 48 ms, and every value was back 1 ms later |
| A8 | `wss://` | Pass (partial) | The built-in message is recorded in §2. The plugin through a TLS proxy wasn't run |
| A9 | OBS on another computer | Not run | Manual |

### B. Built-in actions

| ID | Case | Result | Note |
|---|---|---|---|
| B1 | `scene` = PD Second | Pass | test_action answered in 27 ms; the event arrived at the end of the 300 ms Fade. The same from a key |
| B2 | `scene` = Missing | Pass | OBS sends 600 "No source was found by the name of \`Missing\`.", shown as "OBS: No source was found…" on test_action, the key's alert and Try it |
| B3 | Non-ASCII and slash-and-comma names | Pass | Both switch, and `{obs:scene}` shows them |
| B4 | `toggleRecord` twice | Pass | Exactly one STARTING/STARTED and one STOPPING/STOPPED; the file was written |
| B5 | `startRecord` while recording, `stopRecord` while stopped | Pass | OBS sends 500 and 501 without a comment, so the keys show "OBS: already recording" and "OBS: not recording". Never "request failed" |
| B6 | Stream start and stop with a live target | Blocked | No live target |
| B7 | `startStream` to an unreachable server | Pass | Recorded. The key and test_action show success. OBS goes STARTING → STOPPED after about 2.03 s and opens its own "Failed to connect" dialog. `{obs:streaming}` and the follow key stay off, so nothing on the tablet says the stream failed (bug 8) |
| B8 | `toggleMute` | Pass | Flips both ways; a missing input gets OBS's 600 comment |
| B9 | `toggleReplayBuffer`, `saveReplayBuffer` | Pass | The buffer runs, ReplayBufferSaved arrives and the file is written, from test_action and from keys |
| B10 | Replay buffer stopped / turned off | Pass | Stopped: 501, no comment, shown as "OBS: the replay buffer isn't running". Turned off: 604, shown as "OBS: the replay buffer is turned off (Settings → Output → Replay Buffer)" |
| B11 | Every op from a key | Pass | Every effect showed in OBS. Each refusal arrived as an alert with the same text as test_action, 14–31 ms after the key went down |
| B (command line) | `perritodeck-host action` | Pass | Extra §1 check. Scene switches work, and `stopRecord` while stopped prints "alert: OBS: not recording" |

### C. Built-in live values and follow

| ID | Case | Result | Note |
|---|---|---|---|
| C1 | `{obs:scene}` title | Pass | 26 samples after OBS's event: p50 1.91 s, p95 2.48 s, max 2.51 s. OBS 31 answers with both `currentProgramSceneName` and `sceneName` |
| C2 | `{obs:recording}`, `{obs:streaming}` | Fail | `{obs:recording}` took 1.81–2.90 s, over the 2.5 s target (bug 5). `{obs:streaming}` never had a reason to turn on |
| C3 | Follow `obs:recording` | Pass | REC-ON and REC-OFF flip together with the title, with the same latency |
| C4 | Follow `obs:streaming` | Blocked | No live stream |
| C5 | Close OBS | Pass | Titles showed "–" 1.19 s after OBS closed, and follow keys were in their first state. Diagnostics said "set up, but not reachable (its values show –)". Everything came back after the restart |
| C6 | First 2 s after the host starts | Pass | Recorded: yes. A tablet that connects 0.33 s after start sees the raw `{obs:scene}`, `{obs:recording}`, `{obs:streaming}` and `{cpu}` until the first poll (bug 9) |
| C7 | Braced follow and saving | Pass | The braced follow flips with the plain one; each save sends exactly one layout |
| C8 | Diagnostics line | Pass (partial) | Seen: "set up and answering" and "set up, but not reachable". "Not checked yet" can't appear while the test profile stays loaded. "Not set up" wasn't run in the app |

### D. Sliders

| ID | Case | Result | Note |
|---|---|---|---|
| D1 | Built-in slider curve | Pass | 0, 25, 50 and 100 give `inputVolumeMul` 0, 0.0625, 0.25 and 1.0, exactly (value/100)² |
| D2 | Fader moved in OBS | Pass | The tablet's slider followed in 2.0–3.0 s (target 3.5 s). Once, a tablet that reconnected first showed a stale level (bug 11) |
| D3 | Plugin `volume` slider following `volume:PD Tone` | Pass (partial) | Same curve as D1, and `volume:PD Tone` followed fader changes within 0.1 ms of OBS's event. The tablet half needs the plugin in the app |
| D4 | Editor: follow `volume:<source>` | Blocked | Plugin not installed in the app |
| D5 | `request-value` with `inputVolumeMul: {mul}` | Pass | Gives the D1 curve. `{value}` sent to `inputVolumeDb` fails above 26 with OBS's own message |
| D6 | 100 changes in 2 s | Pass (partial) | Built-in slider: it ended at exactly 0.37², with nothing queued afterwards and no jump back. The plugin slider was blocked |
| D7 | Drag with OBS disconnected | Pass | The slider showed alerts while OBS was closed, and no burst of changes followed after reconnecting |
| D8 | Feel on a real tablet | Not run | Manual |

### E. Plugin actions

Every E case ran at Layer A only. The host half (`test_action` or a key with the plugin
installed) wasn't run, because the plugin wasn't installed in the app.

| ID | Case | Result | Note |
|---|---|---|---|
| E1 | `scene` | Pass | `auto`, `program` and `preview`, with Studio Mode off and on. `preview` with Studio Mode off gives "OBS: Studio Mode is off" (506), and a missing scene gets OBS's 600 comment. Non-ASCII and slash names work |
| E2 | `studio-mode` | Pass | Toggle, on and off. Turning it on while on is silent in OBS |
| E3 | `transition` | Pass | Fade 300 plays (331 ms); the current transition is unchanged |
| E4 | `set-transition` | Pass | Durations 49 and 20001 are refused in the plugin. An unknown name gives "OBS: No scene transition was found by that name." Cut with a duration changes Fade's duration (bug 1) |
| E5 | `stream` | Pass (partial) | Stop while stopped gives "OBS isn't streaming". One start to the dummy server: the key shows ✓, then OBS stops 2.04 s later (bug 8). Toggle and start-while-streaming need a real stream |
| E6 | `record` | Fail | Start, stop, toggle, pause and resume work when the recording has an encoder of its own. Pause and pause-toggle while stopped, or with OBS's default "Use stream encoder", show ✓ and nothing pauses (bug 2) |
| E7 | `record-split` | Pass | The split lands on the next keyframe (8.3 s at the defaults). With splitting off, OBS answers 702 "Verify that file splitting is enabled in the output settings."; while stopped the plugin says "OBS isn't recording" |
| E8 | `record-chapter` | Pass | Checked with ffprobe: Hybrid MP4 has the chapters (a blank name becomes "Unnamed 2", non-ASCII is kept). MKV gets 702 "Verify that the output being used supports chapter markers." |
| E9 | `replay-buffer` | Pass | Every mode works, and the file is saved. Turned off: 604, and the hint for every mode (§2) |
| E10 | `virtualcam` | Pass | Not opened in a camera app |
| E11 | `mute` | Pass | Including `PD A/V` (slash in the name) |
| E12 | `volume` | Pass | Same curve as D1; 150 is clamped to 100 and −5 to 0 |
| E13 | `volume-step` | Pass | Clamped at 0 and 100, also on a source no key watches; step 150 is refused in the plugin |
| E14 | `source-visibility` | Pass | Toggling a nested scene works. A missing source gives "OBS has no source “PD Nothing” in the scene “PD Main”", but a missing scene gets the same sentence (bug 15). No group to test |
| E15 | `filter` | Pass | Also works on a scene filter. A missing filter gives "OBS: No filter was found in the source \`PD Color\` with the name \`PD Nope\`." |
| E16 | `screenshot` | Pass | PNG and JPEG headers; a non-ASCII folder is created; " (2)" and " (3)" for three in one second. The blank-folder default resolves to Pictures, but nothing was written there |
| E17 | `hotkey` | Pass | An unknown name gives "OBS: No hotkeys were found by that name." (a comment, not silence) |
| E18 | `media` | Pass | play-pause works from every state; the slideshow's next and previous work |
| E19 | `browser-refresh` | Pass | The reload reached the local server 7–9 ms after the key. A non-browser source gives "OBS: Unable to find a property by that name." |
| E20 | `set-text` | Pass | The newline renders; emoji and 2 kB of text work on GDI+ and FreeType 2. GDI+ text also updates live |
| E21 | `projector` | Pass (partial) | Four windowed projectors open. Screen 9 on a two-screen PC shows ✓ and opens nothing (bug 16). Full-screen projectors weren't opened |
| E22 | `profile` | Pass | OBS switches profiles mid-recording without a dialog, and the recording carries on. A missing profile gives "OBS: it couldn't find that" (bug 15) |
| E23 | `scene-collection` | Pass | Switching to PD Big took 0.08 s and the values resynced. The "still switching" path (over 8 s) wasn't reproduced |
| E24 | `caption` | Pass (partial) | While not streaming: "OBS isn't streaming" (501). The streaming half needs a stream |
| E25 | `request` | Pass | Answers are cut at 500 characters with "…". An unknown type gives "OBS: Your request type is not valid." (204 comes with a comment) |
| E26 | `request-value` | Pass | See D5 |

### F. Plugin values

| ID | Case | Result | Note |
|---|---|---|---|
| F-conn | `connected` | Pass | `on` 0.2 s after start. When OBS closed, it went `off` and every value showed "–" 0.31 s later |
| F-scene | `scene`, `preview_scene`, `studio_mode` | Pass | All three follow OBS's events at once; `preview_scene` is "–" whenever Studio Mode is off |
| F-out | Output values | Fail | `replay_buffer`, `virtualcam` and `streaming` are right. `recording` drops to off on every pause and flickers off, on, off on every stop, and `record_paused` lags 1–2 s (bugs 3, 4) |
| F-names | `transition`, `profile`, `scene_collection` | Pass | Each follows within 0.2 ms of OBS's event |
| F-time | `stream_time`, `record_time` | Fail | `record_time` drops to 00:00:00 on each pause until the next poll, and reads 00:00:00 → 00:00:04 → 00:00:00 during a stop (bugs 3, 4). `stream_time` while streaming couldn't be tested |
| F-stats | Statistics | Pass (partial) | `fps` = canvas (30). `cpu` and `memory` match `GetStats`. `dropped_frames` = render plus output skips since OBS started. `network_dropped_frames` is 0 while not streaming. Bitrate and network drops during a stream weren't tested |
| F-resp | `last_response` | Pass | Set by every successful request; unchanged by failed ones |
| F-args | Values with an argument | Pass | All nine follow OBS within 0.4 ms of its events; misspelled names show "–" |
| F-rename | Renames in OBS | Fail | Renaming the live scene leaves `scene` on the old name (bug 6). The old `item_visible` name keeps following a renamed input (bug 13) |
| F-follow | Toggles following plugin values | Blocked | Following is done by the host, and the plugin wasn't installed in the app. At the plugin, every followable value flips when OBS changes |

### G. Studio Mode

| ID | Case | Result | Note |
|---|---|---|---|
| G1 | On and off, from OBS and from a key | Pass | `scene_preview:*` is off while Studio Mode is off |
| G2 | `scene` auto in Studio Mode | Pass | Only the preview changes |
| G3 | `scene` program in Studio Mode | Pass | The program changes. OBS then swaps the old program into the preview |
| G4 | Built-in `scene` in Studio Mode | Pass | Recorded: it changes the program, with the current transition, and leaves the preview alone. The guide doesn't say so yet (bug 19) |
| G5 | One-off transition | Fail | One-offs over Fade 300 are put back correctly (OBS 31 sends SceneTransitionEnded even for Cut). A one-off Fade 800 over Cut leaves Fade at 800 (bug 1) |
| G6 | `transition` with Studio Mode off | Pass | "Studio Mode is off; there's nothing to transition"; the one-off is put back within 3 ms |
| G7 | Studio Mode off while preview and program differ | Pass | `preview_scene` shows "–" |

### H. Errors, reconnecting and robustness

| ID | Case | Result | Note |
|---|---|---|---|
| H1 | OBS started after the host | Pass | With no key pressed, the built-in path recovered 31.35 s after OBS started listening (the 30 s backoff plus a poll), and the plugin in 5.9 s. A key pressed 2 ms after OBS listened got OBS's own 207 "OBS is not ready to perform the request.", not a stale "can't reach OBS"; the next press, 0.5 s later, worked |
| H2 | File → Exit mid-session | Pass | The plugin logs "OBS was closed" and its values show "–"; built-in titles show "–" 1.19 s after. While exiting, OBS answers 207 and answers every request before it closes, so the "closed the connection before answering" sentence couldn't be provoked |
| H3 | Kill OBS | Not run | Polite closes only. The plugin's "lost the connection" path ran in H4 |
| H4 | OBS suspended for 7 s | Pass | Built-in: "OBS: OBS didn't answer in time, so it may or may not have happened" at 5.02 s, and OBS started recording exactly once after it resumed (not resent). Plugin: "OBS stopped answering; reconnecting", then reconnected when OBS resumed. Nothing waited past 10 s |
| H5 | Wrong password for 5 minutes | Pass | Built-in (command-line host): attempts after 5, 10 and 20 s, then one every 30–31 s (13 sessions, 2 of them from key presses). Plugin: 0.5 s doubling to a 10 s cap, with the warning logged once. `{cpu}` kept 2 s |
| H6 | WebSocket server off and on | Not run | Semi |
| H7 | Kick from OBS's session list | Not run | Semi |
| H8 | Port changed in OBS | Not run | Semi. The plugin's reconnect on a settings change is A7 |
| H9 | Scene collection switched from OBS's side | Pass | Both paths resync. A key naming a scene that isn't in PD Big gets "OBS: No source was found by the name of \`PD Second\`." |
| H10 | Restart while recording | Pass (partial) | Plugin restarted: `recording` was `on` 0.2 s later, with no stray toggles. The host restart was blocked |
| H11 | OBS as administrator | Not run | Manual |
| H12 | Mistakes in `integrations.toml` | Fail | Command-line host: the banner, diagnostics ("integrations.toml has a mistake and is ignored") and the log name line 3 without quoting it, and `12345678` appears nowhere. But OBS keys and sliders say "OBS isn't set up; add it to integrations.toml…" (bug 7). The app's halves weren't run |
| H13 | Unicode, `/`, `:`, `\|` in names | Fail | Unicode, `/` and `:` work. A source whose name contains `\|` needs the `\|` form, though the README's rule says `/` (bug 12) |

### I. Performance

| ID | Case | Result | Note |
|---|---|---|---|
| I1 | OBS change → tablet title | Fail | Built-in `{obs:scene}`, 50 samples: p50 1.73 s, p95 2.57 s, max 2.68 s (target p95 ≤ 2.5 s; bug 5). Plugin at Layer A: p95 0.2 ms (target 300 ms); through a tablet it wasn't measured |
| I2 | Live-loop stall | Pass | OBS closed: `{cpu}` updated 31 times at 2.0 s and 3 times at 4.0 s. An address that never answers: mostly 2 s, but a 5 s freeze at each backoff attempt (bug 10) |
| I3 | Key press → OBS | Pass | OBS up: 2–33 ms. OBS down: the alert comes after the press's own attempt (2.0 s when refused, 5.0 s when there's no answer), never queued behind a poll |
| I4 | CPU and memory | Pass (partial) | 60 s windows instead of 30 min. The app used about 0.9 % of one core, with or without the fake tablet. The plugin with 40 values used 0.13 %. Memory stayed flat, and OBS showed no visible cost. A 300-event flood: p95 0.3 ms |
| I5 | 20 toggles in 2 s | Pass | Both paths ended in a consistent state with no crash. OBS absorbs toggles that arrive while it is starting or stopping, so 20 presses made 2 recordings on each path |
| I6 | 2-hour soak | Not run | Needs a stream and a person |

### J. Security and privacy

| ID | Case | Result | Note |
|---|---|---|---|
| J1 | Password in logs and diagnostics | Pass | With detailed logs on, the password appeared 0 times in the app's log and data folder, Copy diagnostics, the plugin's output and every harness file |
| J2 | Settings file permissions | Pass | Windows: only SYSTEM, Administrators and the user (inherited). `write_private` sets no ACL on Windows, so the protection comes from the per-user folder; a data folder elsewhere would not be protected. The OBS plugin's settings file didn't exist (plugin not installed), but it goes through the same code |
| J3 | Exported profile | Pass | Checked by the code and by counting: an export zips only the profile's TOML and its images, and the profiles contain the password 0 times. The export dialog itself wasn't used |
| J4 | Password on the LAN | Not run | Manual. In the code, both clients send only the challenge response |

### K. Editor, Plugins page and docs

| ID | Case | Result | Note |
|---|---|---|---|
| K1 | Plugins page OBS form | Blocked | Plugin not installed in the app |
| K2 | OBS action and slider forms | Pass | All 10 ops are offered, with plain validation messages; nothing is saved while a field is blank |
| K3 | Live value picker | Pass (partial) | `obs:scene`, `obs:recording` and `obs:streaming` are listed. The OBS plugin's values were blocked |
| K4 | Try it | Pass | "Ran, with a warning: OBS: …" in plain words, for every OBS error tried |
| K5 | Guide and README vs real OBS | Pass | The README's §1 changes are right. Both documents need this pass's findings (bugs 18, 19) |
| K6 | Slider follow cleared on switch | Blocked | Plugin not installed in the app |

### L. Both paths at once

| ID | Case | Result | Note |
|---|---|---|---|
| L1 | Built-in and plugin together | Pass | OBS had both sessions open. A built-in scene change reached the plugin's `scene` 0.25 s after test_action answered; a plugin recording start showed on `{obs:recording}` 1.47 s later |
| L2 | Auto-switch to an OBS profile | Not run | Semi, and it would change the app's settings |

## 4. Coverage

| Section | Cases | Pass | Fail | Blocked | Not run |
|---|---|---|---|---|---|
| A. Connection | 9 | 7 | 0 | 0 | 2 |
| B. Built-in actions | 11 | 10 | 0 | 1 | 0 |
| C. Built-in values | 8 | 6 | 1 | 1 | 0 |
| D. Sliders | 8 | 6 | 0 | 1 | 1 |
| E. Plugin actions | 26 | 25 | 1 | 0 | 0 |
| F. Plugin values | 10 | 6 | 3 | 1 | 0 |
| G. Studio Mode | 7 | 6 | 1 | 0 | 0 |
| H. Errors | 13 | 6 | 2 | 0 | 5 |
| I. Performance | 6 | 4 | 1 | 0 | 1 |
| J. Security | 4 | 3 | 0 | 0 | 1 |
| K. Editor and docs | 6 | 4 | 0 | 2 | 0 |
| L. Both paths | 2 | 1 | 0 | 0 | 1 |
| **Total** | **110** | **84** | **9** | **6** | **11** |

**Ran:** 93 of 110 cases.
- **Partial passes:** 14 of the 84 passes ran only in part: A3, A5, A6, A8, C8, D3, D6, E5,
  E21, E24, F-stats, H10, I4 and K3.
- **Plugin actions:** all 26 E cases ran, all 26 actions were exercised and all 30 values
  were watched, but only over the plugin protocol (Layer A).
- **Not counted:** the extra "B (command line)" §1 check isn't counted above.
- **Stage logs:** they recorded the plugin's version log line and the doubled "OBS: OBS"
  prefix as failures under H1 and H2. Here they are bugs 20 and 17. H1 and H2 count as
  passes, because those cases' own expected results were met.

**Timings at a glance**

| Measure | Result | Target |
|---|---|---|
| Plugin value after OBS's event (Layer A, 50 samples) | p50 0.1 ms, p95 0.2 ms, max 0.8 ms | p95 < 300 ms |
| Built-in `{obs:scene}` on a tablet (50 samples) | p50 1.73 s, p95 2.57 s, max 2.68 s | p95 ≤ 2.5 s |
| Built-in `{obs:recording}` on a tablet | 1.81–2.90 s | ≤ 2.5 s |
| Key press → OBS event (built-in) | 2–33 ms | < 100 ms |
| Built-in slider after a fader change in OBS | 2.0–3.0 s | ≤ 3.5 s |
| Plugin connected after starting | 0.2 s | ≤ 1 s |
| Recovery after OBS restarts, no key pressed | built-in 31.35 s, plugin 5.9 s | ≤ 30 s, ≤ 10 s (the built-in's 30 s backoff plus one poll; by design) |
| A press while OBS hangs | alert at 5.02 s | nothing waits past 10 s |

## 5. Bugs, ranked by severity

None blocks a release. The file and line references are at `71922d3`.

### Should fix

**1. Plugin: a one-off `transition` over a fixed transition (Cut) changes OBS's transition duration for good** (G5, E4)
- **Steps:**
  1. Turn Studio Mode on, with the current transition Fade at 300 ms.
  2. Make Cut the current transition.
  3. Run `transition` with transition Fade and duration 800.
  4. Select Fade in OBS.
- **Expected:** Cut is current again, and Fade still has 300 ms.
- **Actual:** Fade now has 800 ms.
  - With Cut current, `GetCurrentSceneTransition` returns `transitionDuration` null and
    `transitionFixed` true, so the restore puts back only the name. OBS has one duration
    shared by all transitions.
  - `set-transition` Cut with a duration does the same: OBS accepts it and sends
    DurationChanged.
- **Where:** `plugins/obs/plugin.py:1288` (`_restore_transition` skips the duration when the
  previous transition is fixed) and `:1301-1308`.
- **Fix idea:** when the previous transition is fixed, switch to the one-off transition,
  read its duration before changing it, and restore that value afterwards.

**2. Plugin: `record` pause and pause-toggle show success when nothing can be paused** (E6)
- **Steps:**
  1. With the recording stopped, run `record` in pause or pause-toggle mode.
  2. Separately, keep OBS's default recording encoder "Use stream encoder", start a
     recording and run `record` pause.
- **Expected:** a warning, such as "OBS isn't recording" or "OBS can't pause this recording
  (it shares the stream's encoder)".
- **Actual:** plain ✓ both times. obs-websocket 5.5.6 answers PauseRecord and
  ToggleRecordPause with code 100 in both cases. No PAUSED event comes, and the recording
  keeps running (its timecode went from 12.4 s to 16.7 s).
- **Where:** `plugins/obs/plugin.py:1339-1346` and `output_action` at `:1245-1262`, which
  trust OBS's status code only.
- **Fix idea:** check the cached recording state first, or check
  `GetRecordStatus.outputPaused` after the request.

**3. Plugin: pausing a recording turns `recording` off and `record_time` to 00:00:00, and `record_paused` lags** (F-out, F-time)
- **Steps:**
  1. Give the recording an encoder of its own and reload the profile.
  2. Run `record` start, then `record` pause.
  3. Watch `recording`, `record_paused` and `record_time`.
- **Expected:** `record_paused` turns on at the PAUSED event, `recording` stays on and
  `record_time` freezes.
- **Actual:**
  - OBS's RecordStateChanged `OBS_WEBSOCKET_OUTPUT_PAUSED` carries `outputActive=false`.
  - The plugin sets `recording` off, which clears `paused` and sets `record_time` to
    00:00:00.
  - The next 2 s poll corrects all three, 1.1–1.3 s later.
  - A toggle that follows `recording` drops to its off state on every pause.
- **Where:** `plugins/obs/plugin.py:1000-1009`.
- **Fix idea:** treat PAUSED and RESUMED as `recording` = on.

**4. Plugin: `recording` flickers off, on, off while a recording stops** (F-out, F-time)
- **Steps:**
  1. Run `record` start and wait 3 s.
  2. Run `record` stop.
  3. Watch `recording` and `record_time`.
- **Expected:** `recording` goes off once.
- **Actual:** the value goes off, on, off on every stop:
  - STOPPING (`outputActive` false) sets it off.
  - During the stop, which takes about 1.25 s, the 2 s poll reads `GetRecordStatus`
    `outputActive` true and sets it on again, with `record_time` 00:00:04.
  - STOPPED sets it off again.
- **Where:** `plugins/obs/plugin.py:1110-1114`. The poll overrides the event state, and the
  stream branch at `:1093-1099` is built the same way.

**5. Built-in live values can take about 3 s to reach a tablet** (C2, I1)
- **Steps:**
  1. Put `{obs:scene}` and `{obs:recording}` titles, and a follow on `obs:recording`, on a
     tablet.
  2. Change the scene, and start and stop recording, from OBS's side.
  3. Time each title change after OBS's event.
- **Expected:** within the 2 s poll, so at most 2.5 s at p95 (C1, C2, I1).
- **Actual:**
  - `{obs:scene}` over 50 samples: p50 1.73 s, p95 2.57 s, max 2.68 s.
  - `{obs:recording}`: up to 2.90 s.
  - Measured from the request, a title never appeared in under 1.0 s, although OBS
    answers with the new scene 25–69 ms after the request.
  - Cause: `set_live` only stores the value, and keys are redrawn on a separate 1 s tick,
    which adds up to 1 s to the poll. Plugin values on a tablet take the same path.
- **Where:** `host/crates/host/src/live.rs:11` (`EVERY` = 2 s) and `:36,49`;
  `host/crates/deck/src/lib.rs:1072-1079` (`set_live`); the 1 s ticker at
  `host/crates/host/src/lib.rs:2211-2218`; plugin values at `host/crates/host/src/lib.rs:659`.
- **Fix idea:** redraw keys that show live values right after `set_live`.

**6. Plugin: renaming the live scene leaves `scene` and `scene_active:<name>` on the old name** (F-rename)
- **Steps:**
  1. Make PD Main the program scene, and watch `scene`, `scene_active:PD Main` and
     `scene_active:PD Main R`.
  2. Rename PD Main to "PD Main R" in OBS.
- **Expected:** `scene` = PD Main R, `scene_active:PD Main R` on, `scene_active:PD Main`
  off.
- **Actual:** all three were unchanged 2.5 s later. Values that take an argument, such as
  `item_visible:PD Main R/PD Color`, do refresh.
- **Where:** `plugins/obs/plugin.py:1057-1060`. SceneNameChanged only refreshes argument
  values.
- **Fix idea:** rename `s['program']` and `s['preview']` when they equal the old name.

**7. Built-in: with a mistake in `integrations.toml`, OBS keys and sliders tell the user to add OBS** (H12)
- **Steps:**
  1. Put `[obs]` with `pasword = "x"` (a typo) in `integrations.toml` and start the host.
  2. Press an OBS key and move an OBS slider.
  3. Also run `perritodeck-host action '{"type":"obs","op":"stopRecord"}'`.
- **Expected:** they point at the mistake, as Settings, diagnostics and the Companion
  section do (§1).
- **Actual:** "OBS isn't set up; add it to integrations.toml in the host's settings folder".
  The command line prints it right after "integrations.toml has a mistake, so OBS and Home
  Assistant are off: line 3: unknown field \`pasword\`…".
- **Where:** `host/crates/actions/src/lib.rs:193` (`ActionError::NotConfigured`) and
  `host/crates/host/src/main.rs:139-142`. Seen on the command-line host; the app's own key
  path wasn't tested, but it is presumably the same.

### Nice to have

**8. Both paths: a stream start that can't connect shows success on the key** (B7, E5)
- **Steps:** with the stream server set to an address where nothing listens, run the
  built-in `startStream` or `toggleStream`, or the plugin's `stream` start.
- **Expected:** some sign on the key that the stream didn't start.
- **Actual:** ✓ or no feedback. OBS goes STARTING → STOPPED about 2 s later and opens its
  own modal "Failed to connect" dialog. `streaming` and `{obs:streaming}` rightly never
  turn on.
- **Where:** `plugins/obs/plugin.py:1333-1336`; the built-in `StartStream` request at
  `host/crates/integrations/src/obs.rs:100`.
- **Fix idea:** wait a few seconds for STARTED or STOPPED before answering.

**9. Built-in: raw `{obs:*}` and `{cpu}` tokens show until the first poll** (C6)
- **Steps:** start a host whose profile has `{obs:scene}`, `{obs:recording}`,
  `{obs:streaming}` and `{cpu}` titles, and connect a tablet at once.
- **Expected:** "–" until the first value, as for a declared plugin value.
- **Actual:**
  - The first layout shows the tokens as typed.
  - `{cpu}` fills in about 0.7 s later, and the OBS tokens about 2.7 s later (OBS
    unreachable).
  - Built-in names (`obs:`, `cpu`, `mem`, `ha:`) are never "declared".
- **Where:** `host/crates/deck/src/templates.rs:56-62` (`render_title`).

**10. Built-in: a poll's connection attempt runs inside the live loop, freezing `{cpu}`, `{mem}` and `{ha:*}` for the connect timeout** (I2)
- **Steps:**
  1. Point the OBS address at a port that accepts connections and never answers.
  2. Show three `obs:` tokens and `{cpu}`, and watch for 150 s.
- **Expected:** `{cpu}` about every 2 s.
- **Actual:**
  - Mostly 2 s, but 5.0 s gaps at each backoff attempt, which comes to one per 30 s at the
    cap.
  - With OBS closed on Windows, each attempt costs one 2 s tick.
  - This is much better than before the §1 fix.
- **Where:** `host/crates/host/src/live.rs:39-50`, which awaits `obs()` inline.

**11. Built-in slider: a tablet that connects sees a stale level until the next read-back** (D2)
- **Steps:**
  1. Disconnect a slider-capable tablet that has a built-in OBS slider.
  2. Set the source to 100 % in OBS.
  3. Reconnect the tablet.
- **Expected:** the layout shows 100.
- **Actual:** the last stored level (50, or `SLIDER_START` when nothing was read yet), then
  `setValue 100` about 1 s later (up to 3 s). Levels are polled only while a
  slider-capable device shows the page. This didn't reproduce in a later run, when the
  stored level happened to be current.
- **Where:** `host/crates/deck/src/lib.rs:1226` (`poll_sliders`), `:2106-2109` and `:213`.
- **Fix idea:** poll a device's sliders when it connects.

**12. Plugin: "Scene/Source" arguments split on `|` whenever the text contains `|`** (H13)
- **Steps:**
  1. In PD BRB, make a source called "PD Odd: A|B".
  2. Watch `item_visible:PD BRB/PD Odd: A|B` and `item_visible:PD BRB|PD Odd: A|B`.
  3. Toggle the source.
- **Expected:** per the README, the first `/` splits, and `|` is only needed when the first
  name has a `/`.
- **Actual:** the `/` form stays "–" and only the `|` form follows. A scene whose own name
  contains `|` can't be addressed at all.
- **Where:** `plugins/obs/plugin.py:704-708` (`split_pair`) and `plugins/obs/README.md:103-106`.

**13. Plugin: after an input rename, the old name's `item_visible` value keeps following the renamed item** (F-rename)
- **Steps:**
  1. Watch `item_visible:PD Main/PD Color`.
  2. Rename PD Color to "PD Color R"; the value correctly shows "–".
  3. Hide PD Color R in OBS.
- **Expected:** it stays "–".
- **Actual:** it turns `off`. When the lookup fails, `refresh_args` leaves the old (scene,
  item id) mapping in `item_keys`.
- **Where:** `plugins/obs/plugin.py:960-969` and `:1038-1040`.

**14. Built-in: every connect error ends in "(is the WebSocket server enabled?)", including address mistakes** (A5, A8)
- **Steps:** put `127.0.0.1:4455`, `localhost:4455`, `127.0.0.1` or `wss://127.0.0.1:4455`
  as the URL and run an OBS action.
- **Expected:** a plain hint that the address must start with `ws://`, and that `wss://`
  isn't supported on the built-in path.
- **Actual:** "OBS: can't reach OBS at 127.0.0.1:4455: URL error: URL scheme not supported
  (is the WebSocket server enabled?)", and "…URL error: TLS support not compiled in (is the
  WebSocket server enabled?)" for `wss://`.
- **Where:** `host/crates/integrations/src/obs.rs:321-326`.

**15. Plugin: generic or misleading wording for a missing profile, scene collection or scene** (E14, E22, E23)
- **Steps:**
  - Run `profile` with "No Such Profile".
  - Run `scene-collection` with "No Such Collection".
  - Run `source-visibility` with scene "PD Nowhere" and source "PD Color".
- **Expected:** messages that name what is missing.
- **Actual:**
  - "OBS: it couldn't find that" for both switches (OBS sends 600 with no comment).
  - "OBS has no source “PD Color” in the scene “PD Nowhere”" when it is the scene that is
    missing.
- **Where:** `plugins/obs/plugin.py:77-85` (`friendly()` names only scene, input and source)
  and `:1428-1434`.

**16. Plugin: `projector` with a screen number OBS doesn't have shows success and opens nothing** (E21)
- **Steps:** on a PC with 2 screens, run `projector` with what = program and monitor = 9.
- **Expected:** a warning that OBS has only 2 screens (from `GetMonitorList`).
- **Actual:** ✓ and no window; OBS ignores the out-of-range index.
- **Where:** `plugins/obs/plugin.py:1588-1600`.

**17. Built-in: alerts read "OBS: OBS …", and the built-in's own 207 text can't be shown** (A2, A3, H1, H2, H4)
- **Steps:** press a built-in OBS key while OBS is hung for 7 s, with a wrong password, and
  while OBS is starting or exiting.
- **Expected:** one plain sentence, e.g. "OBS: didn't answer in time…" or "OBS: it isn't
  ready yet; try again in a moment".
- **Actual:**
  - "OBS: OBS didn't answer in time, so it may or may not have happened"
  - "OBS: OBS rejected the password in integrations.toml"
  - "OBS: OBS needs a password…"
  - "OBS: OBS is not ready to perform the request." (OBS's 207 comment wins over the
    built-in's own text)
- **Where:** `host/crates/integrations/src/obs.rs:130` (the comment is preferred) and `:159`
  (the 207 text is never reached). `ActionError::Integration` adds the "OBS: " prefix.

**18. Docs: the plugin README's "What's tested" is out of date and lacks OBS's real wording** (K5)
- **Steps:** compare `plugins/obs/README.md:89` and `:149-161` with this report.
- **Expected:** it lists what has now run on real OBS 31.0.3.
- **Actual:**
  - It still calls these untried, though all now work (E7, E8, E16, E20):
    - `SplitRecordFile` and `CreateRecordChapter`;
    - non-ASCII screenshot folders;
    - GDI+ `InputSettingsChanged`.
  - It doesn't say that pausing needs a recording encoder of its own.
  - It doesn't say that splits land on the next keyframe.
  - It doesn't say that format, encoder and splitting changes need a profile reload.
  - It doesn't say that `dropped_frames` counts since OBS started.
  - It doesn't list OBS's real messages (§6).
- **Where:** `plugins/obs/README.md:89`, `:149-161`.

**19. Docs: the guide's OBS section misses two behaviours seen on real OBS** (G4, K5, A5, A8)
- **Steps:** read the OBS Studio section of `website/guide/integrations.html`.
- **Expected:** it says that the built-in scene key always changes the program scene, even
  in Studio Mode (G4 asks for this). It also says that the address must start with `ws://`
  and that `wss://` isn't supported on the built-in path.
- **Actual:** neither is mentioned. The examples it does give match real OBS: "OBS: already
  recording", the replay-buffer hint, "–" while OBS is closed, and retries from 5 s up to
  30 s.
- **Where:** `website/guide/integrations.html` (OBS Studio: "Set it up" and "OBS buttons").

**20. Plugin: logs "connected to OBS ? at …" on every connection** (H1)
- **Steps:** run the plugin against OBS 31.0.3 and read the log after it connects.
- **Expected:** "connected to OBS 31.0.3 at ws://127.0.0.1:4455".
- **Actual:** "?" every time (all 17 connections in one stage). obs-websocket 5.5.6's Hello
  carries only `obsWebSocketVersion`, `rpcVersion` and `authentication`. The fake OBS in
  `plugins/obs/test_plugin.py:172` puts `obsStudioVersion` in its Hello, so the tests miss
  it.
- **Where:** `plugins/obs/plugin.py:502`.
- **Fix idea:** take the version from `GetVersion`, which `sync()` already requests.

**21. Command-line host: loads `integrations.toml` as a profile when `--profile` points at a file in the data folder** (not OBS)
- **Steps:** run `perritodeck-host --data-dir D --profile D\some-profile.toml` with
  `D\integrations.toml` present.
- **Expected:** only profile files are loaded as profiles.
- **Actual:** "ERROR skipping profile integrations: … unknown field \`obs\`", quoting its
  first line (`[obs]`). The profiles folder is taken as the parent of `--profile`, and every
  `.toml` file in it is loaded. Harmless, but noisy.
- **Where:** `host/crates/host/src/lib.rs:590-593`; `host/crates/deck/src/watch.rs:29-38`.

**22. Logs number the same key press 0-based and 1-based** (not OBS)
- **Steps:** turn on detailed logs and press key index 10.
- **Expected:** one numbering.
- **Actual:** "DEBUG perritodeck_core::server: keyDown … key=10", then "INFO perritodeck_host:
  key 11 pressed".
- **Where:** `host/crates/core/src/server.rs:601` and `host/crates/host/src/lib.rs:2295`.

## 6. OBS's real answers and behaviour

These are for the README and the guide (plan §5 step 8). None of them is a PerritoDeck bug.

**Refusals seen**

| Situation | OBS's code and comment | Shown as |
|---|---|---|
| Start recording while recording | 500, no comment | Built-in "OBS: already recording"; plugin "OBS is already recording" |
| Stop recording or streaming while stopped | 501, no comment | "OBS: not recording" / "OBS: not streaming"; plugin "OBS isn't recording" / "OBS isn't streaming" |
| Save the replay while the buffer is stopped | 501, no comment | Built-in "OBS: the replay buffer isn't running". The plugin answers from its own state: "OBS isn't running the replay buffer" |
| Replay buffer turned off in the profile | 604 "Replay buffer is not available." (also for `GetReplayBufferStatus`) | The "turned off (Settings → Output → Replay Buffer)" hint on both paths |
| Pause while stopped, or with "Use stream encoder" | 100 (success) | ✓ (bug 2) |
| Pause while paused / resume while not paused | 502 / 503 | "the recording is already paused" / "the recording isn't paused" |
| Preview or transition with Studio Mode off | 506, no comment | "OBS: Studio Mode is off" / "Studio Mode is off; there's nothing to transition" |
| Missing scene, input or source | 600 "No source was found by the name of \`X\`." | That comment, after "OBS: " |
| Missing profile or scene collection | 600, no comment | "OBS: it couldn't find that" (bug 15) |
| Missing transition / filter / hotkey | "No scene transition was found by that name." / "No filter was found in the source \`X\` with the name \`Y\`." / "No hotkeys were found by that name." | The comment |
| `browser-refresh` on a non-browser source | "Unable to find a property by that name." | The comment |
| Split with splitting off, or while paused | 702 "Verify that file splitting is enabled in the output settings." | The comment (misleading while paused) |
| Chapter on MKV, or while paused | 702 "Verify that the output being used supports chapter markers." | The comment (misleading while paused) |
| Unknown request type / missing field | 204 "Your request type is not valid." / "Your request must contain at least one of the following fields: …" | The comment |
| `inputVolumeDb` above 26 | "The field value of \`inputVolumeDb\` is above the maximum of \`26.000000\`" | The comment |
| Caption while not streaming | 501, no comment | "OBS isn't streaming" |
| Any request while OBS starts or exits | 207 "OBS is not ready to perform the request." | The comment (bug 17) |
| Wrong password | Close code 4009. OBS's own log shows only a connect and a 1006 disconnect, never an authentication failure | See A2 |
| OBS exiting | Close code 1001 "Server stopping." | Plugin "OBS was closed"; built-in titles "–" |

**Behaviour seen**
- **Pause and encoders:** pausing needs a recording encoder of its own. With "Use stream
  encoder" (OBS's default), PauseRecord succeeds and nothing pauses.
- **Profile reloads:** changes to the recording format, encoder and splitting take effect
  only after a profile reload. OBS switches profiles mid-recording without complaint, and
  the recording carries on.
- **Splitting:**
  - A split lands on the next keyframe (8.3 s at the default keyframe interval and 30 fps).
  - A split requested less than one keyframe interval before Stop carries into the next
    recording, whose first file is then nearly empty.
- **Toggles during a change:** OBS answers success to a toggle that arrives while an
  output is starting or stopping, then ignores it. A replay-buffer toggle sent the instant
  the buffer stops is lost the same way.
- **Short recordings:** a recording of about 1.2 s is a valid but empty MP4.
- **Failed stream start:** to an unreachable server, StartStream succeeds, the state goes
  STARTING → STOPPED in about 2 s, and OBS opens a modal "Failed to connect" dialog.
- **Transitions:**
  - OBS 31 sends SceneTransitionEnded for Cut too.
  - All transitions share one duration.
  - `GetCurrentSceneTransition` can lag the events by about 50 ms after a duration change.
- **Studio Mode:** the built-in scene key changes the program, not the preview.
- **Sources:**
  - GDI+ text sends the full text in InputSettingsChanged, so live updates work.
  - A media action on a colour source succeeds and does nothing.
  - `set-text` on a colour source adds a stray `text` setting.
- **Projectors:** OBS ignores an out-of-range monitor index for a projector.
- **Statistics:** `GetStats` frame counts run from when OBS started, so `dropped_frames`
  never returns to 0.
- **Version and scene fields:** the Hello carries no OBS version. `GetCurrentProgramScene`
  still returns `currentProgramSceneName` alongside `sceneName`.
- **Connections:**
  - A refused connection to 127.0.0.1 takes about 2 s to report on Windows.
  - On a polite exit, OBS answers every in-flight request before it closes.
- **Fixtures:** obs-websocket can't create a group, and a new scene collection has no
  Desktop Audio or Mic.

## 7. What's left

- **With the OBS plugin installed in the app:**
  - the host half of every E case;
  - F-follow;
  - D3 (tablet) and D6 (plugin slider);
  - D4, K1, K3 (plugin values) and K6;
  - plugin values through a tablet (I1).
- **With a live stream target** (a Twitch bandwidth-test key or a local RTMP server): B6,
  C4, E5, E24, F-stats during a stream, and I6.
- **With a person at OBS:**
  - A4, A6 (server off), H6, H7, H8 and H11;
  - full-screen projectors (E21) and the virtual camera in a camera app (E10);
  - D8 and L2.
- **With a second computer:** A9 and J4.
- **With a scratch data folder for the app:**
  - the app's halves of A2, A3, A5 and H12 (Settings text, Companion section);
  - H10 with a host restart;
  - C8 "not set up".
- **Not yet run anywhere:** H3 (kill), I4 over 30 minutes, and groups A, B, C and F on Linux
  and macOS.
- **Paperwork:** list bugs 1–22 in `docs/next-steps.md`, then update the README and the
  guide from §6 (bugs 18 and 19).
