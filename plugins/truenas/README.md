# TrueNAS

Show TrueNAS pool health and space, alerts, app states and CPU use on keys; start,
stop and restart apps, scrub a pool, dismiss alerts and run cloud sync or replication
tasks. For TrueNAS Community Edition (formerly SCALE) 25.04 and newer. Plugin id:
`dev.perritodeck.truenas`.

## Which API, and why

TrueNAS 25.04 deprecated the REST API (`/api/v2.0`), and TrueNAS 26 removes it. This
plugin therefore speaks the **JSON-RPC 2.0 API over a WebSocket** at
`wss://<nas>/api/current`, the API TrueNAS supports from 25.04 on, with a small
WebSocket client built on the Python 3.8+ standard library (no extra packages).

- It logs in with `auth.login_ex` and the `API_KEY_PLAIN` mechanism (user name + key),
  or with `auth.login_with_api_key` when no user name is set (older style). The key is
  only ever sent inside TLS. (TrueNAS 26 also offers SCRAM logins, which never send the
  key; this plugin doesn't do SCRAM yet.)
- It keeps **one connection** open and logged in, because TrueNAS allows only 20
  logins a minute and then blocks for 10 minutes. After a refused key it waits 2
  minutes before trying again (or until you change the settings).
- Jobs (starting an app, a scrub, a sync) only answer when they finish, so the key
  waits up to 6 seconds, reports an error if one came by then, and otherwise answers
  OK while the job runs on.

Older TrueNAS SCALE versions (24.10 and earlier) don't have `/api/current` and aren't
supported.

## Setup

1. In TrueNAS, open the user menu (top right) → **My API Keys** (or **Credentials →
   Users → API Keys**) → **Add**. Pick the user the key acts as and copy the key: it's
   shown only once.

   Permissions follow that user's role. Read-only administrator is enough for the
   values; starting/stopping apps, scrubs, dismissing alerts and running tasks need a
   full administrator (or a role with the matching write rights).
2. In PerritoDeck, open **Plugins → TrueNAS** and fill in:
   - **TrueNAS address**: `https://truenas.local` (must be **https**: TrueNAS revokes an
     API key that is ever sent over plain http, so the plugin refuses `http://`).
   - **User name**: the user the key belongs to, e.g. `truenas_admin`.
   - **API key**: the key, e.g. `1-AbCdEf…`.
   - **Skip certificate check**: tick it while TrueNAS uses its own self-signed
     certificate.

## Actions

| Action | Fields | What it does (API method) |
|---|---|---|
| Start an app | App name | `app.start` |
| Stop an app | App name | `app.stop` |
| Restart an app | App name | `app.stop`, then `app.start` (not `app.redeploy`, which also pulls newer images) |
| Start or stop an app | App name | Stops it when running or deploying, otherwise starts it (`app.query` first) |
| Scrub a pool | Pool name | `pool.scrub.scrub` with `START` |
| Dismiss all alerts | | `alert.dismiss` for every alert `alert.list` shows that isn't dismissed yet |
| Run a backup task | Task type (cloud sync / replication), task number | `cloudsync.sync` or `replication.run` with the task's id |

App names are as TrueNAS shows them under **Apps** (e.g. `jellyfin`). A task's number
is its id: the number in the task's address, or from `midclt call cloudsync.query`.

## Values

| Value | Example title | Shows |
|---|---|---|
| `pool:<pool>` | `{plugin:dev.perritodeck.truenas:pool:tank}` | Pool status: `ONLINE`, `DEGRADED`, `FAULTED`… |
| `pool_ok:<pool>` | `{plugin:dev.perritodeck.truenas:pool_ok:tank}` | `on` when healthy and `ONLINE`, else `off` |
| `pool_used:<pool>` | `{plugin:dev.perritodeck.truenas:pool_used:tank}%` | Space used, 0–100 |
| `alerts` | `{plugin:dev.perritodeck.truenas:alerts} alerts` | Active alerts (not dismissed) |
| `alert_level` | `{plugin:dev.perritodeck.truenas:alert_level}` | The worst active level: `INFO`, `NOTICE`, `WARNING`, `ERROR`, `CRITICAL`, `ALERT`, `EMERGENCY`, or `none` |
| `app:<app>` | `{plugin:dev.perritodeck.truenas:app:jellyfin}` | `running`, `stopped`, `deploying`, `stopping` or `crashed` |
| `app_running:<app>` | `{plugin:dev.perritodeck.truenas:app_running:jellyfin}` | `on` / `off`, for a toggle to follow |
| `cpu` | `CPU {plugin:dev.perritodeck.truenas:cpu}%` | CPU use, 0–100 |
| `temp` | `{plugin:dev.perritodeck.truenas:temp} °C` | CPU temperature (`–` if TrueNAS can't read one) |

Apps are polled every 15 seconds and pools and alerts every 30 seconds (`pool.query`,
`alert.list`, `app.query`). CPU use and temperature come from the
`reporting.realtime` event TrueNAS pushes every couple of seconds; the plugin passes
them on every 5 seconds. Values show `–` when TrueNAS can't be reached.

## Tested and untested

Tested with unit tests against a fake WebSocket JSON-RPC server built from the
TrueNAS middleware API models (`src/middlewared/middlewared/api/v25_10_*` and newer,
plus the `reporting.realtime` event source): handshake, masking, pings, messages over
64 KiB, login, the persistent connection, reconnecting, job calls that answer late,
and error texts.

Not yet tried against a real TrueNAS. Worth checking first: the `API_KEY_PLAIN` login
on TrueNAS 26 (and whether it's ever disabled in favour of SCRAM), the exact error
texts and permission errors, and that jobs really answer only when done on every
version (the plugin works either way).
