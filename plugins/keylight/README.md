# Elgato Key Light

Turn lights for Elgato Key Light, Key Light Air and Ring Light on and off, and set
their brightness and shade of white, over your local network. Light Strip works for
on, off and brightness.

This plugin is made by PerritoDeck contributors. It isn't affiliated with, endorsed by
or supported by Elgato or Corsair; "Elgato" and "Key Light" are their trademarks and are
used here only to say which lights it works with.

## Setup

1. Find each light's address: in Elgato's Control Center app, open the light's
   settings; or look for it in your router's list of devices. Giving the lights fixed
   addresses in the router saves updating them later.
2. On the Plugins page, enter the addresses in **Your lights' addresses**, separated by
   commas: `192.168.1.50, 192.168.1.51` or `key-light-left.local`. The port is 9123
   unless you add another (`192.168.1.50:9124`).

There's no account, password or pairing. The lights aren't found automatically
(mDNS/Bonjour discovery isn't built in), so list them by hand.

## Which light

Each action and value takes a **Light**: blank (or `all`) means every light in the list;
otherwise its address as listed, the address without the port, or the name it has in
Control Center (`Left key`). An address that isn't in the list is used as it is.

## Actions

| Action | Fields |
|---|---|
| Turn a light on or off | light; switch on or off / turn on / turn off. Switching with several lights turns them all off if any is on, otherwise all on |
| Set the brightness (slider) | light; the slider sets 1–100 %, 0 turns it off |
| Brighter | light; step in % (default 10); turns the light on |
| Dimmer | light; step in % (default 10) |
| Set the shade of white (slider) | light; the slider goes from 2900 K (warm) to 7000 K (cool) |
| Warmer white | light; step in kelvin (default 300) |
| Cooler white | light; step in kelvin (default 300) |

A Light Strip has colours rather than shades of white, so the white actions only warn
for it.

## Values

| Title | Shows |
|---|---|
| `{plugin:dev.perritodeck.keylight:on:192.168.1.50}` | `on` or `off`; a toggle can follow it |
| `{plugin:dev.perritodeck.keylight:brightness:Left key}` | `0`–`100`, `0` when off; a slider can follow it |
| `{plugin:dev.perritodeck.keylight:temperature:192.168.1.50}` | `4700 K` (to the nearest 50) |
| `{plugin:dev.perritodeck.keylight:on:all}` | `on` when any light is on |

Values are read every 5 seconds while a key shows them, and right after an action. A
light that doesn't answer shows `–`; so does `temperature` for a Light Strip.

## API details

`GET` and `PUT http://<light>:9123/elgato/lights` with
`{"numberOfLights": 1, "lights": [{"on": 1, "brightness": 20, "temperature": 213}]}`
(a PUT can send just the fields that change; the light answers with its new state).
`temperature` is 143–344, where kelvin = 1,000,000 / value (about 7000–2900 K).
`GET /elgato/accessory-info` gives `displayName`, used to match lights by name. A Light
Strip reports `hue` and `saturation` instead of `temperature`.

## Not tested yet

Against real lights: the tests use fake lights that answer with the request and answer
shapes above. Elgato doesn't publish a reference for this API that we could find; the
shapes come from widely used community descriptions, so confirm on hardware: the lowest
brightness a Key Light accepts (said to be 3), whether lights with
`numberOfLights` above 1 exist, and Ring Light behaviour.
