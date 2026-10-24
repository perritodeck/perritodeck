# OBS Studio: real-world test plan

PerritoDeck can control OBS in two independent ways, and until this pass neither had run
against a real OBS. Both were tested only against fake obs-websocket servers. This plan is
for the maintainer and contributors: what to set up, every case to run, and how a
harness can run most of them unattended. The first pass ran on 2026-10-13 against OBS
31.0.3 with obs-websocket 5.5.6 ([results](obs-results-2026-10-13.md)); what it found and
what has been fixed since is at the end of [§1](#found-by-the-first-pass-2026-10-13).

| | Built-in integration (host) | OBS plugin (`dev.perritodeck.obs`) |
|---|---|---|
| Set up in | `integrations.toml` (`[obs] url`, `password`), read once at start | Plugins → OBS Studio (url, masked password, "insecure" for `wss://`), applied at once |
| Actions | 10: `scene`, `toggleRecord`, `startRecord`, `stopRecord`, `toggleStream`, `startStream`, `stopStream`, `toggleMute`, `toggleReplayBuffer`, `saveReplayBuffer` | 26, two of them sliders (see the [coverage map](#4-coverage-map)) |
| Live values | 3, polled every 2 s: `{obs:scene}`, `{obs:recording}`, `{obs:streaming}` | 30 (21 plain, 9 with an argument), kept current by OBS events; statistics polled every 2 s |
| Slider | `slider = { type = "obs", input = "…" }`, read back every 3 s | `volume` and `request-value` |
| Connection | One shared connection, opened on first use; reconnects when it has dropped. After a failed connection, polls wait 5 s, doubling up to 30 s; a key press tries at once. Starting a stream opens a connection of its own, to hear whether it went live | Its own WebSocket client; reconnects in the background with backoff (0.5 s up to 10 s) |
| Scene key in Studio Mode | Always changes the program scene | `auto` changes the preview, `program` the program |
| Code | `host/crates/integrations/src/obs.rs`, `host/crates/host/src/live.rs`, `host/crates/actions/src/slider.rs` | `plugins/obs/plugin.py`, `perritodeck-plugin.json`, `README.md` (plugin 1.0.3) |

**How to read the cases.** Each case has an ID, steps, the expected result and how it runs:
**Auto** means the harness does it and checks OBS's own state; **Semi** means the harness
plus a person (a click in OBS, a look at the screen, a stream target); **Manual** means a
person only. Record the actual result, the real OBS wording of every error, and timings.

## 1. Fixed before this pass: re-check these

The research for this plan found these bugs, and they were fixed before the pass, tested
only against fakes. Each one has a case that shows whether the fix works on a real OBS. If
a case fails, the bug goes back on the list in `docs/next-steps.md`.

Built-in integration:

| Bug | Re-check in |
|---|---|
| Built-in path ignored OBS's status code, so "start recording" while recording, "stop" while stopped and "save replay" with the buffer off all flashed "OBS: request failed". It now says e.g. "OBS: already recording", or where to turn the replay buffer on | B5, B10 |
| Built-in path had no backoff and held its lock while connecting: with OBS closed or the password wrong, each 2 s live cycle tried once per `obs:` token, stalling `{cpu}`, `{mem}`, `{ha:…}` and OBS key presses | H5, I2, I3 |
| Built-in path resent any request after a failure, including a 5 s timeout, so a slow OBS could apply a toggle twice. Only `Get…` and `Set…` requests are repeated now | H4, I5 |
| Built-in wrong-password hint depended on the close reason containing "closed"; real OBS closes with 4009 "Authentication failed." | A2 |
| `{obs:scene}` read only the deprecated `currentProgramSceneName` field; it falls back to `sceneName` | C1 |
| A Follow typed with braces (`{plugin:…:recording}`, as the plugin README wrote it) never turned on | C7, F-follow |
| A typo in `integrations.toml` turned off OBS, Home Assistant and Companion with only a log line. The file is still ignored until it's fixed, but Settings and diagnostics now say it has a mistake, and the log names the line without quoting the file | H12 |
| `perritodeck-host action` built no integrations, so OBS actions always failed from the command line. It reads `integrations.toml` now; plugin actions still need the running host | B (optional) |
| The "may or may not have happened" alert read badly for a timeout and for OBS's own shutdown close ("OBS didn't answer in time before answering"); each case now has its own sentence | H4, H2 |
| The one-line `integrations.toml` mistake could still quote a value: toml's "invalid type: integer `12345678`" shows an unquoted numeric password. Only the kind of mistake and what was expected are kept now | H12 |
| A press could report "can't reach OBS" from a poll's connection attempt that began before the press (and before OBS was listening); a press now reuses only a failure from an attempt that began after it | H1 |
| Saving a profile that has a braced follow reloaded it a second time (every tablet redrew), because the tidied profile never matched the file | C7 |
| With a mistake in `integrations.toml`, a `companion = true` profile still showed "add this to integrations.toml"; the Companion section now shows the mistake | H12 |

OBS plugin (1.0.2) and the editor:

| Bug | Re-check in |
|---|---|
| A plugin slider couldn't follow a value that takes an argument (`volume:<source>`) from the editor: its Follows list offered bare value ids only, so it worked only when the profile was edited by hand. The editor now shows a box for the argument ("Audio source to follow") and saves `volume:<source>` | D4 |
| A plugin slider's follow, and now its argument, stayed when "Slider sets" switched to another plugin, and was saved unseen on that plugin's slider; switching now clears both | K6 |
| The plugin README's `request-value` example sent `inputVolumeDb` 0–100, outside OBS's −100…26 dB range. It now sends `inputVolumeMul: {mul}` and warns about the dB range | D5 |
| The plugin README said the built-in integration has "no live values"; it now lists its actions, slider and `{obs:*}` values | K5 |
| The plugin's "turned off in OBS's settings" hint keyed only on status 504, while for the replay buffer OBS sends 604 "Replay buffer is not available." Both now say "the replay buffer is turned off in OBS (Settings → Output → Replay Buffer)". The 604 reading comes from obs-websocket's source and hasn't met a real OBS yet | E9 |
| `dropped_frames` counts render and encoder skips, not network drops, while the website said "dropped frames". It's now named "Skipped frames, rendering and encoding (%)", and a new value, `network_dropped_frames` ("Dropped frames, network (%)"), comes from `GetStreamStatus` | F-stats |

Known and **not** being fixed now; record what happens:
- The built-in path has no TLS, so `wss://` addresses fail (A8). Since the first pass it
  refuses them before connecting and says why.
- Besides its hint for a replay buffer that's turned off, the plugin words a refusal that
  comes without a comment from OBS only for codes 204, 500–506 (outputs and Studio Mode),
  600 (naming the scene, source, profile or scene collection when the request names one)
  and 604. Any other code without a comment reads "OBS refused X (error N)". Record any you see. (Corrected after the first
  pass: this said 600 and 604 read "OBS refused X (error N)", but a 600 read "OBS: it
  couldn't find that".)
- fakedeck has no slider command (see [the harness](#5-automated-harness)).

### Found by the first pass (2026-10-13)

The first pass ran on 2026-10-13 against OBS 31.0.3 with obs-websocket 5.5.6 on Windows 11,
with PerritoDeck at `71922d3` and plugin 1.0.2: see [the results](obs-results-2026-10-13.md).
Every re-check above that a real OBS could exercise held. It found 22 bugs, numbered as in
[its §5](obs-results-2026-10-13.md#5-bugs-ranked-by-severity), and all of them have been
fixed since. A re-check on 2026-10-15 against the same OBS ran the cases below (all but B6,
which needs a live target, and K5, which is these documents), checks for bugs 21 and 22,
and B1, B4, B8, D1, E1, E11, E12 and F-args again for regressions. Every one passed. The
built-in cases ran on a command-line host built from the fixed code, which has the same
client and deck code as the desktop app, and the plugin's at Layer A with plugin 1.0.3.
F-follow, D4 and K6 are still blocked: they need the plugin installed in the desktop app.

Built-in integration and the host:

| Bug | Re-check in |
|---|---|
| 5. Live values took up to about 3 s to reach a tablet: keys redrew on a 1 s tick after the 2 s poll. Keys that show or follow a live value now redraw as soon as it changes (re-check: `{obs:scene}` p95 1.46 s over 50 samples, `{obs:recording}` at most 1.85 s) | C1, C2, C3, I1 |
| 7. With a mistake in `integrations.toml`, OBS and Home Assistant keys and sliders said "OBS isn't set up; add it to integrations.toml…". They now say "integrations.toml has a mistake, so OBS is off: line 3: unknown field `pasword`…", on the tablet and from `perritodeck-host action` | H12 |
| 8. A stream start that can't connect showed success. `startStream`, and a `toggleStream` that starts one, now wait up to 6 s on a connection of their own. Back to stopped: "OBS: the stream didn't start; see the message OBS shows, or check Settings → Stream in OBS". No outcome yet: "OBS: the stream hasn't started yet; if it can't connect, OBS shows a message saying why" | B7; B6 with a live target |
| 9. The raw `{obs:…}`, `{cpu}`, `{mem}` and `{ha:…}` tokens showed until the first poll; they show "–" now | C6 |
| 10. A poll's connection attempt ran inside the live loop and froze `{cpu}`, `{mem}` and `{ha:…}` for up to 5 s. OBS and Home Assistant are now asked beside the loop | I2 |
| 11. A slider tablet that connected showed the last stored level until the next read-back. The levels are read as soon as it shows sliders | D2 |
| 14. Every connect error ended in "(is the WebSocket server enabled?)". An address without `ws://` is now refused before connecting with "OBS: the address 127.0.0.1:4455 in integrations.toml must start with ws://, like ws://127.0.0.1:4455", and `wss://` with "OBS's WebSocket server uses ws://, so the address in integrations.toml must start with ws://, not wss://". The hint stays only when nothing answers (refused or timed out) | A5, A6, A8 |
| 17. Alerts read "OBS: OBS …", and OBS's 207 comment hid the built-in's own text. A reason that starts with "OBS" now stands alone ("OBS didn't answer in time, so it may or may not have happened", "OBS rejected the password in integrations.toml", "OBS needs a password; add it to integrations.toml"), and a 207 reads "OBS: it isn't ready yet; try again in a moment" | A2, A3, H2, H4 |
| 21. With `--profile` pointing at a file in the data folder, the command-line host loaded `integrations.toml` as a profile too, and logged an error. It's left alone now | No case: the steps in the results |
| 22. Detailed logs numbered a key press from 0 ("keyDown … key=10") and from 1 ("key 11 pressed"); both count from 1 now | No case: the steps in the results |

OBS plugin 1.0.3:

| Bug | Re-check in |
|---|---|
| 1. A one-off `transition` over a fixed-length transition (Cut) left OBS's shared duration at the one-off's for good, and `set-transition` Cut with a duration did the same. The one-off now notes the shared duration once OBS has switched and puts it back; a duration is left out for a fixed-length transition | G5, E4 |
| 2. `record` pause and pause-toggle showed ✓ while nothing recorded, or when OBS couldn't pause. Nothing recording: ⚠ "OBS isn't recording", with nothing sent. No PAUSED within 2 s: "OBS couldn't pause the recording: pausing needs a recording encoder of its own, not the stream's (Settings → Output)" | E6 |
| 3. Pausing turned `recording` off and `record_time` to 00:00:00 (PAUSED carries `outputActive` false). PAUSED and RESUMED, and RECONNECTING and RECONNECTED for the stream, now count as still on | F-out, F-time |
| 4. `recording` went off, on, off while a recording stopped: the 2 s poll undid STOPPING. A poll no longer overrides a start or stop under way (for up to 30 s), or an event that came during the poll | F-out, F-time |
| 6. Renaming the live scene left `scene` and `scene_active:*` on the old name | F-rename |
| 8. A stream start that can't connect showed ✓. `stream` start, or a toggle that starts one, now waits up to 7 s: the error "OBS couldn't start the stream: see the message in OBS, or check Settings → Stream", or ⚠ "OBS is still connecting to the stream server…" | E5 |
| 12. A "Scene/Source" argument split at a `\|` whenever there was one, against the README's rule. Every split is tried (the first `\|`, the first `/`, then the others) and the first pair OBS has wins | H13 |
| 13. After an input rename, the old name's `item_visible` value kept following the renamed item | F-rename |
| 15. A missing profile or scene collection read "OBS: it couldn't find that"; now "OBS: there's no profile called “X”" and "OBS: there's no scene collection called “X”". `source-visibility` tells a missing scene ("OBS has no scene “X”") from a missing source | E14, E22, E23 |
| 16. `projector` on a screen number OBS doesn't have showed ✓ and opened nothing; now "OBS has no screen 9; it has 2" | E21 |
| 18. The README's "What's tested" was out of date and lacked OBS's real answers | K5 |
| 20. The connect log read "connected to OBS ? at …"; now "connected to OBS 31.0.3 (obs-websocket 5.5.6) at …", once per connection | H1, F-conn |
| Statistics read "–" for about 2 s after connecting; they now come with the first sync | F-stats |
| `record-split` and `record-chapter` while paused showed OBS's misleading 702 comment ("Verify that …"); now ⚠ "the recording is paused; resume it first" | E7, E8 |

The guide (bug 19) now says that the built-in scene key changes the program scene in Studio
Mode and that the address starts with `ws://` (G4, K5). It also says that a stream start
which can't connect flashes ⚠ (bug 8).

What real OBS did also showed five expected results in §3 to be wrong. Each now says
"corrected after the first pass":
- **E4:** Cut has a fixed length, and OBS keeps one duration for all transitions, so Cut
  with a duration must leave Fade's alone.
- **E5:** OBS refuses to start an output that's running with 500 and no comment (seen for
  recording; streaming needs a live target), so the key shows the plugin's own "OBS is
  already streaming", not "it's already running".
- **E6:** pausing needs a recording encoder of its own. With "Use stream encoder", OBS's
  default, OBS says yes to a pause and sends no PAUSED. §2.2 now gives PD Test a recording
  encoder, and E6 also pauses once with the stream's encoder.
- **F-stats:** `GetStats` counts skipped frames from when OBS started, so `dropped_frames`
  needn't be 0.
- **G5:** OBS 31 sends SceneTransitionEnded for Cut too, and with Cut current
  `GetCurrentSceneTransition` reports no duration, so checking it alone missed bug 1. G5
  now also checks Fade's duration afterwards.

## 2. Prerequisites

### 2.1 OBS

1. **OBS 30.2 or newer** (31.x is current). `SplitRecordFile` and `CreateRecordChapter` need
   30.2+. If an older 30.x is at hand, run E7 and E8 there too to record the "update OBS"
   path.
2. **Turn on the WebSocket server.** Tools → WebSocket Server Settings → Enable WebSocket
   server. Keep port 4455 and "Enable Authentication" on. "Show Connect Info" shows the
   password.
3. **Put the password in an environment variable** for the harness, e.g. a user variable
   `PD_OBS_PASSWORD`. The harness reads it, writes it into the test `integrations.toml` and
   the plugin's test settings, and never prints it. Don't paste it into chat, logs or
   issues.
4. **Check OBS's log after the first start** (Help → Log Files → View Current Log). An old
   install can still carry `obs-websocket-compat` (the 4.x plugin); note whether it loads.
5. After a forced kill OBS offers Safe Mode on the next start, which runs without
   third-party plugins and possibly without the WebSocket server (check the OBS 30 notes).
   The harness starts OBS with `--disable-shutdown-check` to skip it.

### 2.2 A dedicated test profile and scene collection

Never test on your real OBS profiles or scene collection, and never stream from your
everyday profile: the PD Test profile holds only a bandwidth-test key, a private event's
key or a local server (§2.3). Everything runs in a profile and a scene collection both
called **PD Test**. The harness can build them through obs-websocket requests only
(`CreateProfile`, `SetProfileParameter`, `CreateSceneCollection`, `CreateScene`,
`CreateInput`, `CreateSourceFilter`), or you can make them by hand:

- **Profile `PD Test`:** Advanced output; recordings to a scratch folder; replay buffer on
  (20 s); recording format **Hybrid MP4** for chapters (plus one run in MKV for the error
  path); Automatic File Splitting set to split manually only, for `SplitRecordFile` (check
  the exact option in your OBS version); a recording encoder of its own, not "Use stream
  encoder", because pausing needs one (E6). Recording settings made with
  `SetProfileParameter` take effect only once the profile is loaded again (switch to
  another profile and back). A second profile `PD Test2` for the profile switch.
- **Scene collection `PD Test`:**
  - Scenes `PD A`, `PD B`, `PD C`, a non-ASCII one `Escena ñ 🎮`, and one with a slash and a
    comma in the name (`Talk/Show, live`).
  - `PD Color`: a colour source, with a Color Correction filter `PD CC`.
  - `PD Text` (Text GDI+ on Windows) and `PD Text FT` (Text FreeType 2).
  - `PD Media`: a media source playing a 5 s MP4 (the harness can record one itself), and a
    slideshow with two images for next/previous.
  - `PD Browser`: a browser source pointing at a local page the harness serves, so it can
    count reloads.
  - `PD Mic` (an audio input) and Desktop Audio.
  - A group containing a source, and a nested scene.
  - Transitions Fade (300 ms) and Cut.
- A second, larger scene collection `PD Big` (a few dozen scenes) for the slow
  collection-switch case.
- The virtual camera is built into OBS on Windows and macOS; on Linux it needs
  `v4l2loopback`.

Afterwards, switch back to your own profile and collection. obs-websocket can remove a
profile (`RemoveProfile`) but not a scene collection; remove that in OBS if you want.

### 2.3 A safe streaming target

Streaming cases (B6, B7, C2, C4, E5, E24, F-stats, I6) need a target that nobody sees. Set
it in the **PD Test profile only**:

- **Twitch bandwidth test:** a stream key with `?bandwidthtest=true` appended. Use the
  test broadcaster T's key from the [Twitch plan](twitch.md), not the main channel's, so
  a lost suffix can't put a test on the main channel. Twitch accepts the stream but never
  shows it. Its "live" status never flips, so the Twitch plugin's values stay offline; a
  real broadcast belongs to the Twitch plan.
- **YouTube:** a private or unlisted scheduled event and its stream key.
- **A local RTMP server:** needs a download, so only with the maintainer's OK.

The maintainer enters stream keys. The harness refuses to start a stream unless the
current profile is `PD Test` **and** either the key ends with `?bandwidthtest=true` or the
server matches the private or local target the maintainer approved. It reads the key with
`GetStreamServiceSettings` for that check only, and never prints or logs it.

### 2.4 PerritoDeck

- **Its own data folder.** The harness sets `PERRITODECK_DATA_DIR` to a scratch folder, so
  real pairings, profiles and settings stay untouched. It writes there:
  - `integrations.toml` with `[obs] url = "ws://127.0.0.1:4455"` and the password from
    `PD_OBS_PASSWORD`;
  - the OBS plugin, copied from `plugins/obs/` into `plugins/dev.perritodeck.obs/`, and its
    settings in `plugin-settings/dev.perritodeck.obs.json`;
  - a test profile `profiles/obs-test.toml` with a key for every built-in op, titles with
    every `{obs:*}` token, toggles that follow `obs:recording` and `obs:streaming` with
    distinct state titles, a built-in OBS slider, and keys, tiles and toggles for the plugin.
- **Port.** Only one host can listen on port 46710, and the desktop app allows one copy
  of itself. The command-line host takes `--port`, so the automated layers run it on a
  spare port and never need 46710. Cases that need the desktop app itself (the editor,
  `test_action`) need the tray app quit first.
- **A real tablet** on USB for the manual slider and toggle checks (D8 and the follow
  toggles by eye).

### 2.5 Baseline before each case

Every automated case starts from the same OBS state, set by the harness:
- streaming, recording, replay buffer and virtual camera stopped;
- Studio Mode off; program scene `PD A`;
- `PD Mic` unmuted at 100 %; `PD CC` on; every scene item visible;
- current transition Fade, 300 ms.

Each case then checks OBS's own state through the harness's independent client, not just
what PerritoDeck says.

## 3. Test cases

### A. Connection and password (both paths)

| ID | Steps | Expected | How |
|---|---|---|---|
| A1 | Right password, OBS running. Press a built-in key; watch the plugin's `connected` value | Built-in key ✓; `connected` = `on` within 1 s; OBS's session list shows two clients | Auto |
| A2 | Wrong password in `integrations.toml` (restart the host) and in the plugin's settings | Built-in key ⚠ "OBS rejected the password in Settings → Integrations" (§1; before 2026-10-24 "… in integrations.toml"); Settings → Integrations' Test connection: "OBS rejected the password. Copy it again from OBS: …"; plugin log and key: "OBS rejected the password: check it in this plugin's settings…"; both retry with backoff, not in a tight loop | Auto |
| A3 | No password while OBS requires one | Built-in: "OBS needs a password; add it in Settings → Integrations"; plugin: "OBS needs a password: enter it in this plugin's settings…" | Auto |
| A4 | Untick authentication in OBS, password still set | Both connect (OBS sends no challenge) | Semi (untick in OBS) |
| A5 | Plugin address without scheme or port (`127.0.0.1`), `localhost`, `[::1]`; the same in `integrations.toml` | Plugin normalises the address; record what the built-in path says without `ws://` | Auto |
| A6 | Wrong port; then the server turned off | Built-in: "can't reach OBS at … (is the WebSocket server enabled?)"; plugin: "couldn't reach OBS at …: connection refused. Is OBS open…" | Auto |
| A7 | Change the plugin's settings while connected | Reconnects at once with the new settings | Auto |
| A8 | `wss://` address | Built-in fails (no TLS): record the message. Plugin works only through a TLS proxy | Manual (optional) |
| A9 | OBS on another computer: firewall closed, then open | Timeout message, then works. The screenshot action with no folder says "OBS runs on another computer: enter a folder on that computer" | Manual |

### B. Built-in actions

Run each one twice: through the desktop app's `test_action` and through a real key press
from fakedeck (B11). The harness checks OBS's state and events.

| ID | Action | Expected in OBS | How |
|---|---|---|---|
| B1 | `scene` = `PD B` | Program scene is `PD B` (CurrentProgramSceneChanged) | Auto |
| B2 | `scene` = `Missing` | Key ⚠ with OBS's comment, e.g. "OBS: No source was found by the name of `Missing`." (record the real wording) | Auto |
| B3 | `scene` = `Escena ñ 🎮`, and the slash-and-comma name | Switches | Auto |
| B4 | `toggleRecord` twice | Recording starts, then stops; the file is in the scratch folder | Auto |
| B5 | `startRecord` while recording; `stopRecord` while stopped | "OBS: already recording" / "OBS: not recording" instead of "OBS: request failed" (§1); record whether OBS sends its own comment instead | Auto |
| B6 | `toggleStream`, `startStream`, `stopStream` (safe target only) | StreamStateChanged active, then inactive | Semi (needs the target) |
| B7 | `startStream` with the server set to an unreachable address | The request succeeds, OBS shows its own "Failed to connect" dialog and the state goes back to stopped. Record what the key and `{obs:streaming}` show | Semi |
| B8 | `toggleMute` on `PD Mic`; on a missing input | Mute flips both ways; missing input → OBS's comment | Auto |
| B9 | `toggleReplayBuffer`, then `saveReplayBuffer` | Buffer runs; a replay file is saved (ReplayBufferSaved) | Auto |
| B10 | `saveReplayBuffer` while the buffer is stopped; `toggleReplayBuffer` with the buffer turned off in the profile | Plain warnings; turned off → "the replay buffer is turned off (Settings → Output → Replay Buffer)" (§1). Record OBS's codes and comments | Auto |
| B11 | Every op above from a fakedeck key press | Same results; the deck gets ⚠ on failure | Auto |

### C. Built-in live values and follow

| ID | Steps | Expected | How |
|---|---|---|---|
| C1 | A key titled `{obs:scene}`; change the scene from the harness | Title updates within 2.5 s (fakedeck `setTitle`); record which field OBS answers with (§1) | Auto |
| C2 | Keys titled `{obs:recording}` and `{obs:streaming}` | `on`/`off` within 2.5 s | Auto (streaming: Semi) |
| C3 | Toggle with `follow = "obs:recording"` and states REC-OFF/REC-ON; start recording **in OBS** (from the harness, not the key) | Key shows REC-ON without being pressed | Auto |
| C4 | The same with `obs:streaming` | Flips | Semi |
| C5 | Close OBS | Titles show `–`; follow keys show their first state; diagnostics say OBS is "set up, but not reachable" | Auto |
| C6 | Start the host and watch the first 2 s | Record whether `{obs:scene}` briefly shows the raw token before the first poll | Auto |
| C7 | Follow typed with braces, `{obs:recording}`; then save that profile from the app (change another key's title) | Follows like `obs:recording` (§1); the save reloads the profile once, so fakedeck sees one new layout, not two | Auto |
| C8 | Diagnostics' integrations line in each state: not set up, not checked yet, answering, not reachable | Matches what OBS is doing | Auto (`get_diagnostics`) |

### D. Sliders

| ID | Steps | Expected | How |
|---|---|---|---|
| D1 | Built-in OBS slider on `PD Mic` to 0, 25, 50, 100 | `GetInputVolume` `inputVolumeMul` = 0, 0.0625, 0.25, 1.0 (about −∞, −24.1, −12.0, 0 dB): the curve is (value/100)² | Auto (slider command) / Semi (tablet) |
| D2 | Move the `PD Mic` fader from the harness | The tablet's slider follows within 3.5 s | Auto once fakedeck has `--sliders` (§5 item 7); otherwise Semi with a tablet |
| D3 | Plugin `volume` slider on `PD Mic` that follows `volume:PD Mic` (written in the profile file) | Same curve as D1; the `volume:PD Mic` value follows fader changes in under 0.5 s, and so does the tablet's slider | Auto (Layer A); the tablet's slider as D2 |
| D4 | In the editor, make that slider follow "Source volume (0–100)" and type `PD Mic` in the "Audio source to follow" box | The choice is offered and saved as `volume:PD Mic`, and the slider follows (§1) | Auto (CDP) |
| D5 | Plugin `request-value` with `SetInputVolume` and `inputVolumeMul: {mul}`, as the README's example now has it | Same curve as D1; the README warns that `{value}` (0–100) doesn't fit `inputVolumeDb` (§1) | Auto |
| D6 | 100 slider changes in 2 s on each slider kind | Ends at the last value; nothing queued after release | Auto once fakedeck has `--sliders` (§5 item 7); otherwise Semi with a tablet |
| D7 | Drag while OBS is disconnected | ⚠ on the slider; no burst of queued changes after reconnecting | Auto once fakedeck has `--sliders` (§5 item 7); otherwise Semi with a tablet |
| D8 | Feel and lag on a real tablet | Smooth; no jump back from the 3 s read-back during a drag | Manual |

### E. Plugin actions

Run each action first straight over the plugin protocol (no host), then through the host
(`test_action` with `{ type = "plugin", plugin = "dev.perritodeck.obs", action = …,
settings = … }`, or a fakedeck key). ⚠ means `ok: true` with a message.

| ID | Action and variants | Check | How |
|---|---|---|---|
| E1 | `scene`: `auto`, `program`, `preview`, with Studio Mode off and on; a missing scene | Program or preview changes as the mode says; missing → OBS's comment | Auto |
| E2 | `studio-mode`: toggle, on, off | StudioModeStateChanged | Auto |
| E3 | `transition` (see G5, G6) | | Auto |
| E4 | `set-transition`: `Fade` 300, `Cut`, `Cut` with duration 500, duration 49 and 20001, an unknown name | Current transition and duration change. Cut has a fixed length and OBS has one duration for all transitions, so Cut with 500 leaves Fade at 300 (corrected after the first pass); out-of-range durations fail in the plugin; unknown name → OBS's comment | Auto |
| E5 | `stream`: toggle, start, stop; start while streaming | StreamStateChanged; start while streaming → ⚠ "OBS is already streaming" (corrected after the first pass) | Semi (target) |
| E6 | `record`: toggle, start, stop, pause-toggle, pause, resume; pause while stopped; pause once more with the recording on "Use stream encoder" (`RecEncoder` = `none`, profile loaded again) | RecordStateChanged including PAUSED and RESUMED; ⚠ texts. With the stream's encoder OBS says yes and sends no PAUSED, and the key says it couldn't pause (corrected after the first pass) | Auto |
| E7 | `record-split` while recording (splitting set up, and not); while stopped | A new file / OBS's comment / ⚠ "OBS isn't recording" | Auto |
| E8 | `record-chapter` with a name and blank; Hybrid MP4 and MKV | Chapter in the MP4 (check with `ffprobe` if present, otherwise by eye in a player); MKV → OBS's comment | Semi |
| E9 | `replay-buffer`: toggle, start, stop, save; buffer turned off in the profile | Events and a saved file; turned off → "the replay buffer is turned off in OBS (Settings → Output → Replay Buffer)" for every mode. Record the real code and comment (§1) | Auto |
| E10 | `virtualcam`: toggle, start, stop | VirtualcamStateChanged; optionally open it in a camera app | Auto / Manual |
| E11 | `mute`: toggle, mute, unmute; a source with `/` in its name | InputMuteStateChanged | Auto |
| E12 | `volume` (slider) at 0, 50, 100 | `inputVolumeMul` as in D1 | Auto |
| E13 | `volume-step` +10 and −10, at the 0 and 100 bounds, on a source no key watches, step 150 | Clamped to 0–100; step 150 fails in the plugin | Auto |
| E14 | `source-visibility`: toggle, show, hide; blank scene (the live scene); a source inside a group; a missing source | SceneItemEnableStateChanged; missing → "OBS has no source … in the scene …"; record the group behaviour | Auto |
| E15 | `filter`: toggle, on, off; a filter on a scene; a missing filter | SourceFilterEnableStateChanged; missing → OBS's comment | Auto |
| E16 | `screenshot`: blank source (live scene), a named source, png and jpg, blank folder (Pictures), a non-ASCII folder `Capturas ñ 🎮`, twice in the same second | The file exists with a PNG or JPEG header; the second gets ` (2)`; the non-ASCII path works | Auto |
| E17 | `hotkey`: `OBSBasic.StartRecording`, `OBSBasic.Screenshot`, an unknown name | Recording starts / a screenshot file / OBS's comment (or silence: record it) | Auto |
| E18 | `media`: play-pause (playing → pause, ended → restart, stopped → play), play, pause, restart, stop; next and previous on the slideshow | `GetMediaInputStatus` state | Auto |
| E19 | `browser-refresh` on `PD Browser` | The local server sees a new request | Auto |
| E20 | `set-text` on GDI+ and FreeType 2, with `\n`, emoji and 2 kB of text | `GetInputSettings` text matches; the newline renders | Auto |
| E21 | `projector`: program, preview, multiview, source; monitor 1, 2 and blank | A projector window appears (window title); on the second screen when asked | Semi |
| E22 | `profile` to `PD Test2` and back; again while recording | CurrentProfileChanged; record what OBS does with outputs running | Auto |
| E23 | `scene-collection` to `PD Big` and back | ⚠ "OBS is still switching to …" if it takes over 8 s; values resync after CurrentSceneCollectionChanged | Auto |
| E24 | `caption` while not streaming, and while streaming | Not streaming → ⚠ "OBS isn't streaming"; streaming → ✓ (only viewers of a real stream see captions) | Semi |
| E25 | `request`: `GetVersion`; invalid JSON in data; data that isn't an object; an unknown request type; an answer over 500 characters | `last_response` holds the JSON, cut with "…"; plain validation errors | Auto |
| E26 | `request-value` (see D5) | | Auto |

### F. Plugin values

Put each value on a key title as `{plugin:dev.perritodeck.obs:<value>}`, change it from
OBS's side (a harness request or the OBS window), and expect the key within 0.5 s for
event-driven values and 2.5 s for polled ones.

| ID | Values | Expected | How |
|---|---|---|---|
| F-conn | `connected` | `on`; `off` when OBS closes | Auto |
| F-scene | `scene`, `preview_scene`, `studio_mode` | `preview_scene` is `–` unless Studio Mode is on | Auto |
| F-out | `streaming`, `recording`, `record_paused`, `replay_buffer`, `virtualcam` | `on`/`off` as the outputs change | Auto (streaming: Semi) |
| F-names | `transition`, `profile`, `scene_collection` | The current names | Auto |
| F-time | `stream_time`, `record_time` | `HH:MM:SS`, `00:00:00` when stopped, frozen while paused | Auto (stream: Semi) |
| F-stats | `fps`, `cpu`, `memory`, `dropped_frames`, `network_dropped_frames`, `bitrate` | `fps` equals the canvas rate; `cpu` and `memory` within a few % or MB of `GetStats`; `dropped_frames` low, rising when the encoder is overloaded (slowest preset), and not necessarily 0, because OBS counts from when it started (corrected after the first pass); `network_dropped_frames` 0 while not streaming, rising when the network is throttled during a stream (throttling needs a tool, so the maintainer's OK), and matching `GetStreamStatus`; `bitrate` near the configured kbps after 4 s of streaming (§1) | Auto (bitrate and network drops: Semi) |
| F-resp | `last_response` | Set by E25 | Auto |
| F-args | `scene_active:PD B`, `scene_preview:PD B`, `muted:PD Mic`, `volume:PD Mic` (50 at −12 dB), `volume_db:PD Mic` (`-12`, `-inf` at 0), `item_visible:PD A/PD Color` and the `\|` form for the slash scene, `filter_enabled:PD Color/PD CC`, `input_text:PD Text` (edit it in OBS's properties window; live updates for GDI+ are unconfirmed), `media_playing:PD Media`; a misspelled name | Each follows OBS; the misspelled one shows `–` | Auto |
| F-rename | Rename the input, the scene and the filter in OBS | The values look the names up again | Auto |
| F-follow | Toggles following `recording`, `streaming`, `studio_mode`, `virtualcam`, `replay_buffer`, `record_paused`, `scene_active:*`, `muted:*`, `item_visible:*`, `filter_enabled:*`, `media_playing:*`, written with and without braces | Each flips when the state changes in OBS | Auto |

### G. Studio Mode

| ID | Steps | Expected | How |
|---|---|---|---|
| G1 | Studio Mode on and off, from OBS and from a key | `studio_mode`, `preview_scene`; `scene_preview:*` off while Studio Mode is off | Auto |
| G2 | Plugin `scene` with `auto` in Studio Mode | Preview changes, program doesn't | Auto |
| G3 | Plugin `scene` with `program` in Studio Mode | Program changes | Auto |
| G4 | Built-in `scene` in Studio Mode | Record what OBS does (expected: the program changes) and document it in the guide | Auto |
| G5 | `transition` with a one-off `Fade` 800 ms while the current one is Fade 300; again while the current one is Cut | Plays with the one-off settings, then `GetCurrentSceneTransition` is back to the original within duration + 2 s. With Cut current OBS reports no duration, and all transitions share one, so then select Fade: it still has 300 ms (corrected after the first pass: OBS 31 sends SceneTransitionEnded for Cut too, and the old check missed bug 1) | Auto |
| G6 | `transition` with Studio Mode off | ⚠ "Studio Mode is off; there's nothing to transition"; a one-off transition is put back at once | Auto |
| G7 | Turn Studio Mode off while preview and program differ | `preview_scene` shows `–` | Auto |

### H. Errors, reconnecting and robustness

| ID | Steps | Expected | How |
|---|---|---|---|
| H1 | Start the host with OBS closed, then start OBS; repeat, pressing a built-in key within 2 s of OBS starting | Built-in recovers within its poll backoff (at most 30 s), the plugin within its own (at most 10 s); both at once on a key press. The early press works and doesn't report "can't reach OBS" from a poll's attempt that began before OBS was listening | Auto |
| H2 | File → Exit in OBS mid-session; once more with a built-in `toggleRecord` pressed just before (the harness closes OBS politely right after the press) | Plugin logs "OBS was closed"; values `–` or off; built-in titles `–`. If the toggle was still waiting, its alert reads "OBS closed the connection before answering, so it may or may not have happened" (§1) | Auto |
| H3 | Kill OBS, then start it again | Plugin: "lost the connection to OBS at …; reconnecting", then recovers; the built-in path's first press after the restart works first time | Auto |
| H4 | Suspend OBS for 7 s during (a) a built-in `toggleRecord`, (b) the plugin's polling | (a) No double toggle: the key says, in one plain sentence, that OBS didn't answer in time so the toggle may or may not have happened, and the request isn't resent (§1); (b) the plugin drops and reconnects. Nothing hangs past 10 s | Auto |
| H5 | Leave a wrong password in place for 5 minutes, with `obs:` titles and an OBS slider on screen | Count authentication failures in OBS's log: both paths back off instead of trying every poll; the built-in path settles at one attempt every 30 s (§1) | Auto |
| H6 | Untick "Enable WebSocket server" mid-session, then tick it again | Both report the outage, then recover | Semi |
| H7 | Kick PerritoDeck's sessions from OBS's session list | Plugin waits for a key press or settings change before reconnecting (as obs-websocket asks); record what the built-in path does | Semi |
| H8 | Change the port in OBS | Plugin follows once its settings change; built-in needs `integrations.toml` and a restart | Semi |
| H9 | Switch scene collection in the OBS window | Plugin resyncs; keys naming scenes that don't exist any more show OBS's comment | Auto |
| H10 | Restart the host, and restart the plugin from the Plugins page, while recording | State comes back; no stray toggles | Auto |
| H11 | OBS run as administrator | WebSocket control works on both paths and the plugin's `hotkey` action works; PerritoDeck's own Hotkey keys don't reach OBS (Windows blocks input into elevated windows) | Manual |
| H12 | A typo in `integrations.toml` (e.g. `pasword`); then an unquoted number as the password (`password = 12345678`) | Settings → Integrations says "integrations.toml has a mistake: …" and diagnostics "integrations.toml has a mistake and is ignored"; the log names the line ("line 3: unknown field `pasword`…") without quoting the file (§1). The number appears nowhere: not in Settings, the command-line host's banner, the log or Copy diagnostics. OBS, Home Assistant and Companion stay off until it's fixed, and a `companion = true` profile's Companion section on the Buttons page shows the mistake instead of the "add this" snippet (§1) | Auto |
| H13 | Unicode, `/`, `:` and `\|` in names, in values and actions | As the plugin README's "two names in one argument" rules say | Auto |

### I. Performance

| ID | Measure | Target | How |
|---|---|---|---|
| I1 | OBS change → tablet title, built-in and plugin: harness event time to fakedeck `setTitle` time, 50 samples each | Built-in at most 2.5 s at p95; plugin under 300 ms at p95 | Auto |
| I2 | Live-loop stall: three `obs:` tokens and one OBS slider with OBS closed; time between `{cpu}` updates. Repeat with an OBS address on the LAN that doesn't answer (5 s timeouts) | About 2 s, not 6–8 s (§1) | Auto |
| I3 | Key press → OBS event latency with OBS up, and with OBS down while polling runs | Up: under 100 ms. Down: presses don't wait behind the poll for seconds | Auto |
| I4 | CPU and memory of the host, the plugin's Python and OBS over 30 minutes with 40 watched values and statistics tiles, against the same 30 minutes with PerritoDeck off | Plugin under 1 % CPU, flat memory; no visible cost in OBS | Auto |
| I5 | 20 `toggleRecord` presses in 2 s on each path | Consistent final state; no crash; every recording file closed cleanly | Auto |
| I6 | 2-hour soak: recording plus a safe stream, statistics on screen | No memory growth; timecodes right; no reconnect storms in the logs | Semi |

### J. Security and privacy

| ID | Check | How |
|---|---|---|
| J1 | With detailed logs on, neither password appears in the host log, the plugin's log lines or Copy diagnostics | Auto |
| J2 | Who can read `plugin-settings/dev.perritodeck.obs.json` (`icacls` on Windows; mode 600 on Linux and macOS) | Auto |
| J3 | An exported `.perritoprofile` contains no OBS password | Auto |
| J4 | OBS on the LAN over `ws://`: only the challenge response crosses the network, never the password. The traffic itself is unencrypted; say so in the guide | Manual |

### K. Editor, Plugins page and docs

| ID | Check | How |
|---|---|---|
| K1 | Plugins page OBS form: default address, masked password, the "insecure" box; saving reconnects | Semi |
| K2 | Editor: the OBS action and OBS slider forms, and their validation messages | Auto (CDP) |
| K3 | The live value picker lists `obs:*` and the plugin's values, with an argument field where needed | Auto (CDP) |
| K4 | "Try it" in the editor shows each OBS error in plain words | Auto (CDP) |
| K5 | The guide (`website/guide/integrations.html`) and `plugins/obs/README.md` match what real OBS did: error wording, the 30.2 requirements, the built-in values the README now lists (§1), the "What's tested" list | Manual |
| K6 | Give a plugin slider the OBS plugin's `volume` with a follow of `volume:PD Mic`, then switch "Slider sets" to another plugin: the follow and its argument are cleared, not saved unseen on the other plugin's slider | Auto (CDP) |

### L. Both paths at once

| ID | Check | How |
|---|---|---|
| L1 | Built-in and plugin set up together: OBS lists two sessions; an action from one path shows up in the other's values | Auto |
| L2 | Auto-switch to an OBS profile when OBS comes to the front | Semi |

## 4. Coverage map

Every action and value, and the cases that exercise it.

| Built-in | Cases |
|---|---|
| `scene` | B1–B3, G4 |
| `toggleRecord`, `startRecord`, `stopRecord` | B4, B5, H4, I5 |
| `toggleStream`, `startStream`, `stopStream` | B6, B7 |
| `toggleMute` | B8 |
| `toggleReplayBuffer`, `saveReplayBuffer` | B9, B10 |
| `{obs:scene}`, `{obs:recording}`, `{obs:streaming}`, follow | C1–C8, I1, I2 |
| OBS slider | D1, D2, D6–D8 |

| Plugin action | Cases | Plugin action | Cases |
|---|---|---|---|
| `scene` | E1, G2, G3 | `source-visibility` | E14 |
| `studio-mode` | E2, G1 | `filter` | E15 |
| `transition` | G5, G6 | `screenshot` | E16, A9 |
| `set-transition` | E4 | `hotkey` | E17, H11 |
| `stream` | E5 | `media` | E18 |
| `record` | E6 | `browser-refresh` | E19 |
| `record-split` | E7 | `set-text` | E20 |
| `record-chapter` | E8 | `projector` | E21 |
| `replay-buffer` | E9 | `profile` | E22 |
| `virtualcam` | E10 | `scene-collection` | E23 |
| `mute` | E11 | `caption` | E24 |
| `volume` (slider) | D3, D4, D6, E12, K6 | `request` | E25 |
| `volume-step` | E13 | `request-value` (slider) | D5, E26 |

Plugin values (30): `connected` (F-conn), `scene`, `preview_scene`, `studio_mode`
(F-scene, G1, G7), `streaming`, `recording`, `record_paused`, `replay_buffer`, `virtualcam`
(F-out), `transition`, `profile`, `scene_collection` (F-names), `stream_time`,
`record_time` (F-time), `fps`, `cpu`, `memory`, `dropped_frames`, `network_dropped_frames`,
`bitrate` (F-stats), `last_response` (F-resp), and `scene_active`, `scene_preview`,
`muted`, `volume`, `volume_db`, `item_visible`, `filter_enabled`, `input_text`,
`media_playing` (F-args, F-rename, F-follow; `volume` also D3, D4).

## 5. Automated harness

The harness is scratch tooling, not part of the repository: it needs a real OBS, a
password and a scratch folder. Keep it in a scratch folder outside the repository (e.g.
`obs-harness/`), and commit only fixes and real-world findings. It uses Python's standard
library (`py -3` on Windows) and the Playwright already in `host/app/node_modules`.

1. **`obsv.py`, the independent checker.** A small obs-websocket 5 client written
   separately from the plugin's code, so the two can't share a mistake: connect and
   authenticate, `call`, `batch`, an event queue with monotonic timestamps,
   `wait_until(predicate, timeout)` and `snapshot()`. It's the source of truth for every
   check.
2. **`fixtures.py`.** Builds `PD Test` and `PD Big` through requests (§2.2), records the 5 s
   MP4, serves `PD Browser` from a local `http.server` that counts requests, and has
   `baseline()` (§2.5) and `restore()` (back to your own profile and collection). Its
   **stream guard** refuses `StartStream`/`ToggleStream` unless the profile is `PD Test`
   and either the key (read with `GetStreamServiceSettings`) ends with
   `?bandwidthtest=true` or the server matches the approved private or local target. It
   checks again before every stream, never prints or logs the key, and never calls
   `SetStreamServiceSettings` itself.
3. **`obs_proc.py`.** Starts OBS from its install folder with
   `--disable-shutdown-check --minimize-to-tray --profile "PD Test" --collection "PD Test"`,
   and closes it politely, kills it, or suspends and resumes it (`NtSuspendProcess` through
   ctypes on Windows, `SIGSTOP`/`SIGCONT` elsewhere) for groups H and I.
4. **Layer A, `plugin_driver.py`: the plugin with no host.** Starts
   `py -3 plugins/obs/plugin.py` with `PERRITODECK_PLUGIN_DATA` in scratch and speaks
   [protocol 1](../../plugins/PROTOCOL.md) on its stdin and stdout: `hello` (with the
   settings), `watch`, `run` (with `settings.value` for the sliders); it collects
   `result`, `value` and `log`. Covers E, F, G and the plugin sliders. Fastest loop: about
   10 minutes for everything.
5. **Layer B, the real key path, on a spare port.** Starts the command-line host
   (`perritodeck-host --port <spare> --no-mdns --no-usb` with `PERRITODECK_DATA_DIR` and
   `--profile` in scratch) and pairs `perritodeck-fakedeck --host 127.0.0.1:<spare>
   --state <scratch>` with it. `fakedeck watch` prints every `setTitle key N → "…"`, which
   gives live values and follow states with timestamps; `fakedeck press N` presses keys.
   Covers B11, C, H and I without touching port 46710. Reload `integrations.toml` by
   restarting this host.
6. **Layer C, the desktop app over DevTools.** For the editor and `test_action` cases (B,
   D4, K, K6): quit the tray app (one copy at a time), start the built app with
   `PERRITODECK_DATA_DIR` in scratch and
   `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<port>`, attach with
   Playwright's `chromium.connectOverCDP`, call `page.emulateMedia({ colorScheme: null })`
   so a dark app isn't forced light, then call commands through
   `window.__TAURI_INTERNALS__.invoke(…)`: `install_catalog_plugin`,
   `set_plugin_settings`, `test_action` (e.g.
   `{ action: { type: "obs", op: "scene", scene: "PD B" } }`), `get_diagnostics`,
   `restart_plugin`. Windows only (WebView2); on Linux use Layer B.
7. **Sliders.** fakedeck can't move or watch a slider yet: it announces only the `keys`
   capability (`host/crates/fakedeck/src/lib.rs`), and the host sends slider positions
   only to tablets that announce `sliders`. Either add a `--sliders` flag that announces
   it, plus a `slide <key> <value>` command that sends `sliderChange` and printing of the
   positions the host sends (a small fakedeck change), or swipe on a real tablet over
   adb. Until then D2, D6, D7 and the tablet half of D3 need a tablet. Results are always
   checked through `obsv.py`.
8. **Report.** Each case saves JSON: steps, OBS state before and after, latencies and every
   message seen. Write a short summary after each run. Real OBS error wording goes into
   `plugins/obs/README.md` ("What's tested") and the guide; bugs go into
   `docs/next-steps.md`.

**Run order:**
1. Layer A (plugin, about 10 minutes).
2. Layer B (built-in and plugin through the host and fakedeck).
3. Robustness, group H.
4. Performance I1–I5.
5. Layer C (editor and `test_action`), with the tray app quit.
6. With a person: the soak (I6) and the manual and semi cases (A4, A8, A9, B6, B7, D8, E8,
   E10, E21, E24, H6–H8, H11, J4, K1, K5, L2), plus D2, D6, D7 and the tablet half of D3
   while fakedeck can't do sliders.

**Done when** every Auto case passes on Windows against the current OBS release, the manual
cases are recorded, real error wording is in the READMEs and the guide, and each bug found
is fixed or listed in `docs/next-steps.md`. Repeat groups A, B, C and F on Linux and macOS
when those builds are tested.
