# Twitch

Run your Twitch stream from PerritoDeck: markers, ads, clips, chat, title, category and
tags, moderation (chat modes, bans, warnings, raids, shoutouts, announcements, AutoMod,
VIPs and mods), channel points rewards and redemptions, polls and predictions. Keys can
show live status, viewers, followers, subscribers, the next ad, polls, predictions and
the latest follower, sub, cheer, raid, redemption or hype train. Plugin id:
`dev.perritodeck.twitch`; its version is the one in `perritodeck-plugin.json`.

It uses the [Twitch Helix API](https://dev.twitch.tv/docs/api/) and
[EventSub over WebSocket](https://dev.twitch.tv/docs/eventsub/handling-websocket-events/).
Python 3.8+ standard library only (the WebSocket client is the small one from the OBS
plugin in this repository).

## Setup

Twitch apps are free to register, and you use your own:

1. Go to [dev.twitch.tv/console](https://dev.twitch.tv/console) (sign in, and turn on
   two-factor authentication if Twitch asks), then **Register Your Application**.
   - **Name**: anything unique, e.g. "PerritoDeck for yourchannel".
   - **OAuth Redirect URLs**: `http://localhost` (the plugin doesn't use it, but the
     form wants one).
   - **Category**: Application Integration.
   - **Client Type**: **Public**. This matters: the plugin signs in without a client
     secret, which Twitch allows only for public clients.
2. Click **Manage** on the app and copy the **Client ID**.
3. In PerritoDeck, open **Plugins → Twitch**, paste the client ID, and untick the
   feature groups you don't want (see below).
4. Put **Sign in to Twitch** on a key and press it. The key flashes ⚠ with a code
   such as "go to twitch.tv/activate and enter ABCDEFGH", and your browser opens that
   page. Enter the code (if it isn't filled in already) and approve. The plugin checks
   for your approval in the background; the host's log says "signed in to Twitch as
   yourchannel" when it's done. Check that name: the page opens in your default
   browser, which may be signed in to a different Twitch account. The code is valid
   for 30 minutes.

The sign-in is kept in the plugin's data folder (`tokens.json`, readable by your user
only). It's refreshed automatically, and checked with Twitch (`GET /oauth2/validate`)
when the plugin starts and every hour after, whether or not any key shows a Twitch
value, as Twitch requires. Each start logs which account is signed in. When the check
finds the token no longer works, the plugin refreshes it once; if that fails too, the
log says to press **Sign in to Twitch** again. For public clients Twitch ends a
sign-in after **30 days**, so expect to sign in again about once a month.

**Sign out of Twitch** ends the sign-in at Twitch too (`POST /oauth2/revoke`), then
deletes `tokens.json`. If Twitch can't be reached it's deleted all the same, the key
shows a warning, and the log says to remove the app under Settings, Connections on
twitch.tv to end it there. Pressed while a sign-in is waiting for your approval, it
cancels that sign-in too: one approved at the last moment is ended at Twitch, not kept.
The plugin also keeps `state.json` in its data folder (the last clip and marker, the
latest events, which saved chat message is next).

### Feature groups and permissions (scopes)

The sign-in only asks for the permissions of the groups ticked on the Plugins page.
**After changing the groups, press "Sign in to Twitch" again**: until then, actions of
a newly ticked group say "the sign-in doesn't allow that", and actions of an unticked
group say "sign in again with Polls and predictions turned on in the plugin's
settings" (or the group's name).

| Group | Scopes | For |
|---|---|---|
| Always | `channel:manage:broadcast` | Markers; title, category, presets, tags, content labels, branded content |
| | `clips:edit` | Make a clip |
| | `user:write:chat` | Send chat messages |
| | `user:manage:chat_color` | Chat name colour |
| **Ads** | `channel:edit:commercial` | Run an ad |
| | `channel:read:ads` | `next_ad`, `ad_snoozes`, `preroll_free`, `ad_live`; ad break events |
| | `channel:manage:ads` | Snooze the next ad |
| **Moderation** | `moderator:manage:chat_settings` | Chat mode |
| | `moderator:manage:shield_mode` | Shield mode |
| | `moderator:manage:banned_users` | Ban, time out, unban |
| | `moderator:manage:chat_messages` | Clear chat |
| | `moderator:manage:warnings` | Warn a user |
| | `moderator:manage:announcements` | Chat announcement |
| | `moderator:manage:shoutouts` | Shout out a channel |
| | `moderator:manage:automod_settings` | AutoMod level |
| | `moderator:read:chatters` | `chatters` (`GET /chat/chatters`) |
| | `channel:manage:raids` | Raid, cancel the raid |
| | `channel:manage:vips` | VIP |
| | `channel:manage:moderators` | Moderator |
| **Channel points** | `channel:manage:redemptions` | Create, pause, enable, price rewards; complete redemptions; `reward_*` values |
| **Polls and predictions** | `channel:manage:polls`, `channel:manage:predictions` | Every poll and prediction action and value |
| **Events and totals** | `moderator:read:followers` | `last_follower`, `session_follows` |
| | `channel:read:subscriptions` | `subscribers`, `last_subscriber`, `last_gift`, `session_subs` |
| | `bits:read` | `last_cheer` |
| | `channel:read:hype_train` | `hype_train` |
| | `channel:read:redemptions` | `last_redemption` (also works with the Channel points group) |

With every group ticked the sign-in asks for these 27 scopes; with none ticked, only
for the 4 in the Always rows. Live status, viewers, uptime, title, category, tags, the
follower total, chat modes and raids need no scope.

## Actions

Most moderation and chat actions take an optional **Channel**: leave it empty for your
own, or give the name of a channel where you're a moderator. User and channel names
can be typed with or without `@`. A name no Twitch account can have (Twitch names are
up to 25 letters, digits or `_`) fails on the key without asking Twitch. New accounts
need at least 4 characters, but shorter names from Twitch's early days are left for
Twitch to judge.

When Twitch answers 401 because the token stopped working, the plugin refreshes it once
and tries again. Other 401s (Twitch also uses it for "broadcaster_id must match the
user ID", for example) show Twitch's own message, and nothing is refreshed.

### Stream

| Action | Fields | API call |
|---|---|---|
| Sign in to Twitch | | `POST id.twitch.tv/oauth2/device`, then polls `POST /oauth2/token` |
| Sign out of Twitch | | `POST id.twitch.tv/oauth2/revoke`, then deletes the saved tokens |
| Add a stream marker | Note (optional, up to 140 characters) | `POST /streams/markers` |
| Make a clip | Channel, open the clip editor | `POST /clips` |
| Open the last clip | Open the clip editor instead | Opens `clips.twitch.tv/<id>` of the last clip made from the deck |
| Set the stream title or category | Title, category (exact name, e.g. `Just Chatting`) | `GET /games?name=`, `PATCH /channels` |
| Use a stream preset | Preset 1–5 | `PATCH /channels` with the preset's title, category and tags |
| Stream tags | Replace all / add / remove; tags separated by commas | `GET` and `PATCH /channels` (`tags`, up to 10, letters and digits only) |
| Content label | Politics, drugs, gambling, profanity, sexual themes, violence; on/off/toggle | `PATCH /channels` (`content_classification_labels`) |
| Branded content | On/off/toggle | `PATCH /channels` (`is_branded_content`) |

The sign-in page and the clip editor are opened only when Twitch's address for them is
`https` on `twitch.tv` (or a subdomain). Any other sign-in address is replaced by
`https://www.twitch.tv/activate`; any other editor address isn't opened, and the log
shows what Twitch sent.

**Presets** are set on the Plugins page as `Title | Category | tag1, tag2`, e.g.
`Speedrunning all evening! | Super Mario 64 | Speedrun, English`. The category and tags
are optional (`Just a new title` only changes the title). The manifest can't list
your presets by name, so the key picks "Preset 1" to "Preset 5".

### Ads

| Action | Fields | API call |
|---|---|---|
| Run an ad | 30 s to 3 min | `POST /channels/commercial` (affiliates and partners, while live) |
| Snooze the next ad | | `POST /channels/ads/schedule/snooze`; warns when that was the last snooze |

### Chat

| Action | Fields | API call |
|---|---|---|
| Send a chat message | Message, channel | `POST /chat/messages` |
| Send a saved chat message | Messages separated by `\|`, one after the other or at random, channel | `POST /chat/messages`; the turn is remembered per message list |
| Chat name colour | `blue`, `blue_violet`, `cadet_blue`, `chocolate`, `coral`, `dodger_blue`, `firebrick`, `golden_rod`, `green`, `hot_pink`, `orange_red`, `red`, `sea_green`, `spring_green`, `yellow_green`, or `#RRGGBB` (Turbo or Prime) | `PUT /chat/color` |
| Chat announcement | Message, colour (channel, blue, green, orange, purple), channel | `POST /chat/announcements` |

### Moderation

| Action | Fields | API call |
|---|---|---|
| Chat mode | Emote-only, followers-only, slow, subscribers-only, unique chat, chat delay for non-moderators; on/off/toggle; slow seconds (3–120), minutes followed (0–129600) or delay (2, 4, 6 s); channel | `GET` and `PATCH /chat/settings` |
| Shield mode | On/off/toggle, channel | `GET` and `PUT /moderation/shield_mode` |
| Ban or time out a user | User, 1 min to 1 week (10 minutes unless you pick another) or "Ban (no end)", reason, channel | `POST /moderation/bans`; a key saved without a length times out for 10 minutes, never bans for good |
| Unban a user | User, channel | `DELETE /moderation/bans` |
| Warn a user | User, reason (required; Twitch shows it), channel | `POST /moderation/warnings` |
| Clear chat | Channel | `DELETE /moderation/chat` |
| Raid a channel | Channel | `POST /raids` |
| Cancel the raid | | `DELETE /raids` |
| Shout out a channel | Channel | `POST /chat/shoutouts` (Twitch limits how often) |
| AutoMod level | 0–4, channel | `PUT /moderation/automod/settings` (`overall_level`) |
| VIP | User; add/remove/toggle | `GET`, `POST`, `DELETE /channels/vips` |
| Moderator | User; add/remove/toggle | `GET`, `POST`, `DELETE /moderation/moderators` |

### Channel points

**Twitch only lets an app change the rewards it created**, and complete their
redemptions. Make the rewards you want to control with **Create a channel points
reward**; for rewards made on twitch.tv the plugin says so. The `reward_enabled` and
`reward_paused` values work for every reward.

| Action | Fields | API call |
|---|---|---|
| Create a channel points reward | Title, cost, description, viewers type a message | `POST /channel_points/custom_rewards` |
| Channel points reward | Reward title; pause/unpause/toggle, turn on/off/toggle, or set the cost | `PATCH /channel_points/custom_rewards` |
| Complete a redemption | Reward title; mark as done or reject and refund | Oldest `UNFULFILLED` redemption, then `PATCH /channel_points/custom_rewards/redemptions` |

### Polls and predictions

| Action | Fields | API call |
|---|---|---|
| Start a poll | Question (60 characters), 2–5 choices separated by commas (25 characters each), 15–1800 s | `POST /polls` |
| End the poll | End and show the results, or end and hide it | `PATCH /polls` (`TERMINATED` or `ARCHIVED`) |
| Start a prediction | Question (45 characters), 2–10 outcomes separated by commas, 30–1800 s to predict | `POST /predictions` |
| Lock the prediction | | `PATCH /predictions` (`LOCKED`) |
| Resolve the prediction | Winning outcome, by name or number (`1`, `2`…) | `PATCH /predictions` (`RESOLVED`) |
| Cancel the prediction | | `PATCH /predictions` (`CANCELED`, refunds) |

## Values

Values take an argument: a **channel name**, or `me` for the channel you signed in with
(an empty argument is `me` too). Reward values take the **reward's title**. Toggles can
follow any value that is `on`/`off`.

| Value | Example title | Shows | Updated |
|---|---|---|---|
| `live` | `{plugin:dev.perritodeck.twitch:live:me}` | `on` while live | 10 s; at once with EventSub |
| `viewers` | `{plugin:dev.perritodeck.twitch:viewers:me} watching` | Viewers (`0` offline) | 10 s |
| `uptime` | `{plugin:dev.perritodeck.twitch:uptime:me}` | `2h 05m`, `45m`, `offline` | 10 s |
| `title` | `{plugin:dev.perritodeck.twitch:title:me}` | Stream title | 30 s |
| `category` | `{plugin:dev.perritodeck.twitch:category:somechannel}` | Category | 30 s |
| `tags` | `{plugin:dev.perritodeck.twitch:tags:me}` | `English, Speedrun` | 30 s |
| `followers` | `{plugin:dev.perritodeck.twitch:followers:me}` | Follower total | 30 s |
| `subscribers` | `{plugin:dev.perritodeck.twitch:subscribers:me} subs` | Subscriber total (your own channel only) | 60 s |
| `chatters` | `{plugin:dev.perritodeck.twitch:chatters:me} in chat` | People in chat (where you're a moderator) | 60 s |
| `chat_mode` | `{plugin:dev.perritodeck.twitch:chat_mode:emote}` | `on`/`off` for `emote`, `followers`, `slow`, `subscribers`, `unique` or `delay`; add `@channel` for another channel | 30 s |
| `next_ad` | `Ad in {plugin:dev.perritodeck.twitch:next_ad:me}` | Countdown `12:34` to the next scheduled ad; `–` if none | 30 s, counts down every second |
| `ad_snoozes` | `{plugin:dev.perritodeck.twitch:ad_snoozes:me} snoozes` | Snoozes left | 30 s |
| `preroll_free` | `{plugin:dev.perritodeck.twitch:preroll_free:me}` | Pre-roll free time left, `10:00` | 30 s |
| `ad_live` | `{plugin:dev.perritodeck.twitch:ad_live:me}` | `on` while an ad break runs | 30 s; at once with EventSub |
| `reward_enabled` | `{plugin:dev.perritodeck.twitch:reward_enabled:Hydrate!}` | `on` when the reward is on | 30 s |
| `reward_paused` | `{plugin:dev.perritodeck.twitch:reward_paused:Hydrate!}` | `on` when paused | 30 s |
| `reward_pending` | `{plugin:dev.perritodeck.twitch:reward_pending:Hydrate!}` | Redemptions waiting (`50+` above 50; rewards the plugin made) | 30 s |
| `poll` | `{plugin:dev.perritodeck.twitch:poll:me}` | The running poll's question | 10 s |
| `poll_leader` | `{plugin:dev.perritodeck.twitch:poll_leader:me}` | `Mario 62%` | 10 s |
| `poll_time` | `{plugin:dev.perritodeck.twitch:poll_time:me}` | `1:30` left, then `ended` | every second |
| `prediction` | `{plugin:dev.perritodeck.twitch:prediction:me}` | The open prediction's question | 10 s |
| `prediction_leader` | `{plugin:dev.perritodeck.twitch:prediction_leader:me}` | Outcome with the most points, `No 55%` | 10 s |
| `prediction_time` | `{plugin:dev.perritodeck.twitch:prediction_time:me}` | `0:50` to predict, `locked`, `Yes won`, `cancelled` | every second |
| `last_follower` | `{plugin:dev.perritodeck.twitch:last_follower:me}` | Name | EventSub (polled every 30 s if EventSub is down) |
| `last_subscriber` | `{plugin:dev.perritodeck.twitch:last_subscriber:me}` | Name (new subs, gifted subs' recipients, resubs) | EventSub |
| `last_gift` | `{plugin:dev.perritodeck.twitch:last_gift:me}` | `Name: 5` subs gifted | EventSub |
| `last_cheer` | `{plugin:dev.perritodeck.twitch:last_cheer:me}` | `Name: 100` (`Anonymous: 100`) | EventSub |
| `last_raid` | `{plugin:dev.perritodeck.twitch:last_raid:me}` | `Name (42)` | EventSub |
| `last_redemption` | `{plugin:dev.perritodeck.twitch:last_redemption:me}` | `Name: Hydrate!` | EventSub |
| `hype_train` | `{plugin:dev.perritodeck.twitch:hype_train:me}` | `L2 45%` (level, progress to the next) while one runs | EventSub |
| `session_follows` | `+{plugin:dev.perritodeck.twitch:session_follows:me}` | Follows since the plugin started | EventSub |
| `session_subs` | `+{plugin:dev.perritodeck.twitch:session_subs:me}` | Subs and resubs since the plugin started | EventSub |
| `last_clip` | `{plugin:dev.perritodeck.twitch:last_clip}` | Time (`20:15`) of the last clip made from the deck | when you clip |
| `last_marker` | `{plugin:dev.perritodeck.twitch:last_marker}` | Stream time (`1:04:04`) of the last marker | when you mark |

A finished poll or prediction stays on keys for 10 minutes. The `last_*` event values
are remembered across restarts; the session counts start at 0. Values show `–` when
signed out, for an unknown channel or reward, for a name no Twitch account can have
(the log says so, and the other channels' values still work), when a permission is
missing (the log says which group to turn on), or when Twitch can't be reached.

### How values are fetched

Everything is fetched only while a key shows it. Helix values are polled: live status
every 10 s (one `GET /streams` for all channels), title, category, tags, followers and
chat modes every 30 s (one `GET /channels` for all channels), subscribers
(`GET /subscriptions`) and chatters (`GET /chat/chatters`, with you as the moderator)
every 60 s, the ad schedule (`GET /channels/ads`) and rewards every 30 s, polls and
predictions every 10 s. Countdowns tick locally between requests. Channel names are
turned into IDs once (`GET /users`, up to 100 names per request). Twitch refuses the
whole request (400) for one name it doesn't accept, so names that can't be Twitch names
are left out of it, and when Twitch refuses a request all the same, the plugin asks
about its names one at a time: one bad name never blanks the others. A name Twitch
refuses is logged once and not asked about again until the settings change or you sign
in again. The plugin reads Twitch's `Ratelimit-Remaining` and `Ratelimit-Reset`
headers and pauses until the reset when the bucket is nearly empty or Twitch answers
429.

The event values come from one EventSub WebSocket session
(`wss://eventsub.wss.twitch.tv/ws`), open only while signed in, with **Events and
totals** ticked, and while a key shows an event value (or `live`/`ad_live`, which then
update at once). After Twitch's welcome message the plugin creates the subscriptions
with `POST /eventsub/subscriptions` (Twitch closes a session with none after 10 s):

| Value | Subscription (version) |
|---|---|
| `last_follower`, `session_follows` | `channel.follow` (2), with you as moderator |
| `last_subscriber`, `session_subs` | `channel.subscribe` (1), `channel.subscription.message` (1) |
| `last_gift` | `channel.subscription.gift` (1) |
| `last_cheer` | `channel.cheer` (1) |
| `last_raid` | `channel.raid` (1), raids *to* the channel |
| `last_redemption` | `channel.channel_points_custom_reward_redemption.add` (1) |
| `hype_train` | `channel.hype_train.begin`, `.progress`, `.end` (2) |
| `ad_live` | `channel.ad_break.begin` (1) |
| `live` | `stream.online`, `stream.offline` (1) |

It answers pings straight away and goes back to waiting; a ping doesn't count as a sign
of life, and a socket read timeout on its own never ends the session. What does is no
message at all (keepalives included) for longer than the keepalive time from Twitch's
welcome message (30 s, asked for in the URL) plus 5 s: then it reconnects and
resubscribes. It follows `session_reconnect` to the new URL (subscriptions carry over;
the old connection is closed once the new one is welcomed), drops duplicate message
IDs, and stops a subscription Twitch revokes (logged). A subscription Twitch refuses is
logged once and tried again after 15 s, then 30 s, 1 min and so on up to 10 min (a
revoked one waits 10 min). It's tried at once when keys change, and on the new
connection after one with working subscriptions was lost. When Twitch refuses every
subscription (a mod-only `channel.follow` where you aren't a moderator, say, or an
outage), the plugin closes the session, since Twitch would close a session with none
after 10 s anyway, and opens a new one only when the next try is due.
Subscriptions are removed when no key needs them any more, and the session closes when
nothing is left.

## Tested and untested

Unit tests run against a fake `id.twitch.tv`, a fake Helix and a fake EventSub
WebSocket server that follow the documented request and answer shapes: the device code
flow, scopes per feature group, which addresses are opened, refresh without a client
secret and single-use refresh tokens, refresh on a 401 for a bad token but not for
"Missing scope" or "must match", `/oauth2/validate` at start and hourly with no values
shown, revoking on sign-out (and signing out when that fails), `Client-Id` and
`Authorization` headers, rate-limit headers and 429, every Helix endpoint above, names
that can't exist and names Twitch refuses, signing out while an approval is on its way,
and on the EventSub side the welcome, subscription bodies (type, version, condition,
`websocket` transport with the session ID), notifications, duplicates, keepalives and
their timeout, pings (answered, and not taken for a lost connection), frames that
arrive in pieces, retrying refused subscriptions (also when Twitch closes a session
with none), `session_reconnect` and revocation.

**Not yet tried against the real Twitch service.** Unconfirmed in particular:

- whether the device and token endpoints want `scopes` (as in the docs' examples) or
  `scope`; whether `verification_uri` already carries the code; `slow_down` handling;
- the `/channels/ads` field formats (`next_ad_at` and `last_ad_at` are read as Unix
  seconds or RFC 3339, whichever comes) and the snooze endpoint's answer when no snoozes
  are left (assumed 400/429 with a message);
- that `GET /chat/settings` shows `non_moderator_chat_delay` only with `moderator_id`;
- that rewards made on twitch.tv answer 403 to `PATCH` and to redemption requests;
- the hype train v2 event fields (`level`, `progress`, `goal`) and the redemption,
  gift and resub event fields beyond the documented follow, subscribe, cheer and raid
  ones;
- that revoking the access token on sign-out also ends its refresh token (the app
  should drop off Settings, Connections on twitch.tv);
- which names make `GET /users` answer 400 for the whole request (the plugin then asks
  about the names one at a time), and whether accounts with names under 4 characters
  still exist;
- the exact wording of Twitch's error messages, including which 401s mean a bad token
  (the plugin asks `/oauth2/validate` when it doesn't recognise the wording).

### Testing against real Twitch

`docs/testing/twitch.md` is the plan for trying the plugin on real Twitch accounts: what
to set up (your own app, a test broadcaster and a test viewer), which cases are safe on
your main channel and which never are, and the order to run them in. The harness, HTTP
trace and reward cleanup in [`plugins/tools/`](../tools/) run that plan's cases through the
plugin's own protocol.
