"""Windows Core Audio through ctypes: volume, mute, default devices and app volume.

Standard library only, no comtypes. Every COM call goes through ComPtr.call with a
method spec from the tables below: (vtable index, argument types after `this`). The
indexes count the IUnknown methods (QueryInterface 0, AddRef 1, Release 2) and then
each interface's methods in the order its header (mmdeviceapi.h, endpointvolume.h,
audiopolicy.h, propsys.h) declares them, base interface first.

IPolicyConfig (default device switching) is undocumented: Microsoft never published
it, but it has kept the same GUID and layout since Windows 7 and is what the sound
control panel itself uses. Every other interface here is documented.

The module imports on any OS, so its pure parts can be tested anywhere; the calls
that touch Windows load ole32/kernel32 only when first used.
"""

import ctypes
import re
import sys
import threading
from ctypes import POINTER, byref, c_float, c_int, c_long, c_uint, c_ushort, c_void_p, c_wchar_p

# --- Types --------------------------------------------------------------------


class GUID(ctypes.Structure):
    _fields_ = [("Data1", ctypes.c_uint32), ("Data2", c_ushort), ("Data3", c_ushort), ("Data4", ctypes.c_ubyte * 8)]

    def __str__(self):
        d4 = bytes(self.Data4)
        return "{%08X-%04X-%04X-%s-%s}" % (self.Data1, self.Data2, self.Data3, d4[:2].hex().upper(), d4[2:].hex().upper())


_GUID_RE = re.compile(r"^\{?([0-9a-fA-F]{8})-([0-9a-fA-F]{4})-([0-9a-fA-F]{4})-([0-9a-fA-F]{4})-([0-9a-fA-F]{12})\}?$")


def guid(text):
    """GUID from "{BCDE0395-E52F-467C-8E3D-C4579291692E}" (braces optional)."""
    m = _GUID_RE.match(text.strip())
    if not m:
        raise ValueError(f"not a GUID: {text!r}")
    d4 = bytes.fromhex(m.group(4) + m.group(5))
    return GUID(int(m.group(1), 16), int(m.group(2), 16), int(m.group(3), 16), (ctypes.c_ubyte * 8)(*d4))


class PROPERTYKEY(ctypes.Structure):
    _fields_ = [("fmtid", GUID), ("pid", ctypes.c_uint32)]


class PROPVARIANT(ctypes.Structure):
    # vt, three reserved words, then a union; 16 bytes covers every member on 32-
    # and 64-bit Windows (the biggest are two pointers or a DECIMAL's tail).
    _fields_ = [("vt", c_ushort), ("r1", c_ushort), ("r2", c_ushort), ("r3", c_ushort),
                ("pwszVal", c_void_p), ("pad", c_void_p)]


VT_LPWSTR = 31

# --- GUIDs --------------------------------------------------------------------

CLSID_MMDeviceEnumerator = "{BCDE0395-E52F-467C-8E3D-C4579291692E}"
IID_IMMDeviceEnumerator = "{A95664D2-9614-4F35-A746-DE8DB63617E6}"
IID_IAudioEndpointVolume = "{5CDF2C82-841E-4546-9722-0CF74078229A}"
IID_IAudioSessionManager2 = "{77AA99A0-1BD6-484F-8BC7-2C654C9A9B6F}"
IID_IAudioSessionControl2 = "{BFB7FF88-7239-4FC9-8FA2-07C950BE9C6D}"
IID_ISimpleAudioVolume = "{87CE5498-68D6-44E5-9215-6DA47EF883D8}"
CLSID_PolicyConfigClient = "{870AF99C-171D-4F9E-AF0D-E63DF40C2BC9}"
IID_IPolicyConfig = "{F8679F50-850A-41CF-9C72-430F290290C8}"  # undocumented, Windows 7+
PKEY_Device_FriendlyName = ("{A45C254E-DF1C-4EFD-8020-67D146A850E0}", 14)

# --- Constants ----------------------------------------------------------------

eRender, eCapture = 0, 1
eConsole, eMultimedia, eCommunications = 0, 1, 2
DEVICE_STATE_ACTIVE = 0x1
CLSCTX_ALL = 0x17  # INPROC_SERVER | INPROC_HANDLER | LOCAL_SERVER | REMOTE_SERVER
COINIT_MULTITHREADED = 0x0
STGM_READ = 0x0
S_OK, S_FALSE = 0, 1
RPC_E_CHANGED_MODE = -2147417850  # 0x80010106: the thread already chose apartment threading
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000

# --- Method specs: (vtable index, argument types after `this`) -----------------

PPV = POINTER(c_void_p)

IUnknown_QueryInterface = (0, (POINTER(GUID), PPV))
IUnknown_Release = (2, ())

