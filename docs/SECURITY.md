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
| Plugins doing more than they claim | Plugins run as the user, with no sandbox; the app says so when adding them. Their settings (tokens) are stored owner-only (`plugin-settings/`, `sdplugins/settings.json`) and never logged by first-party plugins | Declared permissions shown at install; signing |
| Tampered plugin download | Online catalogue: HTTPS only, size and file-count caps, zip paths checked, SHA-256 must match the index, plugin id must match its entry. The hash comes from the same server as the zip, so it guards against corruption, not a compromised perritodeck.dev | Signed index |
| Web page or other local program poses as a Stream Deck plugin | The plugin WebSocket listens on 127.0.0.1 only and refuses upgrades with an `Origin` header (browsers always send one). Another program running as the same user could still connect while a plugin isn't | Per-launch tokens |
| Plugin images (SVG) reading local files | SVG `<image>` may only use `data:` URLs; images are scaled to at most 288 px | — |
| Rogue or spoofed Companion on the LAN | Lines capped at 4 MB; silent connections dropped after 10 s; Companion only draws keys and receives presses of tablets showing a Companion profile | — |
| AI assistant (MCP) or another local program drives the host | Off by default. Listens on 127.0.0.1 only (refuses other addresses); every request needs a 256-bit bearer token (constant-time compare) kept in an owner-only `mcp-token` file; `Host` must be the loopback address and port and any `Origin` a localhost page (DNS rebinding); bodies capped at 1 MB; tool calls rate-limited. Pressing, editing and running are separate permissions, all off by default; keys that run programs or plugins need `run`. **Read tools are always on while it runs and expose key settings** (commands, URLs, text a key types), so don't keep secrets in key actions | Per-assistant tokens; approve-on-host prompt |
| Secrets in logs and bug reports | Log files owner-only; every logged line is redacted (bearer tokens, URL passwords, `token=`/`password:`-style values, long hex, JWTs); diagnostics leave out keys, certificates, tokens and integration addresses | — |
| Command injection via action settings | Command and launch actions take an argv array and never run through a shell; URLs limited to http/https/mailto | Confirm new commands in the UI (Phase 3) |

## Review checklist (every PR touching host networking or actions)

- [ ] No new listener binds beyond loopback without authentication.
- [ ] Secrets compared in constant time; never logged.
- [ ] Every client-supplied size/index is bounds-checked.
- [ ] No shell interpolation of user or plugin data.
- [ ] New protocol messages are in the schema with `additionalProperties: false`.

## Reporting

Until the project has a security contact address, report vulnerabilities privately through GitHub's "Report a vulnerability" button on the repository.
