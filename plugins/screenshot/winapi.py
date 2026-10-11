"""The Windows Pictures folder through shell32 (ctypes, standard library only).

Imports on every platform; only pictures_folder() needs Windows.
"""

import ctypes


class GUID(ctypes.Structure):
    _fields_ = [
        ("Data1", ctypes.c_uint32),
        ("Data2", ctypes.c_uint16),
        ("Data3", ctypes.c_uint16),
        ("Data4", ctypes.c_ubyte * 8),
    ]


# FOLDERID_Pictures {33E28130-4E1E-4676-835A-98395C3BC3BB} (KnownFolders.h)
FOLDERID_PICTURES = GUID(0x33E28130, 0x4E1E, 0x4676, (ctypes.c_ubyte * 8)(0x83, 0x5A, 0x98, 0x39, 0x5C, 0x3B, 0xC3, 0xBB))


def pictures_folder():
    """The user's Pictures folder (it may have moved to OneDrive), or None."""
    shell32 = ctypes.WinDLL("shell32")
    ole32 = ctypes.WinDLL("ole32")
    path = ctypes.c_wchar_p()
    shell32.SHGetKnownFolderPath.argtypes = [ctypes.POINTER(GUID), ctypes.c_uint32, ctypes.c_void_p, ctypes.POINTER(ctypes.c_wchar_p)]
    shell32.SHGetKnownFolderPath.restype = ctypes.c_long
    if shell32.SHGetKnownFolderPath(ctypes.byref(FOLDERID_PICTURES), 0, None, ctypes.byref(path)) != 0:
        return None
    try:
        return path.value
    finally:
        ole32.CoTaskMemFree.argtypes = [ctypes.c_void_p]
        ole32.CoTaskMemFree(ctypes.cast(path, ctypes.c_void_p))
