# Vendored enigo

`enigo` 0.6.1 from crates.io (MIT, © the enigo authors), vendored by PerritoDeck with
these local changes:

Wayland input can use the desktop portal without hurting X11 users:

- `src/linux/libei.rs`: `open_connection` returns errors instead of panicking when the
  RemoteDesktop portal is missing or the user declines the prompt.
- `src/linux/mod.rs`: the libei/portal backend is only tried when the session is
  Wayland (`WAYLAND_DISPLAY` set or `XDG_SESSION_TYPE=wayland`); X11 sessions use
  XTest with no prompt.

Windows:

- `src/keycodes.rs` (`Key::Unicode` on Windows): the result of `VkKeyScanExW` is masked
  with `& 0x00FF`. Its low byte is the virtual-key code and its high byte the shift
  state (Microsoft's `VkKeyScanExW` docs); upstream kept the high byte, so
  `MapVirtualKeyExW` found no scan code and keys that need Shift or AltGr on the
  current layout (`ctrl+plus` on US, digits on AZERTY) sent nothing. Upstream `main` has
  the same bug at the time of writing.
- `src/win/win_impl.rs` (`text`): `'\n'` and `'\t'` are typed only as the Return and Tab
  keys. Upstream queued the key and then the character as well, so apps that act on both
  (classic edit controls, Tk) got two line breaks or two tabs. `"\r\n"` is one Return and a
  lone `'\r'` is a Return; upstream sent no key for `'\r'` but a U+000D character, so a
  CRLF line end could come out as three line breaks.

Build:

- `src/lib.rs`: `#![allow(unused, unknown_lints)]` so the vendored code doesn't add warnings to our builds.

Plan: upstream the input changes and drop this copy.
