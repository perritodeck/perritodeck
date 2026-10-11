# Philips Hue

Turn Hue lights and rooms on and off, dim them, change their colour and start scenes,
through the Hue Bridge on your local network. Nothing goes through the internet except,
once, finding the bridge if you don't enter its address.

## Setup

1. On the Plugins page, enter the **Bridge address** (the Hue app shows it under
   Settings → My Hue system → the bridge → IP address). Leave it blank and the plugin
   asks Philips' bridge finder (`https://discovery.meethue.com`) once and remembers the
   answer.
2. Put the **Pair with the bridge** action on a key.
3. Press the round link button on top of the bridge, then that key within 30 seconds.

The bridge gives the plugin a username (an app key). It's kept in the plugin's data
folder (`hue.json`), not in the settings and never in the log. If the bridge forgets it
(a factory reset, or you removed the app in the Hue app), pair again.

The plugin talks to the bridge over plain HTTP on your network, which the bridge still
serves for this (version 1) API. **Connect with HTTPS** switches to HTTPS; the bridge's
certificate is signed by Philips' own authority, not a public one, so HTTPS also needs
**Skip certificate check**, which is off unless you turn it on.

## Lights and rooms

Use the names from the Hue app. Rooms, zones and other groups are looked for first,
then single lights, ignoring upper and lower case. A blank name (or `all`) means every
light.

## Actions

| Action | Fields |
|---|---|
| Pair with the bridge | none |
| Turn a light or room on or off | light or room; switch on or off / turn on / turn off |
| Set the brightness (slider) | light or room; the slider sets 1–100 %, 0 turns it off |
| Brighter | light or room; step in % (default 10); turns the light on |
| Dimmer | light or room; step in % (default 10) |
| Set a colour | light or room; a colour like `#ff8800` (or `#f80`) |
| Set a shade of white | light or room; colour temperature in kelvin (2700 warm, 6500 daylight) |
| Start a scene | room; scene name. Without a room, the newest scene with that name |

Colours are turned into Hue's xy colour space with the RGB-to-xy formula from the Hue
developer documentation, and pulled into the light's own colour range (its "gamut") for
single lights; rooms leave that to each light. Kelvin becomes mirek (1,000,000 / K),
kept within the light's range (153–500 by default).

## Values

| Title | Shows |
|---|---|
| `{plugin:dev.perritodeck.hue:on:Living room}` | `on` or `off`; a toggle can follow it |
| `{plugin:dev.perritodeck.hue:brightness:Desk lamp}` | `0`–`100`, `0` when off; a slider can follow it |
| `{plugin:dev.perritodeck.hue:on:all}` | `on` when any light is on |

A room is `on` when any of its lights is; its brightness is the average of the lights
that are on. A light the bridge can't reach, a name that doesn't exist, or a bridge that
doesn't answer shows `–`. Values are read every 5 seconds while a key shows them, and
right after an action.

## API details

Hue API v1: `POST /api` with `{"devicetype": "perritodeck#<computer name>"}` to pair;
`GET /api/<username>/lights`, `/groups` and `/scenes`; `PUT .../lights/<id>/state` and
`PUT .../groups/<id>/action` with `on`, `bri` (1–254), `bri_inc`, `xy`, `ct` or `scene`.
Errors come back as `[{"error": {"type": …}}]` with HTTP 200 (type 1: unknown username,
101: link button not pressed).

## Not tested yet

Against a real Hue Bridge: the tests use a fake bridge that mimics the documented
answers. Unconfirmed on hardware: HTTPS to the bridge, the exact xy values a lamp ends
up showing, and how the bridge reports rooms whose lights are unreachable.
