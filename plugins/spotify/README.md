# Spotify

Control Spotify from PerritoDeck: play, pause, skip, shuffle, repeat, volume, like the
current track, play a playlist, album or track, add the current track to a playlist,
and move playback to another device. Keys can show the track, artist, album, position
and more. Plugin id: `dev.perritodeck.spotify`.

It uses the [Spotify Web API](https://developer.spotify.com/documentation/web-api), so
it controls Spotify on any of your devices (phone, speaker, desktop app, web player),
not just this computer. For the desktop app on this computer without an account, the
**Now playing** plugin (`dev.perritodeck.media`) works too. Python 3.8+ standard
library only.

**Controlling playback needs Spotify Premium.** With a free account the values still
show what's playing, but play/pause, skip, volume and the like answer "controlling
playback needs Spotify Premium".

## Setup

Spotify doesn't let one app sign in everybody, so you register your own (free, two
minutes):

1. Go to [developer.spotify.com/dashboard](https://developer.spotify.com/dashboard), sign
   in, accept the terms and **Create app**.
   - **App name** and **description**: anything, e.g. "PerritoDeck".
   - **Redirect URI**: `http://127.0.0.1:47123/callback` (exactly this; Spotify
     doesn't accept `localhost`, only `127.0.0.1`). Click **Add**.
   - **Which API/SDKs are you planning to use?** tick **Web API**.
   - Save.
2. Open the app's **Settings** and copy the **Client ID**. You don't need the client
   secret: the plugin signs in with PKCE, which doesn't use one.
3. In PerritoDeck, open **Plugins → Spotify**, paste the client ID, and leave the
   sign-in port at `47123` (change it only if something else uses that port, and change
   the redirect URI in the dashboard to match).
4. Put the **Sign in to Spotify** action on a key and press it. Your browser opens
   Spotify's sign-in page; approve, and the page says you're signed in. The key flashes
   ⚠ "finish signing in in your web browser" while it waits: that's expected.

The sign-in is kept in the plugin's data folder (`tokens.json`, readable by your user
only) and refreshed automatically; you sign in once. **Sign out of Spotify** forgets
it. Changing the client ID asks you to sign in again.

Spotify's redirect rules also allow registering `http://127.0.0.1/callback` (no port)
and using any port; set the sign-in port to `0` for that. Untested.

### Spotify's limits for your own app (as of October 2026)

Apps in Spotify's *Development mode* (every app you create yourself) have these limits,
introduced in February 2026:

- **The app's owner must have Spotify Premium**, or the app stops working.
- **Up to 5 users per app.** Others must be added by email under the app's **User
  Management** in the dashboard before they can sign in. You can share an app with a
  few people in your household, or each make your own.
- New developers can create a limited number of apps (one at first, raised later).

Spotify can change these; see
[the Web API docs](https://developer.spotify.com/documentation/web-api).

### Permissions (scopes)

The sign-in asks for:

| Scope | For |
|---|---|
| `user-read-playback-state`, `user-read-currently-playing` | Values, play/pause, shuffle, repeat, volume steps, finding devices |
| `user-modify-playback-state` | All playback actions |
| `user-library-read` | The `liked` value and the like toggle |
| `user-library-modify` | Like / unlike |
| `playlist-modify-public`, `playlist-modify-private` | Add the current track to a playlist |

## Actions

| Action | Fields | API call |
|---|---|---|
| Sign in to Spotify | | Authorization code + PKCE (S256) via a one-shot server on `127.0.0.1` |
| Sign out of Spotify | | Deletes the saved tokens |
| Play or pause | | `GET /me/player`, then `PUT /me/player/pause` or `/play` |
| Next track / Previous track | | `POST /me/player/next`, `/previous` |
| Shuffle | Turn on or off / on / off | `PUT /me/player/shuffle?state=` |
| Repeat | Next mode (off → all → one) / off / all / one | `PUT /me/player/repeat?state=off\|context\|track` |
| Spotify volume (slider) | | `PUT /me/player/volume?volume_percent=` |
| Volume up / Volume down | By how much (%, default 10) | reads the device volume, then sets it |
| Like the current track | Like or unlike / like / unlike | `GET /me/library/contains`, `PUT` or `DELETE /me/library?uris=` |
| Play a playlist, album or track | Spotify link or URI, device (optional) | `PUT /me/player/play` with `context_uri` (playlist, album, artist, show) or `uris` (track, episode) |
| Add the current track to a playlist | Playlist link, URI or ID | `POST /playlists/{id}/items` |
| Play on another device | Device name, keep playing there | `GET /me/player/devices`, then `PUT /me/player` with `device_ids` |

Links can be `https://open.spotify.com/playlist/…` (from **Share → Copy link**, with or
without `?si=…` or `/intl-xx/`) or `spotify:playlist:…` URIs. Device names match
exactly first, then by part of the name, ignoring case ("living room" finds "Living
Room TV"); an unknown name lists the devices Spotify knows. A device only appears
while Spotify is open on it.

Liking uses the library endpoints Spotify introduced in February 2026
(`/me/library`, with Spotify URIs) instead of the removed `/me/tracks`; adding to a
playlist uses `/playlists/{id}/items` instead of the removed `/playlists/{id}/tracks`.

## Values

| Value | Example title | Shows |
|---|---|---|
| `track` | `{plugin:dev.perritodeck.spotify:track}` | Track (or podcast episode) name |
| `artist` | `{plugin:dev.perritodeck.spotify:artist}` | Artists, comma-separated (the show for an episode) |
| `album` | `{plugin:dev.perritodeck.spotify:album}` | Album |
| `playing` | `{plugin:dev.perritodeck.spotify:playing}` | `on` while playing, `off` when paused or stopped |
| `shuffle` | `{plugin:dev.perritodeck.spotify:shuffle}` | `on` / `off` |
| `repeat` | `Repeat {plugin:dev.perritodeck.spotify:repeat}` | `off`, `all` or `one` |
| `liked` | `{plugin:dev.perritodeck.spotify:liked}` | `on` when the track is in your Liked Songs |
| `volume` | `{plugin:dev.perritodeck.spotify:volume}%` | The device's volume, 0–100 (a slider can follow it) |
| `progress` | `{plugin:dev.perritodeck.spotify:progress}` | `1:23 / 3:45` |
| `device` | `{plugin:dev.perritodeck.spotify:device}` | The device playing, e.g. `Desk speaker` |

Toggles can follow `playing`, `shuffle` and `liked`. Values show `–` when signed out,
when nothing is playing, or when Spotify can't be reached.

The plugin asks Spotify every 5 seconds while something plays and every 15 seconds
otherwise (`GET /me/player`, plus one `GET /me/library/contains` when the track
changes), and right after each action. These values take no argument, so the host
doesn't tell the plugin whether a key shows them: it polls whenever it's signed in.
When Spotify answers 429 (too many requests), the plugin waits the `Retry-After` time
before asking again, and actions say "try again in N s" meanwhile.

## Tested and untested

Unit tests run against a fake Spotify (accounts token endpoint and Web API) that
follows the documented request and answer shapes: the PKCE S256 challenge (checked
against the RFC 7636 appendix B test vector, and verified by the fake token endpoint),
the `state` check, the redirect URI, token refresh before expiry and on 401 (with and
without a new refresh token), revoked refresh tokens, 429 with `Retry-After`, the
`PREMIUM_REQUIRED` and `NO_ACTIVE_DEVICE` errors, and every endpoint above.

**Not yet tried against the real Spotify service.** In particular unconfirmed:
whether `uris` for `/me/library` is accepted as a query parameter (the docs' example
uses the query string), the exact error `reason` strings, the per-type scope rules of
`/me/library`, and the portless redirect URI with a dynamic port.
