# Twitch plugin: real-world test plan

The Twitch plugin (`plugins/twitch/`, `dev.perritodeck.twitch`) has 37 actions and 34 live
values, and until this pass it had only run against fakes: a fake id.twitch.tv, a fake
Helix and a fake EventSub (`plugins/twitch/test_plugin.py`). This plan says which accounts
to set up, every case to run from harmless to risky, which ones are safe on the main
channel, and what a harness can do unattended. It's for the maintainer and contributors.

**How the plugin talks to Twitch**
- **Helix REST** (`https://api.twitch.tv/helix`) with the user's token and the app's client
  ID. Chat goes out through `POST /chat/messages`; nothing reads chat (no IRC).
- **id.twitch.tv** for sign-in (the device code flow), refresh and the hourly validation.
- **EventSub over WebSocket** (`wss://eventsub.wss.twitch.tv/ws`) for follows, subs,
  cheers, raids, redemptions, hype trains, ad breaks and going live or offline.
- **Rate limits:** the plugin reads `Ratelimit-Remaining` and `Ratelimit-Reset` and pauses
  every Helix call when 2 or fewer are left ("Twitch asked to slow down; try again in N s").
  Per-endpoint limits (raids, shoutouts, VIP and mod changes, ad cooldowns) aren't checked
  ahead; the plugin shows Twitch's error.
- **Timeouts:** 6 s per HTTP call; the host gives an action 10 s to answer.
- **Values** are polled only while a key shows them (10–60 s depending on the value), plus
  events.

**Columns used below**
- **Main?** Yes = safe on the main channel. Caution = public but reversible, or only
  visible to the channel's own viewers. No = never on the main channel.
- **How:** **Auto** = the harness runs it unattended once signed in. **Semi** = the harness
  runs it while a person acts or watches. **Manual** = a person only.

## 1. Fixed before this pass: re-check these

The research for this plan found these bugs, and they were fixed before the pass (plugin
1.1.2, and 1.1.3 for the last row), tested only against the fakes. Each has a case below. If a case fails, the bug
goes back on the list in `docs/next-steps.md`.

| Bug | Re-check in |
|---|---|
| The `chatters` value called `GET /helix/chatters`, which doesn't exist (Twitch's is `/helix/chat/chatters`), and the fake copied the mistake, so the value always showed the placeholder | B5 |
| EventSub dropped and rebuilt its session whenever Twitch sent a WebSocket Ping that wasn't followed by data within 10 s | M1 |
| The sign-in wasn't validated at start, and not at all while no value key was showing (Twitch asks apps to validate hourly) | A8 |
| Sign out only deleted the local tokens; it didn't revoke them at Twitch | A12 |
| An EventSub subscription that failed once stayed dead until a key or a setting changed | M3 |
| Every 401 without "scope" in it refreshed the token, so a polled value such as `subscribers:<another channel>` refreshed on every poll, with a misleading message | B4 |
| Ban with no duration saved meant a **permanent** ban, though the manifest's default is 600 s | G5 |
| The content-label picker showed "Politics" but saved nothing, so the key failed with "pick a content label" | C6 |
| `moderator:read:automod_settings` was requested but never used | A3 (the consent page's list) |
| Addresses Twitch sends back (the device page, the clip editor) were opened without checking them; now only https pages on twitch.tv are opened | A3, D4 |
| One invalid channel name among the live values could blank them all, because the names share one `GET /users` request and Twitch refuses the whole batch. Names that can't be Twitch names are now left out of that request, and when Twitch refuses a batch all the same the plugin asks about the names one at a time, so only the bad name's values show the placeholder | B8 |
| Pressing Sign out while an approval was on its way (Twitch had approved, the plugin was still asking which account it is) left the plugin signed in again, with `tokens.json` back | A15 |
| When Twitch refused every wanted subscription (a mod-only `channel.follow` where T isn't a moderator, or an outage), the backoff didn't hold: Twitch closes a session with no subscription after 10 s, and the plugin reconnected and posted again about every 12 s. Now it closes such a session itself and opens a new one only when the next try is due | L10 |
| On Windows, saving `tokens.json` failed ("Access is denied") whenever another thread was reading it at that moment, and a failed save right after a refresh lost Twitch's new single-use refresh token. The save now waits briefly for the reader (Spotify's token save got the same fix) | A10 |

Known and **not** being fixed now; record what happens:
- A session reconnect closes the old connection without reading what's left on it, so an
  event arriving during the switch can be lost (low risk).
- A clip counts as made as soon as Twitch answers 202, without checking about 15 s later
  that it exists (D4).
- An ad's `retry_after` isn't shown, so the person isn't told the cooldown (I3).
- "Replace all tags" with an empty field clears every tag, on purpose (C5).
- Which names make Twitch refuse a whole `GET /users` request is unconfirmed. The plugin
  leaves out only names that can't be Twitch names (over 25 characters, or anything but
  letters, digits and `_`) and passes shorter-than-4 names from Twitch's early days on
  to Twitch; whether such accounts still exist is also unconfirmed (B8).
