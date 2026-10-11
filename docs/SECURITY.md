# Security

PerritoDeck's host can type keystrokes and (from Phase 2) run commands on your computer, so anything that can talk to it is effectively a keyboard. This file is the threat model and review checklist; it grows with each phase.

## Threat model

| Threat | Phase 1 | Planned |
|---|---|---|
| Someone on the same Wi-Fi drives your computer | Host binds to loopback by default; LAN mode requires a random 20-char session token, compared in constant time | QR pairing, per-device keys (Phase 2) |
| Traffic sniffed on Wi-Fi | **Not protected** — `ws://` cleartext. Only key indexes and titles travel today | TLS with the host certificate pinned at pairing (Phase 2) |
| Malicious web page in your browser connects to the host | Possible on loopback in Phase 1, but no actions execute yet | Check `Origin`, require pairing for every connection (Phase 2) |
| Oversized or malformed frames | 64 KiB message limit; malformed frames rejected with `badMessage`; handshake timeout; idle timeout | Rate limiting |
| Plugins doing more than they claim | n/a | Declared permissions shown at install (Phase 5) |
| Command injection via action settings | n/a | Actions take argv arrays, never shell strings (Phase 2) |

## Review checklist (every PR touching host networking or actions)

- [ ] No new listener binds beyond loopback without authentication.
- [ ] Secrets compared in constant time; never logged.
- [ ] Every client-supplied size/index is bounds-checked.
- [ ] No shell interpolation of user or plugin data.
- [ ] New protocol messages are in the schema with `additionalProperties: false`.

## Reporting

Until the project has a security contact address, report vulnerabilities privately through GitHub's "Report a vulnerability" button on the repository.
