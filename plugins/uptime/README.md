# Is it up?

Shows whether your servers and web services answer, checked from the computer running
PerritoDeck. Good for a homelab wall: a key per service that turns red when it's down.

## Targets

The target goes after the value id in a title, e.g. `{plugin:dev.perritodeck.uptime:up:nas.local:22}`.

| Target | Check |
|---|---|
| `https://nas.local:5001/` or `http://…` | HTTP GET. Up when the answer is 2xx or 3xx; redirects aren't followed. A page that asks for a login with 401/403 counts as down, so point it at one that answers without logging in (a health or status page). |
| `nas.local:22`, `192.168.1.10:8006`, `[fd00::5]:22` | Opens a TCP connection to that port. |
| `nas.local` | TCP port 443, then port 80 if 443 doesn't answer. |

There's no ping: ICMP needs administrator rights on most systems. Each check gives up
after 3 seconds; all targets are checked at the same time.

## Setup (Plugins page)

- **Check every (seconds)**: 10 by default, 2 at the least.
- **Skip certificate check**: turn on for services with self-signed certificates
  (Proxmox, a NAS, a router). Off by default; without it such a service shows as down and
  the log says why.

## Actions

| Action | Fields |
|---|---|
| Check now | target (optional; empty checks every target your keys show). Warns on the key with what is down. |

## Values

| Title | Shows |
|---|---|
| `{plugin:dev.perritodeck.uptime:up:https://nas.local:5001}` | `on`/`off`, for a toggle to follow |
| `{plugin:dev.perritodeck.uptime:ms:nas.local:22}` | `12 ms`, `–` when down |
| `{plugin:dev.perritodeck.uptime:status:https://nas.local:5001}` | the HTTP code (`200`, `503`), or `up`/`down` |

## Not tested yet

Real services, IPv6 targets and slow DNS: the tests use servers on 127.0.0.1 (HTTP,
HTTPS with a throwaway self-signed certificate, and plain TCP).
