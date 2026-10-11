# Roadmap

| Phase | Goal | Status |
|---|---|---|
| 0 — Research | Prior art, Phase 1 plan, open questions | Done ([prior-art](../prior-art.md)) |
| 1 — [Skeleton & protocol](phase-1.md) | Tablet shows a host-sent grid; presses reach the host | Done; verified end to end in emulators (Android 5.1 and 14); Kindle check pending |
| 2 — [Core loop](phase-2.md) | Pairing + encryption, discovery, real actions, profiles, host-rendered images | Done (Kindle check pending) |
| 3 — [Desktop app & editor](phase-3.md) | Tauri tray app with drag-and-drop editor, profiles, auto-switch | Mostly done ([status](phase-3.md#status-2026-10-11)) |
| 4 — [Rich controls](phase-4.md) | USB polish, dials, touch strip, haptic patterns, live feedback (clock, CPU, now playing), animated keys | Mostly done ([status](phase-4.md#status-2026-10-11)) |
| 5 — [Plugins](phase-5.md) | Plugin SDK (TypeScript, Python), plugin manager, first-party plugins: OBS, Home Assistant, MQTT, HTTP, Spotify/MPRIS/SMTC | Mostly done ([status](phase-5.md#status-2026-10-11)) |
| 6 — Packaging & distribution | Signed installers for all desktops, F-Droid, IzzyOnDroid, Play, website downloads | Release workflow and store metadata ready ([distribution](../distribution.md), [releasing](../releasing.md)) |
| 7 — [Stretch](phase-7.md) | Companion Satellite mode, Elgato-plugin compatibility, iOS client, real Stream Deck hardware, multi-device per host | Pulled forward: Companion mode and Stream Deck (Node.js) plugins done ([status](phase-7.md)) |

Each phase starts with a plan here, ends with a review, and records decisions in [`DECISIONS.md`](../DECISIONS.md).
