"""The clipboard and the paste shortcut on Windows through user32 and kernel32 (ctypes,
standard library only).

Imports on every platform; only get_text(), set_text() and press() need Windows.
"""

import ctypes
import time

KEYEVENTF_EXTENDEDKEY = 0x0001
KEYEVENTF_KEYUP = 0x0002

# Virtual-key codes (winuser.h). Insert is an extended key.
KEYS = {"ctrl": 0x11, "shift": 0x10, "alt": 0x12, "v": 0x56, "insert": 0x2D}
EXTENDED = {"insert"}

CF_UNICODETEXT = 13
GMEM_MOVEABLE = 0x0002
HWND_MESSAGE = -3  # parent of a message-only window: never shown, gets no broadcasts

# OpenClipboard fails while another program has the clipboard open, which clipboard
# managers and the app being pasted into do for a moment after every change.
OPEN_TRIES = 20
OPEN_WAIT = 0.025  # seconds between tries: half a second in all

# Registered formats that, set to a DWORD 0, keep what's on the clipboard out of
# Windows' clipboard history (Win+V) and cloud clipboard ("Clipboard Formats" in the
# Windows docs). Other programs, Remote Desktop's clipboard sharing included, still
# see the text.
PRIVATE_FORMATS = ("CanIncludeInClipboardHistory", "CanUploadToCloudClipboard")


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


# --- The clipboard ---------------------------------------------------------------

_dlls = []


def dlls():
    """(user32, kernel32) with the clipboard functions' types set. Handles are
    pointer-sized, so they're c_void_p, never the default int."""
    if not _dlls:
        u = ctypes.WinDLL("user32", use_last_error=True)
        k = ctypes.WinDLL("kernel32", use_last_error=True)
        handle, uint = ctypes.c_void_p, ctypes.c_uint
        u.OpenClipboard.argtypes = [handle]
        u.OpenClipboard.restype = ctypes.c_int
        u.CloseClipboard.argtypes = []
        u.CloseClipboard.restype = ctypes.c_int
        u.EmptyClipboard.argtypes = []
        u.EmptyClipboard.restype = ctypes.c_int
        u.GetClipboardData.argtypes = [uint]
        u.GetClipboardData.restype = handle
        u.SetClipboardData.argtypes = [uint, handle]
        u.SetClipboardData.restype = handle
        u.RegisterClipboardFormatW.argtypes = [ctypes.c_wchar_p]
        u.RegisterClipboardFormatW.restype = uint
        u.CreateWindowExW.argtypes = [ctypes.c_uint32, ctypes.c_wchar_p, ctypes.c_wchar_p, ctypes.c_uint32] + [ctypes.c_int] * 4 + [handle] * 4
        u.CreateWindowExW.restype = handle
        u.DestroyWindow.argtypes = [handle]
        u.DestroyWindow.restype = ctypes.c_int
        k.GlobalAlloc.argtypes = [uint, ctypes.c_size_t]
        k.GlobalAlloc.restype = handle
        k.GlobalLock.argtypes = [handle]
        k.GlobalLock.restype = handle
        k.GlobalUnlock.argtypes = [handle]
        k.GlobalUnlock.restype = ctypes.c_int
        k.GlobalSize.argtypes = [handle]
        k.GlobalSize.restype = ctypes.c_size_t
        k.GlobalFree.argtypes = [handle]
        k.GlobalFree.restype = handle
        _dlls.append((u, k))
    return _dlls[0]


class ClipboardBusy(OSError):
    """Another program kept the clipboard open."""


def failed(what):
    return OSError(f"{what} failed (Windows error {ctypes.get_last_error()})")


def open_clipboard(user32, owner, tries=OPEN_TRIES, wait=OPEN_WAIT, sleep=time.sleep):
    for attempt in range(tries):
        if user32.OpenClipboard(owner):
            return
        if attempt + 1 < tries:
            sleep(wait)
    raise ClipboardBusy("another program is using the clipboard; try again")


def get_text():
    """The clipboard's text, or None if it holds none (or a picture, or files)."""
    user32, kernel32 = dlls()
    open_clipboard(user32, None)
    try:
        data = user32.GetClipboardData(CF_UNICODETEXT)
        if not data:
            return None
        pointer = kernel32.GlobalLock(data)
        if not pointer:
            raise failed("GlobalLock")
        try:
            # Never read past the block, even if the text isn't NUL-terminated.
            text = ctypes.wstring_at(pointer, kernel32.GlobalSize(data) // ctypes.sizeof(ctypes.c_wchar))
        finally:
            kernel32.GlobalUnlock(data)
    finally:
        user32.CloseClipboard()
    return text.split("\0", 1)[0] or None


def put(user32, kernel32, format_id, data):
    """Hand `data` (bytes) to the open clipboard as `format_id`; it owns the memory then."""
    memory = kernel32.GlobalAlloc(GMEM_MOVEABLE, len(data))
    if not memory:
        raise failed("GlobalAlloc")
    pointer = kernel32.GlobalLock(memory)
    if not pointer:
        error = failed("GlobalLock")
        kernel32.GlobalFree(memory)
        raise error
    ctypes.memmove(pointer, data, len(data))
    kernel32.GlobalUnlock(memory)
    if not user32.SetClipboardData(format_id, memory):
        error = failed("SetClipboardData")
        kernel32.GlobalFree(memory)
        raise error


def set_text(text, history=True):
    """Put text on the clipboard ("" empties it). With history=False, Windows'
    clipboard history and cloud clipboard leave it out."""
    user32, kernel32 = dlls()
    # The clipboard needs an owner window: with none, the docs say SetClipboardData
    # fails after EmptyClipboard. A message-only window is enough; what it owns stays
    # on the clipboard after it's destroyed, since nothing is rendered on request.
    owner = user32.CreateWindowExW(0, "STATIC", None, 0, 0, 0, 0, 0, HWND_MESSAGE, None, None, None)
    if not owner:
        raise failed("CreateWindowEx")
    try:
        open_clipboard(user32, owner)
        try:
            if not user32.EmptyClipboard():
                raise failed("EmptyClipboard")
            if text:
                put(user32, kernel32, CF_UNICODETEXT, text.encode("utf-16-le") + b"\0\0")
                if not history:
                    for name in PRIVATE_FORMATS:
                        format_id = user32.RegisterClipboardFormatW(name)
                        if format_id:
                            put(user32, kernel32, format_id, (0).to_bytes(4, "little"))
        finally:
            user32.CloseClipboard()
    finally:
        user32.DestroyWindow(owner)
