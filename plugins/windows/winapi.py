"""Windows window management through user32/kernel32 (ctypes, standard library only).

Nothing here touches Windows until a function is called, so the module imports on
macOS and Linux too (the tests check that). Structures use fixed-size ctypes types
(c_uint32 and so on) rather than ctypes.wintypes, whose DWORD is 8 bytes on Linux,
so their sizes can be checked anywhere.

Functions return plain tuples (hwnd, pid, exe name, title); plugin.py does the rest.
"""

import ctypes
import os

# Window messages, ShowWindow commands and styles (winuser.h).
WM_CLOSE = 0x0010
SW_MAXIMIZE = 3
SW_MINIMIZE = 6
SW_RESTORE = 9
GW_OWNER = 4
GWL_EXSTYLE = -20
WS_EX_TOOLWINDOW = 0x00000080
DWMWA_CLOAKED = 14
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
TH32CS_SNAPPROCESS = 0x00000002
KEYEVENTF_EXTENDEDKEY = 0x0001
KEYEVENTF_KEYUP = 0x0002
VK_MENU = 0x12  # Alt
VK_LWIN = 0x5B
VK_D = 0x44
MAX_PATH = 260


class PROCESSENTRY32W(ctypes.Structure):
    _fields_ = [
        ("dwSize", ctypes.c_uint32),
        ("cntUsage", ctypes.c_uint32),
        ("th32ProcessID", ctypes.c_uint32),
        ("th32DefaultHeapID", ctypes.c_size_t),  # ULONG_PTR
        ("th32ModuleID", ctypes.c_uint32),
        ("cntThreads", ctypes.c_uint32),
        ("th32ParentProcessID", ctypes.c_uint32),
        ("pcPriClassBase", ctypes.c_int32),
        ("dwFlags", ctypes.c_uint32),
        ("szExeFile", ctypes.c_wchar * MAX_PATH),
    ]


_dll = {}


def _libs():
    """user32, kernel32 and dwmapi (or None), with argument types set once."""
    if _dll:
        return _dll["user32"], _dll["kernel32"], _dll["dwmapi"]
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    try:
        dwmapi = ctypes.WinDLL("dwmapi")
    except OSError:
        dwmapi = None
    hwnd = ctypes.c_void_p
    handle = ctypes.c_void_p
    for name, args, res in (
        ("GetForegroundWindow", [], hwnd),
        ("SetForegroundWindow", [hwnd], ctypes.c_int),
        ("BringWindowToTop", [hwnd], ctypes.c_int),
        ("ShowWindow", [hwnd, ctypes.c_int], ctypes.c_int),
        ("IsIconic", [hwnd], ctypes.c_int),
        ("IsWindowVisible", [hwnd], ctypes.c_int),
        ("IsWindow", [hwnd], ctypes.c_int),
        ("GetWindow", [hwnd, ctypes.c_uint], hwnd),
        ("GetWindowLongW", [hwnd, ctypes.c_int], ctypes.c_long),
        ("GetWindowTextLengthW", [hwnd], ctypes.c_int),
        ("GetWindowTextW", [hwnd, ctypes.c_wchar_p, ctypes.c_int], ctypes.c_int),
        ("GetWindowThreadProcessId", [hwnd, ctypes.POINTER(ctypes.c_uint32)], ctypes.c_uint32),
        ("AttachThreadInput", [ctypes.c_uint32, ctypes.c_uint32, ctypes.c_int], ctypes.c_int),
        ("PostMessageW", [hwnd, ctypes.c_uint, ctypes.c_size_t, ctypes.c_ssize_t], ctypes.c_int),
        ("keybd_event", [ctypes.c_ubyte, ctypes.c_ubyte, ctypes.c_uint32, ctypes.c_size_t], None),
    ):
        fn = getattr(user32, name)
        fn.argtypes, fn.restype = args, res
    for name, args, res in (
        ("GetCurrentThreadId", [], ctypes.c_uint32),
        ("OpenProcess", [ctypes.c_uint32, ctypes.c_int, ctypes.c_uint32], handle),
        ("CloseHandle", [handle], ctypes.c_int),
        ("QueryFullProcessImageNameW", [handle, ctypes.c_uint32, ctypes.c_wchar_p, ctypes.POINTER(ctypes.c_uint32)], ctypes.c_int),
        ("CreateToolhelp32Snapshot", [ctypes.c_uint32, ctypes.c_uint32], handle),
        ("Process32FirstW", [handle, ctypes.POINTER(PROCESSENTRY32W)], ctypes.c_int),
        ("Process32NextW", [handle, ctypes.POINTER(PROCESSENTRY32W)], ctypes.c_int),
    ):
        fn = getattr(kernel32, name)
        fn.argtypes, fn.restype = args, res
    if dwmapi is not None:
        dwmapi.DwmGetWindowAttribute.argtypes = [hwnd, ctypes.c_uint32, ctypes.c_void_p, ctypes.c_uint32]
        dwmapi.DwmGetWindowAttribute.restype = ctypes.c_long
    _dll.update(user32=user32, kernel32=kernel32, dwmapi=dwmapi)
    return user32, kernel32, dwmapi