- Sign-in can take longer than the host's 10 s action limit on a slow network (A3).
- Sign out with an expired token refreshes it first so there's something to revoke: two
  calls of up to 6 s each, which on a very slow network can pass the host's 10 s limit.
  The key then shows the host's timeout while the sign-out still finishes (A12).
- "Resolve" on a prediction that's still open sends RESOLVED directly; whether Twitch
  allows that is unknown (K5).
- `perritodeck-host action` can't run plugin actions; use the running host.

## 2. Prerequisites

### 2.1 Accounts

| Who | What | Why |
|---|---|---|
| **T**, test broadcaster | A Twitch account that signs in to the plugin. Turn on "Store past broadcasts" (for markers) and allow clips. Two-factor authentication on | Keeps every public effect off the main channel |
| **V**, test viewer | A second account. Follows T and chats | Target for bans, timeouts, warnings, VIP and mod; votes and redeems where possible |
| **R**, raid target | A third account that can stream | Raid and shoutout target; raids T to fire `channel.raid`. Must be live at the same time as T: a second OBS profile or computer streaming a static "test, please ignore" scene |
| **M**, the main channel | The project's or maintainer's real channel | Only the cases marked Yes or Caution; the Affiliate cases only in a later pass (next row) |
| Affiliate or Partner | **Ads, channel points, polls and predictions need an Affiliate or Partner channel**, and so do real subs, cheers and hype trains. A new account can't get there quickly (it needs followers, broadcast days and average viewers) | **Not in this pass** (decided on 2026-10-13; `docs/DECISIONS.md`, 2026-10-15): it uses test accounts only, so sections I, J and K are deferred. A later pass runs them on an Affiliate's channel, offline where possible: M if it's Affiliate or Partner, or a consenting Affiliate signed in with their own account. L6 (subs, gifts, cheers, hype trains) is still open: the Twitch CLI's mock server (§4) could stand in for those events in this pass if the maintainer approves the download, or L6 waits for the Affiliate pass |

Use **one browser profile per account**, and check which account the default browser is
signed in to before pressing Sign in: the device page opens in the default browser and
approves for whoever is signed in there.

### 2.2 The developer app

Register one at [dev.twitch.tv/console](https://dev.twitch.tv/console) (the registering
account needs two-factor authentication):
- **OAuth Redirect URLs:** `http://localhost` (unused, but the form wants one).
- **Category:** Application Integration.
- **Client Type: Public.** The plugin signs in with the **device code flow** and no client
  secret, which Twitch allows only for public clients.

The client ID isn't secret; share it with the harness. Any account can sign in with it.
For the Confidential-app error case (A6), register a second app with Client Type
Confidential.

Every user registering their own app is a big hurdle; whether the project should own a
public client ID instead is an open decision in `docs/next-steps.md`.

### 2.3 PerritoDeck

- The desktop app from current `main` with the bundled Twitch plugin, or the command-line
  host with the plugin installed in its data folder. Use a separate data folder
  (`PERRITODECK_DATA_DIR`) so the test sign-in never mixes with a real one.
- Ways to press actions: the editor's "Try it", a tablet, fakedeck, or the stdio harness
  (§4).
- The sign-in is stored in `<settings folder>/plugin-data/dev.perritodeck.twitch/tokens.json`
  (plaintext JSON, owner-only where the OS supports it). Delete the test data folder when
  done.
