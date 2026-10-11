"""The paste shortcut on Windows through user32 keybd_event (ctypes, standard library only).

Imports on every platform; only press() needs Windows.
"""

import ctypes

KEYEVENTF_EXTENDEDKEY = 0x0001
KEYEVENTF_KEYUP = 0x0002

# Virtual-key codes (winuser.h). Insert is an extended key.
KEYS = {"ctrl": 0x11, "shift": 0x10, "alt": 0x12, "v": 0x56, "insert": 0x2D}
EXTENDED = {"insert"}


def plan(combo):
    """The key events for a combo like "ctrl+shift+v": [(vk, extended, up), …],
    modifiers down first and up last, in reverse order."""
    names = [k.strip().lower() for k in combo.split("+") if k.strip()]
    for name in names:
        if name not in KEYS:
            raise ValueError(f"unknown key {name}")
    down = [(KEYS[n], n in EXTENDED, False) for n in names]
    up = [(KEYS[n], n in EXTENDED, True) for n in reversed(names)]
    return down + up


def press(combo):
    user32 = ctypes.WinDLL("user32")
    user32.keybd_event.argtypes = [ctypes.c_ubyte, ctypes.c_ubyte, ctypes.c_uint32, ctypes.c_size_t]
    user32.keybd_event.restype = None
    for vk, extended, up in plan(combo):
        flags = (KEYEVENTF_KEYUP if up else 0) | (KEYEVENTF_EXTENDEDKEY if extended else 0)
        user32.keybd_event(vk, 0, flags, 0)