def _exe_name(pid):
    """The process's executable file name ("firefox.exe"), or "" if Windows won't say."""
    _, kernel32, _ = _libs()
    h = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
    if not h:
        return ""
    try:
        size = ctypes.c_uint32(1024)
        buf = ctypes.create_unicode_buffer(size.value)
        if kernel32.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(size)):
            return os.path.basename(buf.value)
        return ""
    finally:
        kernel32.CloseHandle(h)


def _title(hwnd):
    user32, _, _ = _libs()
    n = user32.GetWindowTextLengthW(hwnd)
    if n <= 0:
        return ""
    buf = ctypes.create_unicode_buffer(n + 1)
    user32.GetWindowTextW(hwnd, buf, n + 1)
    return buf.value


def _pid(hwnd):
    user32, _, _ = _libs()
    pid = ctypes.c_uint32(0)
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    return pid.value


def _cloaked(hwnd):
    """True for windows DWM hides (suspended Store apps, other virtual desktops)."""
    _, _, dwmapi = _libs()
    if dwmapi is None:
        return False
    value = ctypes.c_int(0)
    if dwmapi.DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, ctypes.byref(value), ctypes.sizeof(value)) != 0:
        return False
    return value.value != 0


def info(hwnd):
    """(hwnd, pid, exe name, title) for one window."""
    pid = _pid(hwnd)
    return hwnd, pid, _exe_name(pid), _title(hwnd)


def list_windows():
    """The windows Alt+Tab would show, front to back, as (hwnd, pid, exe, title)."""
    user32, _, _ = _libs()
    found = []
    callback_type = ctypes.WINFUNCTYPE(ctypes.c_int, ctypes.c_void_p, ctypes.c_void_p)

    def visit(hwnd, _lparam):
        if not hwnd or not user32.IsWindowVisible(hwnd) or user32.GetWindowTextLengthW(hwnd) <= 0:
            return 1
        if user32.GetWindow(hwnd, GW_OWNER):
            return 1  # dialogs and other owned windows
        if user32.GetWindowLongW(hwnd, GWL_EXSTYLE) & WS_EX_TOOLWINDOW:
            return 1
        if _cloaked(hwnd):
            return 1
        found.append(hwnd)
        return 1

    user32.EnumWindows.argtypes = [callback_type, ctypes.c_void_p]
    user32.EnumWindows.restype = ctypes.c_int
    user32.EnumWindows(callback_type(visit), None)
    names = {}
    out = []
    for hwnd in found:
        pid = _pid(hwnd)
        if pid not in names:
            names[pid] = _exe_name(pid)
        out.append((hwnd, pid, names[pid], _title(hwnd)))
    return out


def foreground():
    """The foreground window's handle, or None."""
    user32, _, _ = _libs()
    return user32.GetForegroundWindow() or None


def process_names():
    """Executable names of every running process, lower case ("firefox.exe")."""
    _, kernel32, _ = _libs()
    snap = kernel32.CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
    if not snap or snap == ctypes.c_void_p(-1).value:
        raise OSError("couldn't list processes")
    names = set()
    try:
        entry = PROCESSENTRY32W()
        entry.dwSize = ctypes.sizeof(PROCESSENTRY32W)
        ok = kernel32.Process32FirstW(snap, ctypes.byref(entry))
        while ok:
            names.add(entry.szExeFile.lower())
            ok = kernel32.Process32NextW(snap, ctypes.byref(entry))
    finally:
        kernel32.CloseHandle(snap)
    return names


def _key(vk, up=False, extended=False):
    user32, _, _ = _libs()
    flags = (KEYEVENTF_KEYUP if up else 0) | (KEYEVENTF_EXTENDEDKEY if extended else 0)
    user32.keybd_event(vk, 0, flags, 0)


def focus(hwnd):
    """Bring a window to the front; True if it got there.

    Windows only lets the process that received the last input change the
    foreground window. Two well-known ways round that: attach our input queue to the
    foreground window's thread, and send a synthetic Alt press first so we count as
    the last input source."""
    user32, kernel32, _ = _libs()
    if user32.IsIconic(hwnd):
        user32.ShowWindow(hwnd, SW_RESTORE)
    current = foreground()
    if current == hwnd:
        return True
    me = kernel32.GetCurrentThreadId()
    other = user32.GetWindowThreadProcessId(current, None) if current else 0
    attached = bool(other and other != me and user32.AttachThreadInput(me, other, True))
    try:
        _key(VK_MENU, extended=True)
        _key(VK_MENU, up=True, extended=True)
        user32.BringWindowToTop(hwnd)
        user32.SetForegroundWindow(hwnd)
    finally:
        if attached:
            user32.AttachThreadInput(me, other, False)
    return foreground() == hwnd


def show(hwnd, command):
    """ShowWindow with SW_MINIMIZE, SW_MAXIMIZE or SW_RESTORE."""
    user32, _, _ = _libs()
    user32.ShowWindow(hwnd, command)


def close(hwnd):
    """Ask a window to close, as its X button does (the app may ask to save)."""
    user32, _, _ = _libs()
    if not user32.PostMessageW(hwnd, WM_CLOSE, 0, 0):
        raise OSError(f"Windows refused to close the window (error {ctypes.get_last_error()})")


def show_desktop():
    """Win+D: shows the desktop, or brings the windows back if it's showing."""
    _key(VK_LWIN, extended=True)
    _key(VK_D)
    _key(VK_D, up=True)
    _key(VK_LWIN, up=True, extended=True)
