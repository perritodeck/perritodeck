# Vendored enigo

`enigo` 0.6.1 from crates.io (MIT, © the enigo authors), vendored by PerritoDeck with
one change, so Wayland input can use the desktop portal without hurting X11 users:

- `src/linux/libei.rs`: `open_connection` returns errors instead of panicking when the
  RemoteDesktop portal is missing or the user declines the prompt.
- `src/linux/mod.rs`: the libei/portal backend is only tried when the session is
  Wayland (`WAYLAND_DISPLAY` set or `XDG_SESSION_TYPE=wayland`); X11 sessions use
  XTest with no prompt.

Plan: upstream both changes and drop this copy.
- `src/lib.rs`: `#![allow(unused, unknown_lints)]` so the vendored code doesn't add warnings to our builds.
