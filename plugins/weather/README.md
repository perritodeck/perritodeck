# Weather

Current weather and today's forecast for any place, from [Open-Meteo](https://open-meteo.com)
(free, no account or key; their data is CC BY 4.0).

## Setup

Nothing to set up. On the Plugins page, **Units** switches between metric (°C, km/h) and
imperial (°F, mph).

## Places

The place goes after the value id in a title:

- a name: `{plugin:dev.perritodeck.weather:temp:Lisbon}`; add a country or region after a
  comma to pick the right one: `Paris, France`, `Paris, Texas`, `Springfield, US`;
- or coordinates: `{plugin:dev.perritodeck.weather:temp:38.72,-9.14}`.

Names are looked up once with Open-Meteo's geocoding API (and remembered while the
plugin runs); each place's weather is fetched every 10 minutes while a key shows it, and
retried after 2 minutes if that fails.

## Actions

| Action | Fields |
|---|---|
| Refresh the weather | none; fetches every place your keys show now |

## Values

| Title | Shows |
|---|---|
| `{plugin:dev.perritodeck.weather:temp:Lisbon}` | `18°C` |
| `{plugin:dev.perritodeck.weather:feels:Lisbon}` | `16°C` |
| `{plugin:dev.perritodeck.weather:condition:Lisbon}` | `Light rain`, `Partly cloudy`, `Thunderstorm`… |
| `{plugin:dev.perritodeck.weather:icon:Lisbon}` | an emoji: ☀️ 🌤️ ⛅ ☁️ 🌫️ 🌦️ 🌧️ 🌨️ ❄️ ⛈️ (🌙 at night) |
| `{plugin:dev.perritodeck.weather:humidity:Lisbon}` | `72%` |
| `{plugin:dev.perritodeck.weather:wind:Lisbon}` | `13 km/h` |
| `{plugin:dev.perritodeck.weather:high:Lisbon}` | today's high, `21°C` |
| `{plugin:dev.perritodeck.weather:low:Lisbon}` | today's low, `13°C` |
| `{plugin:dev.perritodeck.weather:rain_chance:Lisbon}` | today's highest chance of rain, `40%` |

Anything unknown (a place that can't be found, no internet) shows `–`.

## API details

Forecast: `GET https://api.open-meteo.com/v1/forecast` with `latitude`, `longitude`,
`current=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m,is_day`,
`daily=temperature_2m_max,temperature_2m_min,precipitation_probability_max`, `timezone=auto`,
`forecast_days=1`, `wind_speed_unit=kmh|mph` and, for imperial, `temperature_unit=fahrenheit`.
Geocoding: `GET https://geocoding-api.open-meteo.com/v1/search?name=…&count=…&language=en&format=json`
(the answer has no `results` when nothing matches). Condition texts follow the WMO
weather codes in Open-Meteo's docs.

## Not tested yet

Against the live Open-Meteo service: the tests use a fake server with the documented
request and answer shapes.
