# Roadmap

| Phase | Goal | Status |
|---|---|---|
| 0 — Research | Prior art, Phase 1 plan, open questions | Done ([prior-art](../prior-art.md)) |
| 1 — [Skeleton & protocol](phase-1.md) | Tablet shows a host-sent grid; presses reach the host | Done; verified end to end in emulators (Android 5.1 and 14) and on a real Fire 7 (Fire OS 7) |
| 2 — [Core loop](phase-2.md) | Pairing + encryption, discovery, real actions, profiles, host-rendered images | Done; pairing, mDNS discovery and the actions verified on Windows with a Fire 7 |
| 3 — [Desktop app & editor](phase-3.md) | Tauri tray app with drag-and-drop editor, profiles, auto-switch | Mostly done ([status](phase-3.md#status-2026-10-11)) |
| 4 — [Rich controls](phase-4.md) | USB polish, dials, touch strip, haptic patterns, live feedback (clock, CPU, now playing), animated keys | Mostly done ([status](phase-4.md#status-2026-10-11)) |
| 5 — [Plugins](phase-5.md) | Plugin SDK (TypeScript, Python), plugin manager, first-party plugins: OBS, Home Assistant, MQTT, HTTP, Spotify/MPRIS/SMTC | Mostly done ([status](phase-5.md#status-2026-10-11)); 24 first-party plugins |
| 6 — Packaging & distribution | Signed installers for all desktops, F-Droid, IzzyOnDroid, Play, website downloads | Groundwork done: draft-release workflow for all OSes with checksums, version script, download page, F-Droid/Flatpak/AUR drafts; the NSIS installer tested on Windows. Waiting on signing secrets, public repo and the go-ahead ([distribution](../distribution.md), [releasing](../releasing.md)). This phase has no plan doc of its own; those two docs and [`fdroid/`](../fdroid/README.md) cover it |
| 7 — [Stretch](phase-7.md) | Companion Satellite mode, Elgato-plugin compatibility, iOS client, real Stream Deck hardware, multi-device per host | Pulled forward: Companion mode, Stream Deck (Node.js) plugins and the iOS app written ([status](phase-7.md)); iOS not yet compiled |
| 8 — Going public | Make the repository public, put the website live, ship the first release | In progress. The open items and the decisions they need are in [next-steps](../next-steps.md) (items 8–18), the steps in order in the [launch checklist](../launch-checklist.md), and the real-world test plans, not yet run, in [testing/obs.md](../testing/obs.md) and [testing/twitch.md](../testing/twitch.md) |

Each phase starts with a plan here, ends with a review, and records decisions in [`DECISIONS.md`](../DECISIONS.md).
