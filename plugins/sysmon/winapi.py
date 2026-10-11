"""System counters on Windows through kernel32 (ctypes, standard library only).

Imports on every platform; the functions need Windows. Structures use fixed-size
ctypes types so their sizes can be checked anywhere.
"""

import ctypes


class FILETIME(ctypes.Structure):
    _fields_ = [("dwLowDateTime", ctypes.c_uint32), ("dwHighDateTime", ctypes.c_uint32)]

    def value(self):
        return (self.dwHighDateTime << 32) | self.dwLowDateTime


class MEMORYSTATUSEX(ctypes.Structure):
    _fields_ = [
        ("dwLength", ctypes.c_uint32),
        ("dwMemoryLoad", ctypes.c_uint32),
        ("ullTotalPhys", ctypes.c_uint64),
        ("ullAvailPhys", ctypes.c_uint64),
        ("ullTotalPageFile", ctypes.c_uint64),
        ("ullAvailPageFile", ctypes.c_uint64),
        ("ullTotalVirtual", ctypes.c_uint64),
        ("ullAvailVirtual", ctypes.c_uint64),
        ("ullAvailExtendedVirtual", ctypes.c_uint64),
    ]


class SYSTEM_POWER_STATUS(ctypes.Structure):
    _fields_ = [
        ("ACLineStatus", ctypes.c_ubyte),
        ("BatteryFlag", ctypes.c_ubyte),
        ("BatteryLifePercent", ctypes.c_ubyte),
        ("SystemStatusFlag", ctypes.c_ubyte),
        ("BatteryLifeTime", ctypes.c_uint32),
        ("BatteryFullLifeTime", ctypes.c_uint32),
    ]


BATTERY_FLAG_NO_BATTERY = 128
BATTERY_FLAG_UNKNOWN = 255
BATTERY_PERCENT_UNKNOWN = 255

_kernel32 = []


def kernel32():
    if not _kernel32:
        k = ctypes.WinDLL("kernel32", use_last_error=True)
        k.GetSystemTimes.argtypes = [ctypes.POINTER(FILETIME)] * 3
        k.GetSystemTimes.restype = ctypes.c_int
        k.GlobalMemoryStatusEx.argtypes = [ctypes.POINTER(MEMORYSTATUSEX)]
        k.GlobalMemoryStatusEx.restype = ctypes.c_int
        k.GetSystemPowerStatus.argtypes = [ctypes.POINTER(SYSTEM_POWER_STATUS)]
        k.GetSystemPowerStatus.restype = ctypes.c_int
        k.GetTickCount64.argtypes = []
        k.GetTickCount64.restype = ctypes.c_uint64
        _kernel32.append(k)
    return _kernel32[0]


def system_times():
    """(idle, kernel, user) in 100 ns units since boot; kernel includes idle."""
    idle, kernel, user = FILETIME(), FILETIME(), FILETIME()
    if not kernel32().GetSystemTimes(ctypes.byref(idle), ctypes.byref(kernel), ctypes.byref(user)):
        raise OSError(f"GetSystemTimes failed ({ctypes.get_last_error()})")
    return idle.value(), kernel.value(), user.value()


def memory():
    """(total, available) physical memory in bytes."""
    status = MEMORYSTATUSEX()
    status.dwLength = ctypes.sizeof(MEMORYSTATUSEX)
    if not kernel32().GlobalMemoryStatusEx(ctypes.byref(status)):
        raise OSError(f"GlobalMemoryStatusEx failed ({ctypes.get_last_error()})")
    return status.ullTotalPhys, status.ullAvailPhys


def battery_percent(status=None):
    """Charge in percent, or None without a battery."""
    if status is None:
        status = SYSTEM_POWER_STATUS()
        if not kernel32().GetSystemPowerStatus(ctypes.byref(status)):
            return None
    if status.BatteryFlag in (BATTERY_FLAG_UNKNOWN,) or status.BatteryFlag & BATTERY_FLAG_NO_BATTERY:
        return None
    if status.BatteryLifePercent == BATTERY_PERCENT_UNKNOWN:
        return None
    return status.BatteryLifePercent


def uptime_seconds():
    return kernel32().GetTickCount64() / 1000.0