IMMDeviceEnumerator_EnumAudioEndpoints = (3, (c_int, c_uint, PPV))
IMMDeviceEnumerator_GetDefaultAudioEndpoint = (4, (c_int, c_int, PPV))

IMMDeviceCollection_GetCount = (3, (POINTER(c_uint),))
IMMDeviceCollection_Item = (4, (c_uint, PPV))

IMMDevice_Activate = (3, (POINTER(GUID), c_uint, c_void_p, PPV))
IMMDevice_OpenPropertyStore = (4, (c_uint, PPV))
IMMDevice_GetId = (5, (POINTER(c_void_p),))

IPropertyStore_GetValue = (5, (POINTER(PROPERTYKEY), POINTER(PROPVARIANT)))

# IAudioEndpointVolume: 3 Register…Notify, 4 Unregister…Notify, 5 GetChannelCount,
# 6 SetMasterVolumeLevel, 7 SetMasterVolumeLevelScalar, 8 GetMasterVolumeLevel,
# 9 GetMasterVolumeLevelScalar, 10–13 channel volumes, 14 SetMute, 15 GetMute, …
IAudioEndpointVolume_SetMasterVolumeLevelScalar = (7, (c_float, POINTER(GUID)))
IAudioEndpointVolume_GetMasterVolumeLevelScalar = (9, (POINTER(c_float),))
IAudioEndpointVolume_SetMute = (14, (c_int, POINTER(GUID)))
IAudioEndpointVolume_GetMute = (15, (POINTER(c_int),))

# IAudioSessionManager: 3 GetAudioSessionControl, 4 GetSimpleAudioVolume;
# IAudioSessionManager2 adds 5 GetSessionEnumerator, …
IAudioSessionManager2_GetSessionEnumerator = (5, (PPV,))
IAudioSessionEnumerator_GetCount = (3, (POINTER(c_int),))
IAudioSessionEnumerator_GetSession = (4, (c_int, PPV))

# IAudioSessionControl: 3 GetState … 11 UnregisterAudioSessionNotification;
# IAudioSessionControl2 adds 12 GetSessionIdentifier, 13 GetSessionInstanceIdentifier,
# 14 GetProcessId, 15 IsSystemSoundsSession, 16 SetDuckingPreference.
IAudioSessionControl2_GetProcessId = (14, (POINTER(ctypes.c_uint32),))

ISimpleAudioVolume_SetMasterVolume = (3, (c_float, POINTER(GUID)))
ISimpleAudioVolume_GetMasterVolume = (4, (POINTER(c_float),))
ISimpleAudioVolume_SetMute = (5, (c_int, POINTER(GUID)))
ISimpleAudioVolume_GetMute = (6, (POINTER(c_int),))

# IPolicyConfig: 3 GetMixFormat, 4 GetDeviceFormat, 5 ResetDeviceFormat,
# 6 SetDeviceFormat, 7 GetProcessingPeriod, 8 SetProcessingPeriod, 9 GetShareMode,
# 10 SetShareMode, 11 GetPropertyValue, 12 SetPropertyValue, 13 SetDefaultEndpoint,
# 14 SetEndpointVisibility.
IPolicyConfig_SetDefaultEndpoint = (13, (c_wchar_p, c_int))

# --- Calling COM --------------------------------------------------------------

# COM methods are stdcall on 32-bit Windows; elsewhere (and in tests on Linux) the
# platform's one calling convention is the right one.
_PROTOTYPE = getattr(ctypes, "WINFUNCTYPE", ctypes.CFUNCTYPE)
_prototypes = {}


def prototype(argtypes):
    key = tuple(argtypes)
    if key not in _prototypes:
        _prototypes[key] = _PROTOTYPE(c_long, c_void_p, *argtypes)
    return _prototypes[key]


class ComError(OSError):
    def __init__(self, what, hresult):
        self.hresult = hresult & 0xFFFFFFFF
        super().__init__(f"{what} failed (0x{self.hresult:08X})")


class ComPtr:
    """An interface pointer; released when the `with` block ends."""

    def __init__(self, address, name="COM object"):
        self.ptr = c_void_p(address)
        self.name = name

    def call(self, spec, *args, what=None):
        index, argtypes = spec
        vtable = ctypes.cast(self.ptr, POINTER(POINTER(c_void_p))).contents
        fn = prototype(argtypes)(vtable[index])
        hr = fn(self.ptr, *args)
        if hr < 0:
            raise ComError(what or f"{self.name} method {index}", hr)
        return hr

    def out(self, spec, *args, name="COM object", what=None):
        """Call a method whose last argument returns an interface; wrap it."""
        result = c_void_p()
        self.call(spec, *args, byref(result), what=what)
        if not result.value:
            raise ComError(what or f"{self.name} method {spec[0]}", 0x80004003)  # E_POINTER
        return ComPtr(result.value, name)

    def query(self, iid, name):
        return self.out(IUnknown_QueryInterface, byref(guid(iid)), name=name, what=f"asking for {name}")

    def release(self):
        if self.ptr.value:
            self.call(IUnknown_Release)
            self.ptr = c_void_p()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.release()


