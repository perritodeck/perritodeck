# Security policy

PerritoDeck's host can type keystrokes and run programs on your computer, so we treat
anything that lets someone else drive it as serious. The threat model and the review
checklist are in [docs/SECURITY.md](docs/SECURITY.md).

## Supported versions

PerritoDeck is pre-1.0. Only the latest release (and `main`) gets security fixes.

## Reporting a vulnerability

Please **don't** open a public issue, discussion or pull request for a security problem.

1. **Preferred:** use GitHub's private vulnerability reporting: the
   [**Report a vulnerability**](https://github.com/perritodeck/perritodeck/security/advisories/new)
   button on the repository's Security tab.
2. **Or email** [dev@perritodeck.dev](mailto:dev@perritodeck.dev) with "SECURITY" in the
   subject.

Include what you found, the versions and OS involved, and steps or a proof of concept.
Strip tokens, pairing secrets and personal paths from any logs you attach.

## What happens next

- We acknowledge your report within **5 days**.
- We confirm the problem, agree a severity with you, and aim to release a fix within
  **90 days** (sooner for anything that lets a device on the network control a computer
  without pairing).
- We publish a GitHub Security Advisory with the fix and credit you, unless you'd rather
  stay anonymous.
- Please give us a chance to ship the fix before you disclose publicly. If we go quiet
  for more than 30 days, you're free to disclose.

## Scope

In scope: the desktop host and app (`host/`), the Android and iOS apps, the wire protocol,
the plugin runtime, the online plugin catalogue, and the first-party plugins.

Out of scope: third-party plugins (report to their authors; plugins run with your user's
permissions by design, see docs/SECURITY.md), problems that need an attacker who already
runs code as your user (such as reading plugin sign-in tokens, device keys or settings
from the data folder), and denial of service by someone who is already paired.