- Optional, with the maintainer's OK because it's a download: the
  [Twitch CLI](https://dev.twitch.tv/docs/cli/) for its EventSub mock server and
  `event trigger` (subs, gifts, cheers, hype trains, reconnects and revocations without
  spending money).
- Optional, real money on M: gift a sub from V, or cheer 100 bits. The only way to see real
  sub, cheer and hype-train payloads.

## 3. Test cases, harmless to risky

### A. Sign-in and the token's life

| ID | Steps | Expected | Main? | How |
|---|---|---|---|---|
| A1 | Empty client ID, press Sign in | "set your Twitch app's client ID on the Plugins page" | Yes | Auto |
| A2 | A random 30-character client ID, press Sign in | "Twitch doesn't know that client ID (…); check it, and that the app is "Public"". Record Twitch's exact message | Yes | Auto |
| A3 | Public app, all groups ticked; press Sign in, approve as T in T's browser profile | The key shows the code; the browser opens twitch.tv/activate with the code filled in; the consent page asks for the 27 scopes of all groups, without `moderator:read:automod_settings` (record how Twitch lists them); the log says "signed in to Twitch as <T's login>" within about 5 s; `tokens.json` has `login` = T and only the user, SYSTEM and Administrators can read it (`icacls`); no token in the host log or diagnostics. Note how long the key took (10 s limit) | Sign in as T | Manual |
| A4 | Press Sign in, then Deny on Twitch | Log: "Twitch refused the sign-in (…)" (record it); no `tokens.json` | Yes | Manual |
| A5 | Press Sign in twice; approve only the second code | The first attempt logs "replaced by a new sign-in"; the second succeeds | Yes | Semi |
| A6 | Try the Confidential app | Record what the device and token requests return, and check the advice reaches the person (polling failures only reach the log) | Yes | Manual |
| A7 | Untick Polls and Moderation, sign in; press Start a poll and Ban; then tick Polls and press Start a poll without signing in again | The consent page lists fewer scopes; "sign in again with Polls and predictions turned on in the plugin's settings" / the same for Moderation; then "the sign-in doesn't allow that (channel:manage:polls); …"; no Helix call | Yes | Semi |
| A8 | One value key showing for 75 minutes with the HTTP trace on; restart the plugin; then only action keys for 75 minutes | A `GET /oauth2/validate` at start and one an hour later in both setups, with action keys only too (§1). Each start logs which account is signed in | Yes | Auto |
| A9 | Stop the plugin, set `expires_at` in `tokens.json` to the past, start it, press Add marker | Log "Twitch sign-in refreshed"; `refresh_token` changed; record the real `expires_in` | Yes | Semi |
| A10 | Force expiry, then press 5 different keys at once | Exactly one refresh (refresh tokens are single-use); all 5 succeed | T | Auto |
| A11 | twitch.tv → Settings → Connections → disconnect the app; press a key; watch the event keys | 401, refresh fails, "Twitch ended the sign-in; press Sign in again"; values go to the placeholder; EventSub revocation logged (record the status) | Yes | Manual |
| A12 | Press Sign out; then sign in again, let the token expire (as in A9) and press Sign out | The trace shows `POST /oauth2/revoke`; `tokens.json` gone; values show the placeholder; the WebSocket closes; twitch.tv → Connections no longer lists the app (§1). With the expired token: a refresh, then the revoke; record how long the key took | Yes | Semi |
| A13 | Change the client ID while signed in | Actions say "not signed in: press "Sign in to Twitch" first" | Yes | Auto |
| A14 | Restart the host while signed in | Still signed in; `last_*` values come back; `session_*` start again at 0 | Yes | Semi |
| A15 | Press Sign in, approve on Twitch, and press Sign out within a second or two, while the plugin is still fetching the approval | Stays signed out: no "signed in to Twitch as …" after "signed out of Twitch", and no `tokens.json` comes back (§1). The token Twitch just gave is revoked, so twitch.tv → Connections doesn't list the app | Yes | Semi |

### B. Read-only values (safe on any channel)

| ID | Steps | Expected | Main? | How |
|---|---|---|---|---|
| B1 | `live`, `viewers`, `uptime` for `me` (T offline, then live), a large public channel, and the harness's unused name (§4) | off / 0 / offline, then on / N / "Xm"; the public channel matches twitch.tv; the unused name shows the placeholder and logs once | Yes | Auto (going live: Manual) |
| B2 | `title`, `category`, `tags` for `me` and a public channel; change T's title on the dashboard | The key updates within 30 s; emoji and accents display correctly | Yes | Auto |
| B3 | `followers` for `me` and a public channel | Matches twitch.tv | Yes | Auto |
| B4 | `subscribers:me` on T, then `subscribers:<public channel>` with the trace on | T: record the answer (0 or an error); the other channel: Twitch's error, and no refresh on every poll (§1) | Yes | Auto |
| B5 | `chatters:me`; `chatters:<channel T doesn't moderate>` | `me`: a number, from `GET /chat/chatters` (§1); the other channel: the placeholder, and Twitch's 403 in the log (T isn't a moderator there) | Yes | Auto |
| B6 | `chat_mode` for each mode on `me`; `slow@<public channel>`; `delay@<public channel>` | on/off match the dashboard; `delay` on another channel shows the placeholder | Yes | Auto |
| B7 | `next_ad`, `ad_snoozes`, `preroll_free`, `ad_live` on T (not Affiliate) | Record Twitch's answer; the values show the placeholder | Yes | Auto |
| B8 | 10 channel names in live values, plus one invalid name such as `bad name`; then add a 3-character name | One `GET /streams` every 10 s; only the invalid name shows the placeholder, the log says it can't be a Twitch channel name, and the other 10 work (§1). The short name: record whether Twitch takes it in the batch or refuses the batch (the trace then shows the names asked one at a time, and the others still work), and whether an account has it (§1, known) | Yes | Auto |
| B9 | Signed out, with value keys showing | All show the placeholder; no Helix calls in the trace | Yes | Auto |

