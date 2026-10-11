# Security

PerritoDeck's host can type keystrokes and (from Phase 2) run commands on your computer, so anything that can talk to it is effectively a keyboard. This file is the threat model and review checklist; it grows with each phase.

## Threat model

| Threat | Today | Planned |
|---|---|---|
| Someone on the same Wi-Fi drives your computer | Every device must pair with a one-time 6-digit code (5-minute window, single use, burned after 5 wrong guesses). Each device gets its own 256-bit secret; the host stores only its hash; `revoke` disconnects and forgets a device | Approve-on-host prompt for new devices (Phase 3 UI) |
| Traffic sniffed or altered on Wi-Fi | TLS (rustls, TLS 1.2/1.3) with the host's self-signed certificate pinned by SHA-256 on the client | — |
| Someone impersonates the host during pairing | QR link carries the fingerprint; typed codes show a fingerprint to compare by eye | — |
| Malicious web page in your browser connects to the host | WebSocket upgrades with an `Origin` header are refused; pairing is required anyway | — |
| Host key theft from disk | Private key and device list written with owner-only permissions (Unix); Windows relies on the per-user profile ACL | OS keychain storage (Phase 6) |
| Oversized or malformed frames | 64 KiB message limit; malformed frames rejected with `badMessage`; handshake timeout; idle timeout | Rate limiting |
| Plugins doing more than they claim | n/a | Declared permissions shown at install (Phase 5) |
| Command injection via action settings | Command and launch actions take an argv array and never run through a shell; URLs limited to http/https/mailto | Confirm new commands in the UI (Phase 3) |

## Review checklist (every PR touching host networking or actions)

- [ ] No new listener binds beyond loopback without authentication.
- [ ] Secrets compared in constant time; never logged.
- [ ] Every client-supplied size/index is bounds-checked.
- [ ] No shell interpolation of user or plugin data.
- [ ] New protocol messages are in the schema with `additionalProperties: false`.

## Reporting

Until the project has a security contact address, report vulnerabilities privately through GitHub's "Report a vulnerability" button on the repository.
