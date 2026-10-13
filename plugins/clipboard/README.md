# Clipboard snippets

Paste a saved piece of text with one press, and keep numbered slots: save whatever is on
the clipboard into a slot, and paste it back later, even after a restart.

## Paste text or type text?

PerritoDeck also has a built-in **Type text** action. Which to use:

| | Paste text (this plugin) | Type text (built in) |
|---|---|---|
| Speed | instant, however long | types one character at a time |
| Emoji, accents, other alphabets | always right | can go wrong on some keyboard layouts and apps |
| The clipboard | used for a moment, then put back | not touched |
| Works where pasting is blocked (some password fields, remote desktops, games) | no | yes |

## Setup

Nothing to fill in. The Plugins page has one setting: **Wait before putting the
clipboard back** (0.5 s by default). Raise it if an app sometimes pastes your old
clipboard instead of the snippet (slow apps, remote desktops).

Per system:

- **Windows**: nothing to install. The clipboard is used in-process through Windows'
  own functions (a few milliseconds); Windows PowerShell 5.1 is the fallback if they
  fail.
- **macOS**: pasting presses ⌘V for you, which needs **System Settings → Privacy &
  Security → Accessibility** for PerritoDeck (or the app that started the host).
- **Linux**: `wl-clipboard` on Wayland, else `xclip` or `xsel`; to press the paste
  shortcut, `wtype` or `ydotool` on Wayland, `xdotool` on X11.

## Actions

| Action | Fields |
|---|---|
| Paste text | Text; put back what was on the clipboard afterwards (on by default); paste with Ctrl+V, Ctrl+Shift+V (terminals) or Shift+Insert. On a Mac it's always ⌘V. |
| Copy text | Text |
| Save the clipboard to a slot | Slot number |
| Paste a slot | Slot number; put back…; paste with… |
| Copy a slot | Slot number |

The text field is one line. For text with several lines, copy it, save it to a slot, and
paste the slot. Slots are kept in the plugin's own data folder (`slots.json`).

Only text is handled: if the clipboard held a picture or files when you pasted, they
can't be put back. If you copy something new before the old clipboard is put back,
your new copy is kept. On Windows, a snippet that's pasted and then replaced by your old
clipboard is kept out of clipboard history (Win+V) and the cloud clipboard, and so is
the old text when it's put back (it's in the history already). Copy text, and pasting
with "put back" off, count as copying: those show up in the history as usual.

## Values

| Title | Shows |
|---|---|
| `{plugin:dev.perritodeck.clipboard:slot:1}` | the first 20 characters of slot 1, on one line; `–` when empty |

## Verified vs untested

- **Verified** on Linux X11 (Xvfb, `xclip`, `xdotool`): copy text with accents and ✓,
  save to a slot, paste a slot and put the old clipboard back, over the real plugin
  protocol. That the paste shortcut landed in an app was not checked (only that
  `xdotool` accepted it).
- **Tested on Windows 11** without touching your clipboard: the test suite runs the
  clipboard functions in a process moved to a window station of its own, which has a
  separate clipboard (text with emoji and line breaks, emptying, the clipboard-history
  formats, another program holding the clipboard open). Copy and paste in the app and
  the Win+V history were not checked by hand yet, and the key presses aren't run.
- **Unit-tested only**: the Windows PowerShell fallback, macOS
  (`pbcopy`/`pbpaste`/`osascript` arguments), Wayland (`wl-copy`, `wl-paste`, `wtype`,
  `ydotool` arguments) and `xsel`.