_dlls = {}


def _dll(name):
    if name not in _dlls:
        if sys.platform != "win32":
            raise OSError("Core Audio is only on Windows")
        _dlls[name] = ctypes.WinDLL(name)
    return _dlls[name]


class _Apartment:
    """CoInitializeEx for this thread for the length of a `with` block."""

    def __enter__(self):
        ole32 = _dll("ole32")
        hr = ole32.CoInitializeEx(None, COINIT_MULTITHREADED)
        self.uninit = hr in (S_OK, S_FALSE)
        if hr < 0 and hr != RPC_E_CHANGED_MODE:
            raise ComError("starting COM", hr)
        return self

    def __exit__(self, *exc):
        if self.uninit:
            _dll("ole32").CoUninitialize()


def _create(clsid, iid, name):
    result = c_void_p()
    hr = _dll("ole32").CoCreateInstance(byref(guid(clsid)), None, CLSCTX_ALL, byref(guid(iid)), byref(result))
    if hr < 0:
        raise ComError(f"creating {name}", hr)
    return ComPtr(result.value, name)


# Tests replace these to hand in fake objects; nothing else needs to change.
apartment = _Apartment


def create_enumerator():
    return _create(CLSID_MMDeviceEnumerator, IID_IMMDeviceEnumerator, "IMMDeviceEnumerator")


def create_policy_config():
    return _create(CLSID_PolicyConfigClient, IID_IPolicyConfig, "IPolicyConfig")


def free_string(address):
    _dll("ole32").CoTaskMemFree(c_void_p(address))


def clear_propvariant(pv):
    _dll("ole32").PropVariantClear(byref(pv))


def process_name(pid):
    """The executable's file name for a process id ("Spotify.exe"), or ""."""
    kernel32 = _dll("kernel32")
    kernel32.OpenProcess.restype = c_void_p
    handle = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
    if not handle:
        return ""
    try:
        size = ctypes.c_uint32(32768)
        buffer = ctypes.create_unicode_buffer(size.value)
        if not kernel32.QueryFullProcessImageNameW(c_void_p(handle), 0, buffer, byref(size)):
            return ""
        return buffer.value.replace("/", "\\").rsplit("\\", 1)[-1]
    finally:
        kernel32.CloseHandle(c_void_p(handle))


# --- Devices ------------------------------------------------------------------

_lock = threading.Lock()  # one Core Audio conversation at a time; they're quick


def _flow(kind):
    if kind not in ("output", "input"):
        raise ValueError(f"unknown device kind {kind}")
    return eRender if kind == "output" else eCapture


def _device_id(device):
    address = c_void_p()
    device.call(IMMDevice_GetId, byref(address), what="reading a device id")
    try:
        return ctypes.wstring_at(address.value)
    finally:
        free_string(address.value)


def _device_name(device):
    with device.out(IMMDevice_OpenPropertyStore, STGM_READ, name="IPropertyStore", what="opening device properties") as store:
        key = PROPERTYKEY(guid(PKEY_Device_FriendlyName[0]), PKEY_Device_FriendlyName[1])
        value = PROPVARIANT()
        store.call(IPropertyStore_GetValue, byref(key), byref(value), what="reading a device name")
        try:
            if value.vt == VT_LPWSTR and value.pwszVal:
                return ctypes.wstring_at(value.pwszVal)
            return ""
        finally:
            clear_propvariant(value)


def _default_device(enumerator, kind):
    return enumerator.out(
        IMMDeviceEnumerator_GetDefaultAudioEndpoint, _flow(kind), eMultimedia,
        name="IMMDevice", what=f"finding the default {kind} device",
    )


def _endpoint_volume(device):
    return device.out(
        IMMDevice_Activate, byref(guid(IID_IAudioEndpointVolume)), CLSCTX_ALL, None,
        name="IAudioEndpointVolume", what="opening the device's volume",
    )


def _each_device(enumerator, kind):
    """Yield (id, name, device) for each active device; each device is released after."""
    with enumerator.out(
        IMMDeviceEnumerator_EnumAudioEndpoints, _flow(kind), DEVICE_STATE_ACTIVE,
        name="IMMDeviceCollection", what=f"listing {kind} devices",
    ) as collection:
        count = c_uint()
        collection.call(IMMDeviceCollection_GetCount, byref(count))
        for i in range(count.value):
            with collection.out(IMMDeviceCollection_Item, i, name="IMMDevice") as device:
                yield _device_id(device), _device_name(device), device


