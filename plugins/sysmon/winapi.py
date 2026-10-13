"""System counters on Windows through kernel32 and iphlpapi (ctypes, standard library
only).

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


class GUID(ctypes.Structure):
    _fields_ = [
        ("Data1", ctypes.c_uint32),
        ("Data2", ctypes.c_uint16),
        ("Data3", ctypes.c_uint16),
        ("Data4", ctypes.c_ubyte * 8),
    ]


class MIB_IF_ROW2(ctypes.Structure):
    """One network interface (netioapi.h). The WCHAR strings are kept as UTF-16 code
    units, so the size is the same with every compiler's wchar_t."""

    _fields_ = [
        ("InterfaceLuid", ctypes.c_uint64),
        ("InterfaceIndex", ctypes.c_uint32),
        ("InterfaceGuid", GUID),
        ("Alias", ctypes.c_uint16 * 257),
        ("Description", ctypes.c_uint16 * 257),
        ("PhysicalAddressLength", ctypes.c_uint32),
        ("PhysicalAddress", ctypes.c_ubyte * 32),
        ("PermanentPhysicalAddress", ctypes.c_ubyte * 32),
        ("Mtu", ctypes.c_uint32),
        ("Type", ctypes.c_uint32),
        ("TunnelType", ctypes.c_uint32),
        ("MediaType", ctypes.c_uint32),
        ("PhysicalMediumType", ctypes.c_uint32),
        ("AccessType", ctypes.c_uint32),
        ("DirectionType", ctypes.c_uint32),
        ("InterfaceAndOperStatusFlags", ctypes.c_ubyte),  # bit fields, see IF_HARDWARE…
        ("OperStatus", ctypes.c_uint32),
        ("AdminStatus", ctypes.c_uint32),
        ("MediaConnectState", ctypes.c_uint32),
        ("NetworkGuid", GUID),
        ("ConnectionType", ctypes.c_uint32),
        ("TransmitLinkSpeed", ctypes.c_uint64),
        ("ReceiveLinkSpeed", ctypes.c_uint64),
        ("InOctets", ctypes.c_uint64),
        ("InUcastPkts", ctypes.c_uint64),
        ("InNUcastPkts", ctypes.c_uint64),
        ("InDiscards", ctypes.c_uint64),
        ("InErrors", ctypes.c_uint64),
        ("InUnknownProtos", ctypes.c_uint64),
        ("InUcastOctets", ctypes.c_uint64),
        ("InMulticastOctets", ctypes.c_uint64),
        ("InBroadcastOctets", ctypes.c_uint64),
        ("OutOctets", ctypes.c_uint64),
        ("OutUcastPkts", ctypes.c_uint64),
        ("OutNUcastPkts", ctypes.c_uint64),
        ("OutDiscards", ctypes.c_uint64),
        ("OutErrors", ctypes.c_uint64),
        ("OutUcastOctets", ctypes.c_uint64),
        ("OutMulticastOctets", ctypes.c_uint64),
        ("OutBroadcastOctets", ctypes.c_uint64),
        ("OutQLen", ctypes.c_uint64),
    ]


class MIB_IF_TABLE2(ctypes.Structure):
    _fields_ = [("NumEntries", ctypes.c_uint32), ("Table", MIB_IF_ROW2 * 1)]  # really NumEntries rows


# InterfaceAndOperStatusFlags
IF_HARDWARE = 0x01  # HardwareInterface
IF_FILTER = 0x02  # FilterInterface: an NDIS filter's view of another row's adapter
IF_CONNECTOR = 0x04  # ConnectorPresent

_kernel32 = []
_iphlpapi = []


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


def iphlpapi():
    if not _iphlpapi:
        i = ctypes.WinDLL("iphlpapi")
        i.GetIfTable2.argtypes = [ctypes.POINTER(ctypes.POINTER(MIB_IF_TABLE2))]
        i.GetIfTable2.restype = ctypes.c_uint32
        i.FreeMibTable.argtypes = [ctypes.c_void_p]
        i.FreeMibTable.restype = None
        _iphlpapi.append(i)
    return _iphlpapi[0]


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


def physical(row):
    """Whether a row is a physical network adapter, the way Get-NetAdapter -Physical
    sees it. The table also lists every adapter once more for each NDIS filter bound
    to it (QoS Packet Scheduler, the WFP layers, Npcap…), with the same counters, and
    virtual adapters (vEthernet, VPNs, loopback) whose traffic also crosses a physical
    one. netstat -e adds up every row, which counts each byte four to six times."""
    flags = row.InterfaceAndOperStatusFlags
    return bool(flags & IF_HARDWARE and flags & IF_CONNECTOR and not flags & IF_FILTER)


def traffic(rows):
    """(received, sent) bytes so far over the physical adapters among `rows`."""
    received = sent = 0
    for row in rows:
        if physical(row):
            received += row.InOctets
            sent += row.OutOctets
    return received, sent


def network_bytes():
    """(received, sent) bytes so far over the physical network adapters, from their
    64-bit counters. (netstat -e adds up 32-bit ones for every row and wraps.)"""
    table = ctypes.POINTER(MIB_IF_TABLE2)()
    error = iphlpapi().GetIfTable2(ctypes.byref(table))
    if error:
        raise OSError(f"GetIfTable2 failed ({error})")
    try:
        first = ctypes.addressof(table.contents) + MIB_IF_TABLE2.Table.offset
        return traffic((MIB_IF_ROW2 * table.contents.NumEntries).from_address(first))
    finally:
        iphlpapi().FreeMibTable(table)