### C. Stream details on T (public and reversible; T live or offline)

| ID | Steps | Expected | Main? | How |
|---|---|---|---|---|
| C1 | `set-info` with only a title, then only a category ("Just Chatting") | The dashboard updates; the title and category values follow within about a second | Caution | Auto |
| C2 | Category "just chatting" (lower case), and "Mincraft" | Record whether matching ignores case; the typo gives "Twitch has no category called 'Mincraft'; use its exact name, as on twitch.tv" | Caution | Auto |
| C3 | A 141-character title; all fields empty | Fails in the plugin; nothing sent | Yes | Auto |
| C4 | `preset` 1–5: with tags, without tags, title only, an empty preset | The PATCH carries only the parts given; the empty preset says so | Caution | Auto |
| C5 | `tags`: set, add, remove; "two words", "a-b", 11 tags, "Español"; set with an empty field | Invalid tags fail in the plugin; record whether Twitch takes non-ASCII; an empty set clears every tag (confirm that's wanted) | Caution | Auto |
| C6 | `label`: each of the six labels with toggle, on and off; also a key saved without touching the picker | The dashboard shows the labels; the untouched key uses the label it shows (§1) | Caution | Auto / Semi |
| C7 | `branded`: toggle, on, off | The dashboard checkbox follows | Caution | Auto |

### D. Markers and clips

| ID | Steps | Expected | Main? | How |
|---|---|---|---|---|
| D1 | `marker` while T is offline | Twitch's error on the key (record the status and wording) | Yes | Auto |
| D2 | `marker` while live, with a 200-character note | The marker appears in Video Producer with the note cut to 140; `last_marker` shows the stream time | Yes (private) | Semi |
| D3 | `marker` with "Store past broadcasts" off | Record the error | T | Semi |
| D4 | `clip` on T while live, "open editor" ticked | The clip editor opens; the clip exists after about 15 s; `last_clip` updates. Delete the clip afterwards | Caution | Semi |
| D5 | `clip` on T while offline; `clip` with Channel = R while R is live | Offline: record the error. R: a clip on R's channel (delete it) | Caution | Semi |
| D6 | `open-last-clip` in both modes; again with no clip made yet | Opens the clip or the editor; "no clip made from the deck yet" | Yes | Auto |

### E. Chat

| ID | Steps | Expected | Main? | How |
|---|---|---|---|---|
| E1 | `chat` on T; with Channel = R; a 501-character message | The messages appear (V watches); the long one is cut to 500 | Caution | Semi |
| E2 | To R, a message with a term on R's blocked list or caught by strict AutoMod | "Twitch didn't send it: <reason>" (record it) | T | Semi |
| E3 | `chat-saved`, in order across a host restart; then random order | Messages come in order and the turn survives the restart; random varies | Caution | Semi |
| E4 | `chat-color`: "Blue Violet"; "#9146FF" without Turbo or Prime; "mauve" | The named colour applies; record Twitch's error for hex; "mauve" fails in the plugin | Yes | Auto |
| E5 | `announce` in each colour on T; in a channel where T isn't a moderator | A coloured announcement appears; 403 with Twitch's message | Caution | Semi |
| E6 | 25 `chat` presses in 30 s | Record when Twitch starts refusing; other actions still work right after | T | Auto |

### F. Chat modes, Shield Mode and AutoMod on T (V watches)

| ID | Steps | Expected | Main? | How |
|---|---|---|---|---|
| F1 | `chat-mode`: each mode with toggle, on and off; slow 3 and 120; followers 0 and 10; delay 2, 4, 6; out-of-range values | V's chat shows each mode; out-of-range values fail in the plugin; record whether `delay` shows without a moderator id | No | Semi |
| F2 | `chat-mode` with Channel = R, first with T as R's moderator, then not | Works as moderator; 403 otherwise | No | Semi |
| F3 | `shield`: toggle, on, off | The Shield Mode banner on the dashboard | No | Semi |
| F4 | `automod` levels 0 to 4, then back to the original level | The dashboard's moderation settings follow | No | Semi |

### G. Moderating users (on T, target V only; never on the main channel)

| ID | Steps | Expected | Main? | How |
|---|---|---|---|---|
| G1 | `warn` V with a reason; without a reason | V must acknowledge before chatting; no reason fails in the plugin | No | Semi |
| G2 | `ban` V for 1 minute (a timeout), then `unban` | V can't chat, then can | No | Semi |
| G3 | `ban` V with no end, then `unban` | Banned, then unbanned | No | Semi |
| G4 | Ban the harness's unused name (§4); ban T itself; ban V twice; unban someone who isn't banned | "Twitch has no user called <name>"; record Twitch's errors for the rest | No | Auto |
| G5 | Run `ban` with no duration in the settings (harness only, V only) | A 10-minute (600 s) timeout, not a permanent ban: V can chat again after 10 minutes (§1) | No | Auto |
| G6 | `clear-chat` | Every message removed | No | Semi |
| G7 | `vip` toggle on V | A fresh T: record the 425 "Build a Community" error. Where allowed: V gets the badge; toggling removes it | No (main: Caution, V only) | Semi |
| G8 | `mod`: add, add again, remove; mod V while V is a VIP | Adding twice is harmless; record Twitch's error for the VIP case | No | Semi |
| G9 | Ban V in R's channel (Channel = R) with T as R's moderator, then not | Works as moderator; 403 otherwise | No | Semi |

### H. Raids and shoutouts (T and R live)

| ID | Steps | Expected | Main? | How |
|---|---|---|---|---|
| H1 | `raid` R, then `cancel-raid` within 90 s | The raid countdown appears on T, then is cancelled; R isn't raided | No | Semi |
| H2 | `raid` R and let it complete | R receives the raid | No | Semi |
| H3 | Raid while T is offline; raid an offline R; cancel with no raid pending | Record each error | No | Semi |
| H4 | R raids T (from R's chat) | T's `last_raid` shows "R (n)" through EventSub within seconds | Yes (as target) | Semi |
| H5 | `shoutout` R; again within 2 minutes; shout out T itself; while offline | The shoutout card appears; record the limit errors; another action works at once afterwards (one endpoint's 429 mustn't pause the others) | Caution | Semi |

### I. Ads (Affiliate or Partner only)

| ID | Steps | Expected | Main? | How |
|---|---|---|---|---|
| I1 | On T: `ad`, `snooze-ad` | Record Twitch's "not Affiliate" errors | Yes | Auto |
| I2 | On M, live, with an ad scheduled: `snooze-ad` until the snoozes run out | `next_ad` moves 5 minutes later; `ad_snoozes` drops; "snoozed; that was the last snooze for now"; then Twitch's error (429 per its docs). Other actions still work right after | Caution (only delays ads) | Semi |
| I3 | On M, live, when an ad is due anyway: one 30 s `ad`; a second one at once | `ad_live` on within about a second through EventSub; the second shows the cooldown error (record whether `retry_after` should be shown) | Caution (interrupts viewers) | Manual |
| I4 | `ad` while M is offline | Twitch's "must be streaming live" error | Yes | Auto |

### J. Channel points (Affiliate or Partner only)

| ID | Steps | Expected | Main? | How |
|---|---|---|---|---|
| J1 | On T: `reward-create`; reward values | Record the 403 wording | Yes | Auto |
| J2 | On M, offline: `reward-create` "PerritoDeck test" (cost 1, a prompt, viewer input) | The reward appears, enabled at once (visible to viewers) | Caution | Semi |
| J3 | `reward` on the test reward: pause, unpause, disable, enable, cost 5 | The dashboard and `reward_*` values follow | Caution | Auto |
| J4 | `reward` on a reward made on twitch.tv | "Twitch only lets the app that made a reward change it or its redemptions; …" (confirms the real 403) | Yes | Auto |
| J5 | V redeems twice; `redemption`: fulfil, then reject and refund | `last_redemption` "V: PerritoDeck test"; `reward_pending` 2 → 1 → 0; V refunded on the second; then "no redemptions waiting" | Caution | Semi |
| J6 | A duplicate title; a 50-character title | Record Twitch's duplicate error; the title is cut to 45 | Caution | Auto |
| J7 | Remove the test reward | There's no delete action. Try the dashboard (it may not delete app-made rewards); otherwise `plugins/tools/twitch_cleanup.py` with the same client ID, run by the maintainer (§4) | Caution | Semi |

### K. Polls and predictions (Affiliate or Partner only)

| ID | Steps | Expected | Main? | How |
|---|---|---|---|---|
| K1 | On T: `poll-start`, `prediction-start` | Record Twitch's 403 errors | Yes | Auto |
| K2 | On M (record whether offline is allowed): a 15 s poll with 3 choices; V votes | `poll`, `poll_leader` and `poll_time` follow; "ended" shows for 10 minutes | Caution | Semi |
| K3 | `poll-end`: show results, then hide; with no poll running; a 61-character question, a 26-character choice, 6 choices, 14 s | Results shown or hidden; "no poll is running"; the rest fail in the plugin | Caution | Auto |
| K4 | A 30 s prediction; V predicts; lock; lock again; resolve by name; a new one resolved by number; one more, cancelled | Locked; "the prediction is already locked"; "<X> won" with points paid; refunds on cancel | Caution (cancel refunds) | Semi |
| K5 | Resolve while still open; resolve with an outcome that doesn't exist | Record what Twitch does (§1, known); an error listing the outcomes | Caution | Auto |

### L. EventSub

| ID | Steps | Expected | Main? | How |
|---|---|---|---|---|
| L1 | Events group on, a `last_*` key showing; check the trace and `GET /eventsub/subscriptions` | One WebSocket; subscriptions created within 10 s, all enabled | Yes | Auto |
| L2 | V follows T | `last_follower` = V within about 2 s; `session_follows` +1 | Yes | Semi |
| L3 | T goes live, then offline | `live` flips within about 2 s instead of up to 10 s | Yes | Semi |
| L4 | `live:<R>` with Events on | Record whether Twitch allows a `stream.online` subscription for another broadcaster over WebSocket | Yes | Auto |
| L5 | A follow event for a channel T doesn't moderate | That subscription fails and is logged once; the others keep working | Yes | Auto |
| L6 | Subs, gifts, cheers, hype train: for real on M (money), or the Twitch CLI's `event trigger` against its mock server | The fields map correctly (hype train level, progress and goal) | Yes | Auto (mock) / Manual (real) |
| L7 | Turn the Events group off | No WebSocket (`netstat`); event values show the placeholder | Yes | Auto |
| L8 | Remove every event key | The session closes within about 5 s | Yes | Auto |
| L9 | Block `eventsub.wss.twitch.tv` in the firewall | `last_follower` falls back to polling within 30 s; EventSub retries from 2 s up to 60 s | Yes | Semi |
| L10 | The only event key is one Twitch refuses: `last_follower` for a channel T doesn't moderate. Watch for 10 minutes with the trace on | The refused subscription is tried again after 15 s, 30 s, 1 min and so on, with at most one new WebSocket and one POST per try; between tries there's no session open (§1). Record whether Twitch's own close of an empty session (code 4003) is ever seen | Yes | Auto |

### M. Resilience and soak

| ID | Steps | Expected | Main? | How |
|---|---|---|---|---|
| M1 | 2-hour idle soak with value and event keys, trace on | No "EventSub connection:" warnings and no reconnects caused by Pings: the WebSocket stays the same one unless Twitch sends `session_reconnect` (§1); Helix calls well under 800 a minute; one validation an hour | Yes | Auto |
| M2 | Pull the network for 2 minutes, then restore it | "couldn't reach Twitch" logged once; actions fail in under 10 s; values and EventSub recover; events arrive afterwards | Yes | Semi |
| M3 | Pull the network just after a reconnect, while it subscribes | The failed subscription recovers by itself: at once on the next connection, otherwise after 15 s, 30 s, 1 min and so on up to 10 min; no key or setting change needed (§1) | Yes | Semi |
| M4 | Sleep and wake the computer; kill the plugin's Python process | The host restarts the plugin; still signed in; resubscribed | Yes | Semi |
| M5 | On T only: the harness reads until `Ratelimit-Remaining` ≤ 2 | The plugin pauses ("try again in N s") and resumes after the reset | T | Auto |
| M6 | A throttled network (needs a tool, so the maintainer's OK), with a multi-request action (reward, or a ban of a new user) | Record whether the key shows the host's timeout while the action still completes; document it | T | Manual |

### N. Platforms

| ID | Steps | Expected | Main? | How |
|---|---|---|---|---|
| N1 | Windows, the installed app: the sign-in browser, the clip editor, `open-last-clip` | The browser opens and stays open after a plugin restart | Yes | Semi |
| N2 | macOS and Linux: the same | `open` / `xdg-open` work; `tokens.json` is mode 600 | Yes | Semi |

### Order

1. A1, A2 (no account involved).
2. A3, A7, A13 as T. Check `tokens.json` says `login` = T.
3. B1–B9 (read-only, safe anywhere).
4. Changes only T can see: E4, D1, D6.
5. T goes live: D2–D5, then C1–C7.
6. Public chat: E1–E3, E5, E6.
7. Chat settings: F1–F4.
8. EventSub: L1–L10. Start M1 here and let it run in the background.
9. Moderation on V: G1–G9.
10. H5 (shoutouts), then H1–H4 (raids).
11. Errors and the token's life: A4–A6, A8–A12, A14, A15, M2–M6, N1, N2.
12. Affiliate cases, deferred to a later pass on an Affiliate's channel (M if it qualifies,
    or a consenting Affiliate's; §2.1). Offline first: J, then K (cancel predictions to
    refund). Live last: I2, then I3 (one 30 s ad).

**Never on the main channel:** bans, timeouts, warnings, clearing chat, chat modes and
Shield Mode during a real stream, AutoMod changes while live, raids.

## 4. What can be automated

1. **Regression tests for §1 exist** in `plugins/twitch/test_plugin.py`
   (`test_pings_and_slow_frames_keep_the_session`, `test_pings_alone_are_not_keepalives`,
   `test_chatters_where_you_dont_moderate`, `test_sign_out_revokes_the_token`,
   `test_validation_at_start_and_hourly`, `test_failed_subscriptions_are_tried_again`,
   `test_a_bad_channel_name_spoils_only_its_own_values`,
   `test_sign_out_while_the_approval_is_on_its_way`,
   `test_refused_subscriptions_wait_when_twitch_closes_unused_sessions`, and the ban and
   label defaults).
   Extend them with the recorded contract fixtures from item 3.
2. **The stdio harness exists:** `plugins/tools/twitch_live_test.py`
   ([plugins/tools/README.md](../../plugins/tools/README.md)). It starts `plugin.py` the way
   the host does ([plugins/PROTOCOL.md](../../plugins/PROTOCOL.md): `hello` with the settings,
   `watch`, `run`; it reads `result`, `value` and `log`), with `PERRITODECK_PLUGIN_DATA` in a
   test folder of its own, never the real data folder. It runs §3 in the order above and
   writes a JSON pass/fail report. Read-only stages run unattended once a person has signed
   in; the harness shows the device code and waits, and never enters credentials. A sign-in
   approved by another account, or not approved in time, is signed out at once (the
   plugin's Sign out cancels the attempt and revokes the token). It stops for a typed yes
   before every case with a public effect (chat, announcements, title and category changes,
   clips, markers, raids, moderation, ads, polls, predictions, rewards), listing each action
   and the channel it acts in. It checks that the signed-in account is T (`tokens.json` and
   `GET /users`) before anything risky. Before the first risky case it looks V and R up on
   Twitch (both must exist, and V must follow T), shows their IDs and creation dates, and
   asks for a typed yes that these are the test accounts. It moderates only V in T's
   channel, so F2 and G9 (in R's channel) stay manual, though it checks V and R before
   showing their steps too. With `--main-channel <M>` it refuses
   moderation, chat modes, Shield Mode, AutoMod, clearing chat, warnings and raids whenever
   the broadcaster is M, including the by-hand steps of F2, G9 and H3. A case that stops early puts back what it changed: V unbanned, mod
   and VIP removed, chat modes and Shield Mode off, AutoMod back, the raid cancelled.
   For the "unknown user" cases (B1, G4) it makes up a name that is valid in form (16 random
   letters and digits) and checks with `GET /users` that no account has it before using it,
   so a ban can never hit a real stranger.

   ```
   py -3 plugins/tools/twitch_live_test.py --dry-run-fake
   $env:PERRITODECK_TWITCH_CLIENT_ID = "<your app's client ID>"
   py -3 plugins/tools/twitch_live_test.py --broadcaster <T> --viewer <V> --raid-target <R> --main-channel <M>
   py -3 plugins/tools/twitch_live_test.py --broadcaster <T> --only B --auto-only
   ```

   The first command runs every case the fakes from `test_plugin.py` can answer, answering
   yes by itself; a live run never does. Sections I, J and K are reported as "needs an
   Affiliate channel" unless `--no-skip-affiliate` is given, and this pass doesn't give it
   (test accounts only, §2.1). It's for the later pass on an Affiliate's channel:
   `--broadcaster <M> --main-channel <M> --no-skip-affiliate --only I,J,K` when that's M.
   The Affiliate channel is usually
   M, so with `--no-skip-affiliate` every other case with a public effect is refused unless
   `--only` names it by its ID, and `--main-channel` is required. A8 and M1 run at the end of stage 11 (M1 not in the
   background, because only one plugin process can hold a sign-in); the tools README lists
   where else the harness differs from §3. `--soak-minutes`, `--validate-minutes` (A8) and
   `--l10-minutes` set the long cases.
3. **The HTTP trace and recorder exist:** `plugins/tools/twitch_trace.py`. The harness runs
   the plugin under it by default (`--no-trace` runs `plugin.py` itself). It imports the
   plugin and patches `urllib` to log every request and answer to a JSONL file in the test
   folder. Tokens, device codes and the Authorization header are redacted; the client ID
   stays. `ID_URL`, `HELIX_URL` and `EVENTSUB_URL` are overridden the way the tests already
   do, with no change to the plugin.

   ```
   py -3 plugins/tools/twitch_trace.py summary <trace.jsonl>
   py -3 plugins/tools/twitch_trace.py fixtures <trace.jsonl> <fixtures.json>
   ```

   `summary` gives call counts, the busiest minute of Helix calls, the validation rhythm,
   EventSub sessions and Twitch's real error bodies. `fixtures` turns a trace into **recorded
   contract fixtures** for the fake server, with the accounts replaced by stand-ins (the
   format is in the script), so the unit tests can use Twitch's real answers (ads formats,
   the snooze answer, 403 wording, `delay` without a moderator id). `test_plugin.py`'s fake
   doesn't replay them yet.
4. **Twitch CLI mocks** (if the download is approved): point `EVENTSUB_URL` and
   `HELIX_URL` at the mock server for paid events, reconnects and revocations.
5. **The cleanup script exists:** `plugins/tools/twitch_cleanup.py` deletes app-made
   rewards (DELETE with the same client ID), for the maintainer to run. It's a dry run
   unless `--yes` is given, and `--yes` needs `--broadcaster`: nothing is deleted unless the
   sign-in belongs to that account. `--sign-in` uses a sign-in of its own (only
   `channel:manage:redemptions`, revoked afterwards) for M or a friend's channel.

   ```
   py -3 plugins/tools/twitch_cleanup.py --match "PerritoDeck test"
   py -3 plugins/tools/twitch_cleanup.py --broadcaster <T> --match "PerritoDeck test" --yes
   ```

**Needs a person:**
- Creating the developer app and the accounts (two-factor authentication, email, phone).
- Approving the device code in the browser: the harness never enters passwords or signs
  in.
- Going live in OBS on T and R.
- Acting as V: following, chatting, acknowledging warnings, voting, redeeming.
- Checking what appears on twitch.tv: banners, announcements, the shoutout card, the raid
  pop-up, dashboard settings.
- Revoking the app in Connections; unplugging the network; putting the computer to sleep.
- In the later Affiliate pass: every Affiliate case on the main channel or a friend's
  channel, and any spending on subs or bits.

**Done when** A–H and L–N pass on Windows with T, V and R, L6 only if it runs on the mock
server (the Affiliate cases are deferred to a later pass, and L6 is still open, §2.1);
Twitch's real wording and payloads are in the fixtures and
`plugins/twitch/README.md` ("What's tested"); and each bug found is fixed or listed in
`docs/next-steps.md`.