def devices(kind):
    """[(id, friendly name)] of the active output or input devices."""
    with _lock, apartment(), create_enumerator() as enumerator:
        return [(dev_id, name) for dev_id, name, _ in _each_device(enumerator, kind)]


def default_device(kind):
    """(id, friendly name) of the default output or input device."""
    with _lock, apartment(), create_enumerator() as enumerator, _default_device(enumerator, kind) as device:
        return _device_id(device), _device_name(device)


def set_default_device(device_id):
    """Make a device the default for every role (console, multimedia, communications)."""
    with _lock, apartment(), create_policy_config() as policy:
        for role in (eConsole, eMultimedia, eCommunications):
            policy.call(IPolicyConfig_SetDefaultEndpoint, device_id, role, what="switching the default device")


# --- Volume and mute of the default devices -----------------------------------


def get_volume(kind="output"):
    """Volume of the default device, 0–100."""
    with _lock, apartment(), create_enumerator() as e, _default_device(e, kind) as d, _endpoint_volume(d) as v:
        level = c_float()
        v.call(IAudioEndpointVolume_GetMasterVolumeLevelScalar, byref(level), what="reading the volume")
        return int(round(level.value * 100))


def set_volume(percent, kind="output"):
    level = max(0.0, min(1.0, percent / 100.0))
    with _lock, apartment(), create_enumerator() as e, _default_device(e, kind) as d, _endpoint_volume(d) as v:
        v.call(IAudioEndpointVolume_SetMasterVolumeLevelScalar, level, None, what="setting the volume")


def get_mute(kind="output"):
    with _lock, apartment(), create_enumerator() as e, _default_device(e, kind) as d, _endpoint_volume(d) as v:
        muted = c_int()
        v.call(IAudioEndpointVolume_GetMute, byref(muted), what="reading mute")
        return bool(muted.value)


def set_mute(muted, kind="output"):
    with _lock, apartment(), create_enumerator() as e, _default_device(e, kind) as d, _endpoint_volume(d) as v:
        v.call(IAudioEndpointVolume_SetMute, 1 if muted else 0, None, what="setting mute")


# --- App volume (audio sessions) ----------------------------------------------


def app_matches(exe, wanted):
    """"spotify", "Spotify.exe" and "SPOTIFY" all match Spotify.exe."""
    exe, wanted = exe.lower(), wanted.strip().lower()
    if not exe or not wanted:
        return False
    strip = lambda name: name[:-4] if name.endswith(".exe") else name  # noqa: E731
    return strip(exe) == strip(wanted)


def _app_volumes(enumerator, app):
    """Yield ISimpleAudioVolume for each session of `app` on any active output."""
    for _, _, device in _each_device(enumerator, "output"):
        with device.out(
            IMMDevice_Activate, byref(guid(IID_IAudioSessionManager2)), CLSCTX_ALL, None,
            name="IAudioSessionManager2", what="opening the app sessions",
        ) as manager, manager.out(IAudioSessionManager2_GetSessionEnumerator, name="IAudioSessionEnumerator") as sessions:
            count = c_int()
            sessions.call(IAudioSessionEnumerator_GetCount, byref(count))
            for i in range(count.value):
                with sessions.out(IAudioSessionEnumerator_GetSession, i, name="IAudioSessionControl") as control:
                    with control.query(IID_IAudioSessionControl2, "IAudioSessionControl2") as control2:
                        pid = ctypes.c_uint32()
                        control2.call(IAudioSessionControl2_GetProcessId, byref(pid))
                    if not pid.value or not app_matches(process_name(pid.value), app):
                        continue  # 0 is the system sounds session
                    with control.query(IID_ISimpleAudioVolume, "ISimpleAudioVolume") as volume:
                        yield volume


def get_app_volume(app):
    """(volume 0–100, muted) of the app's first session, or None if it has none."""
    with _lock, apartment(), create_enumerator() as enumerator:
        for volume in _app_volumes(enumerator, app):
            level, muted = c_float(), c_int()
            volume.call(ISimpleAudioVolume_GetMasterVolume, byref(level))
            volume.call(ISimpleAudioVolume_GetMute, byref(muted))
            return int(round(level.value * 100)), bool(muted.value)
    return None


def set_app_volume(app, percent=None, muted=None):
    """Set the volume and/or mute of every session of an app; returns how many."""
    level = None if percent is None else max(0.0, min(1.0, percent / 100.0))
    changed = 0
    with _lock, apartment(), create_enumerator() as enumerator:
        for volume in _app_volumes(enumerator, app):
            if level is not None:
                volume.call(ISimpleAudioVolume_SetMasterVolume, level, None, what="setting the app's volume")
            if muted is not None:
                volume.call(ISimpleAudioVolume_SetMute, 1 if muted else 0, None, what="muting the app")
            changed += 1
    return changed
