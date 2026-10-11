# Phase 4 — Rich controls

**Goal:** keys that do more than fire once: sliders, live information, state that
reflects the computer, and a dependable cable connection for tablets that stay on a desk.

## Status (2026-10-11)

Done:
- **Sliders.** A cell can be a slider (`slider = { type = … }` in a profile, or "Slider"
  in the editor). It sets the computer's volume (wpctl/pactl on Linux, osascript on macOS,
  the Core Audio endpoint on Windows), a Home Assistant light's brightness, an OBS audio
  source's volume, or runs a program with `{value}`. Fast drags coalesce to the latest
  value; other tablets follow; changes made on the computer come back every 3 s.
  Protocol: `control`/`value` on layout keys for clients with the `sliders` capability,
  plus `sliderChange` and `setValue`.
- **Live values on keys:** `{cpu}`, `{mem}`, `{ha:<entity>}` (with unit) or
  `{ha:<entity>.<attribute>}`, `{obs:scene}`, `{obs:recording}`, `{obs:streaming}`, and
  `{plugin:<id>:<value>}` from plugins. Only values some profile uses are fetched.
- **Toggles that follow state:** `follow = "obs:recording"` (any live value) makes a
  two-state toggle show its second state while that value is on, however it changed.
- **Haptics:** a tick on key press (already), ticks every tenth while dragging a
  slider, and a longer buzz when an action reports a problem.
- **USB:** with Android's `adb` installed, the host forwards its port to every tablet
  on a cable (`adb reverse`), and Perrito tries 127.0.0.1 alongside the saved address,
  preferring whichever worked last. Settings shows the cable status.

Not done:
- Animated keys (GIF/APNG icons).
- "Now playing" on Windows (SMTC) and macOS; Linux has it through the Now playing plugin.
- Dials and a touch strip as distinct controls (sliders cover the main use).
- Live values through subscriptions (OBS events, Home Assistant WebSocket) instead of
  polling every 2 s.

Needs checking on hardware: slider feel and haptics on the Fire 7, USB on the Fire 7
(Fire OS needs Amazon's adb driver on Windows).
