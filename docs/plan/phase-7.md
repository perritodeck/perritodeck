# Phase 7 — Stretch (pulled forward: Companion and Stream Deck plugins)

**Goal:** be useful on day one to people who already run Bitfocus Companion or own Stream
Deck plugins, before packaging (Phase 6).

## Companion Satellite mode

Bitfocus Companion (MIT) drives broadcast and AV gear with thousands of modules. Its
**Satellite API** lets a remote panel register as a surface: Companion draws the buttons
and receives the presses. PerritoDeck speaks it as a client, so a tablet can be a
Companion surface without anything installed on the Companion side.

- **Turn it on:** `[companion]` in `integrations.toml` (`host`, `port`, default 16622),
  and a profile with `companion = true` (the app's "+ Profile" has a checkbox for it).
- **Per tablet:** each tablet showing a Companion profile is registered as its own
  surface (`ADD-DEVICE`, id `perritodeck-<tablet id>`, named "PerritoDeck <tablet
  name>"), sized to the tablet's grid, with key images at the tablet's key size. It's
  removed when the tablet disconnects or switches to another profile, so auto-switching
  works with Companion profiles too.
- **Drawing:** Companion's `KEY-STATE` images go to the tablet as they are (PNG when
  Companion offers it, from API 1.12; raw RGB from older versions is converted). Its
  text and colours are kept too, for tablets that show text only.
- **Presses:** key down and key up both go to Companion (`KEY-PRESS PRESSED=true/false`),
  so Companion's press and release actions work. Pausing keys on the computer also
  pauses Companion keys.
- **Connection:** pings every 2 s, reconnects with backoff (1 s up to 15 s), re-registers
  every tablet after a reconnect. While Companion is unreachable the tablet's first key
  says "Companion offline"; Settings and the Buttons page show the connection state.
- **Code:** `host/crates/companion` (wire format and client, tested against a fake
  Companion), `host/crates/host/src/companion.rs` (bridge to the deck), and the deck's
  `RemoteSurfaces` hook. Written from Companion's published Satellite API documentation.

Not done (Companion):
- The WebSocket transport (port 16623); TCP covers every Companion since 2.x.
- Advanced layouts (`LAYOUT_MANIFEST`), so no Companion rotary encoders or LED rings yet;
  PerritoDeck sliders don't map to anything in Companion.
- Button subscriptions (`ADD-SUB`): showing single Companion buttons on an ordinary
  PerritoDeck page.
- Tested only against a fake Companion. A check against a real Companion 3.x/4.x is the
  next step.
