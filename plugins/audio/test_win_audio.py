"""win_audio can't talk to Windows here, so these tests check its pure parts and
drive its COM plumbing against fake objects: real vtables of ctypes callbacks, laid
out the way COM lays them out, so every method index and argument type is used."""

import ctypes
import os
import sys
import unittest
import uuid
from ctypes import POINTER, c_void_p
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import win_audio as w  # noqa: E402

S_OK = 0
E_FAIL = -2147467259
CALLBACK_ERRORS = []  # ctypes swallows exceptions in callbacks; collect them instead


class FakeCom:
    """A COM object: a pointer to a vtable of callbacks. methods: {spec: fn(*args)}."""

    def __init__(self, name, methods=None):
        self.name = name
        self.released = 0
        self.queried = []
        methods = dict(methods or {})
        methods.setdefault(w.IUnknown_Release, self._release)
        size = max(index for index, _ in methods) + 1
        self._callbacks = []
        self.vtable = (c_void_p * size)()
        for (index, argtypes), fn in methods.items():
            callback = w.prototype(argtypes)(self._wrap(fn))
            self._callbacks.append(callback)  # keep it alive
            self.vtable[index] = ctypes.cast(callback, c_void_p).value
        self.obj = c_void_p(ctypes.addressof(self.vtable))
        self.address = ctypes.addressof(self.obj)

    def _wrap(self, fn):
        def call(this, *args):
            try:
                assert this == self.address, f"{self.name} called with the wrong `this`"
                result = fn(*args)
            except BaseException as e:  # noqa: B902 - reported by the test's cleanup
                CALLBACK_ERRORS.append(f"{self.name}: {e!r}")
                return E_FAIL
            return S_OK if result is None else result
        return call

    def _release(self):
        self.released += 1
        return 0


def hand_out(ppv, fake):
    ppv[0] = fake.address


def endpoint_fakes(level=0.42, muted=1):
    calls = []

    def get_scalar(out):
        out[0] = level

    def get_mute(out):
        out[0] = muted

    endpoint = FakeCom("IAudioEndpointVolume", {
        w.IAudioEndpointVolume_GetMasterVolumeLevelScalar: get_scalar,
        w.IAudioEndpointVolume_SetMasterVolumeLevelScalar: lambda value, ctx: calls.append(("volume", round(value, 3), bool(ctx))),
        w.IAudioEndpointVolume_GetMute: get_mute,
        w.IAudioEndpointVolume_SetMute: lambda value, ctx: calls.append(("mute", value, bool(ctx))),
    })

    def activate(iid, clsctx, params, ppv):
        calls.append(("activate", str(iid.contents), clsctx))
        hand_out(ppv, endpoint)

    device = FakeCom("IMMDevice", {w.IMMDevice_Activate: activate})

    def default(flow, role, ppv):
        calls.append(("default", flow, role))
        hand_out(ppv, device)

    enumerator = FakeCom("IMMDeviceEnumerator", {w.IMMDeviceEnumerator_GetDefaultAudioEndpoint: default})
    return enumerator, device, endpoint, calls


class NoApartment:
    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False


class GuidTest(unittest.TestCase):
    def test_parse_matches_the_windows_byte_layout(self):
        for text in (w.CLSID_MMDeviceEnumerator, w.IID_IMMDeviceEnumerator, w.IID_IAudioEndpointVolume,
                     w.IID_IAudioSessionManager2, w.IID_IAudioSessionControl2, w.IID_ISimpleAudioVolume,
                     w.CLSID_PolicyConfigClient, w.IID_IPolicyConfig, w.PKEY_Device_FriendlyName[0]):
            g = w.guid(text)
            self.assertEqual(ctypes.sizeof(g), 16)
            self.assertEqual(bytes(g), uuid.UUID(text).bytes_le)  # GUIDs are little-endian in memory
            self.assertEqual(str(g), text.upper())

    def test_parse_forms(self):
        self.assertEqual(str(w.guid("bcde0395-e52f-467c-8e3d-c4579291692e")), w.CLSID_MMDeviceEnumerator)
        for bad in ("", "{BCDE0395-E52F-467C-8E3D}", "not-a-guid", "{BCDE0395-E52F-467C-8E3D-C4579291692G}"):
            with self.assertRaises(ValueError):
                w.guid(bad)

    def test_documented_guids(self):
        self.assertEqual(w.CLSID_MMDeviceEnumerator, "{BCDE0395-E52F-467C-8E3D-C4579291692E}")
        self.assertEqual(w.IID_IMMDeviceEnumerator, "{A95664D2-9614-4F35-A746-DE8DB63617E6}")
        self.assertEqual(w.IID_IAudioEndpointVolume, "{5CDF2C82-841E-4546-9722-0CF74078229A}")
        self.assertEqual(w.IID_IAudioSessionManager2, "{77AA99A0-1BD6-484F-8BC7-2C654C9A9B6F}")
        self.assertEqual(w.IID_ISimpleAudioVolume, "{87CE5498-68D6-44E5-9215-6DA47EF883D8}")
        self.assertEqual(w.PKEY_Device_FriendlyName, ("{A45C254E-DF1C-4EFD-8020-67D146A850E0}", 14))

    def test_struct_sizes(self):
        self.assertEqual(ctypes.sizeof(w.PROPERTYKEY), 20)
        self.assertEqual(ctypes.sizeof(w.PROPVARIANT), 8 + 2 * ctypes.sizeof(c_void_p))  # 24 on 64-bit, 16 on 32-bit


class VtableTest(unittest.TestCase):
    def test_indexes_follow_the_headers(self):
        # IUnknown is 0–2; each interface's own methods follow in header order.
        expected = {
            "IUnknown_QueryInterface": 0, "IUnknown_Release": 2,
            "IMMDeviceEnumerator_EnumAudioEndpoints": 3, "IMMDeviceEnumerator_GetDefaultAudioEndpoint": 4,
            "IMMDeviceCollection_GetCount": 3, "IMMDeviceCollection_Item": 4,
            "IMMDevice_Activate": 3, "IMMDevice_OpenPropertyStore": 4, "IMMDevice_GetId": 5,
            "IPropertyStore_GetValue": 5,
            "IAudioEndpointVolume_SetMasterVolumeLevelScalar": 7, "IAudioEndpointVolume_GetMasterVolumeLevelScalar": 9,
            "IAudioEndpointVolume_SetMute": 14, "IAudioEndpointVolume_GetMute": 15,
            "IAudioSessionManager2_GetSessionEnumerator": 5,
            "IAudioSessionEnumerator_GetCount": 3, "IAudioSessionEnumerator_GetSession": 4,
            "IAudioSessionControl2_GetProcessId": 14,
            "ISimpleAudioVolume_SetMasterVolume": 3, "ISimpleAudioVolume_GetMasterVolume": 4,
            "ISimpleAudioVolume_SetMute": 5, "ISimpleAudioVolume_GetMute": 6,
            "IPolicyConfig_SetDefaultEndpoint": 13,
        }
        for name, index in expected.items():
            with self.subTest(name=name):
                self.assertEqual(getattr(w, name)[0], index)

    def test_constants(self):
        self.assertEqual((w.eRender, w.eCapture), (0, 1))
        self.assertEqual((w.eConsole, w.eMultimedia, w.eCommunications), (0, 1, 2))
        self.assertEqual(w.DEVICE_STATE_ACTIVE, 1)
        self.assertEqual(w.CLSCTX_ALL, 0x17)
        self.assertEqual(w.RPC_E_CHANGED_MODE & 0xFFFFFFFF, 0x80010106)

    def test_windows_only_parts_explain_themselves_elsewhere(self):
        if sys.platform == "win32":
            self.skipTest("this is Windows")
        with self.assertRaisesRegex(OSError, "only on Windows"):
            w.get_volume()

    def test_app_matches(self):
        self.assertTrue(w.app_matches("Spotify.exe", "spotify"))
        self.assertTrue(w.app_matches("Discord.exe", "DISCORD.EXE"))
        self.assertFalse(w.app_matches("Discord.exe", "disc"))
        self.assertFalse(w.app_matches("", "x"))


class ComPlumbingTest(unittest.TestCase):
    def setUp(self):
        CALLBACK_ERRORS.clear()
        self.addCleanup(lambda: self.assertEqual(CALLBACK_ERRORS, []))
        patches = [mock.patch.object(w, "apartment", NoApartment)]
        for p in patches:
            p.start()
            self.addCleanup(p.stop)

    def patch_enumerator(self, enumerator):
        p = mock.patch.object(w, "create_enumerator", lambda: w.ComPtr(enumerator.address, "IMMDeviceEnumerator"))
        p.start()
        self.addCleanup(p.stop)

    def test_volume_and_mute(self):
        enumerator, device, endpoint, calls = endpoint_fakes(level=0.42, muted=1)
        self.patch_enumerator(enumerator)
        self.assertEqual(w.get_volume("output"), 42)
        self.assertTrue(w.get_mute("input"))
        w.set_volume(150, "output")
        w.set_mute(False, "input")
        self.assertIn(("default", w.eRender, w.eMultimedia), calls)
        self.assertIn(("default", w.eCapture, w.eMultimedia), calls)
        self.assertIn(("activate", w.IID_IAudioEndpointVolume, w.CLSCTX_ALL), calls)
        self.assertIn(("volume", 1.0, False), calls)  # clamped; no event context
        self.assertIn(("mute", 0, False), calls)
        # Each interface obtained was released exactly once per use (4 uses; the
        # enumerator is the patched factory's, released by the `with` too).
        self.assertEqual((enumerator.released, device.released, endpoint.released), (4, 4, 4))

    def test_errors_carry_the_hresult(self):
        enumerator = FakeCom("IMMDeviceEnumerator", {
            w.IMMDeviceEnumerator_GetDefaultAudioEndpoint: lambda flow, role, ppv: -2147023728,  # 0x80070490 not found
        })
        self.patch_enumerator(enumerator)
        with self.assertRaisesRegex(OSError, r"finding the default input device failed \(0x80070490\)"):
            w.get_mute("input")
        self.assertEqual(enumerator.released, 1)

    def test_null_interface_is_an_error(self):
        enumerator = FakeCom("IMMDeviceEnumerator", {w.IMMDeviceEnumerator_GetDefaultAudioEndpoint: lambda f, r, ppv: None})
        self.patch_enumerator(enumerator)
        with self.assertRaisesRegex(OSError, "0x80004003"):
            w.get_volume()

    def test_devices_and_default(self):
        keep = []

        def make_device(dev_id, name):
            def get_id(out):
                text = ctypes.create_unicode_buffer(dev_id)
                keep.append(text)
                out[0] = ctypes.addressof(text)

            def get_value(key, pv):
                assert str(key.contents.fmtid) == w.PKEY_Device_FriendlyName[0] and key.contents.pid == 14
                text = ctypes.create_unicode_buffer(name)
                keep.append(text)
                pv.contents.vt = w.VT_LPWSTR
                pv.contents.pwszVal = ctypes.addressof(text)

            store = FakeCom("IPropertyStore", {w.IPropertyStore_GetValue: get_value})
            keep.append(store)
            return FakeCom("IMMDevice", {
                w.IMMDevice_GetId: get_id,
                w.IMMDevice_OpenPropertyStore: lambda access, ppv: hand_out(ppv, store),
            })

        speakers = make_device("{0.0.0.00000000}.{aaa}", "Speakers (Realtek(R) Audio)")
        headset = make_device("{0.0.0.00000000}.{bbb}", "Headset (G435 Wireless)")

        def count(out):
            out[0] = 2

        collection = FakeCom("IMMDeviceCollection", {
            w.IMMDeviceCollection_GetCount: count,
            w.IMMDeviceCollection_Item: lambda i, ppv: hand_out(ppv, [speakers, headset][i]),
        })
        seen = []

        def enum(flow, state, ppv):
            seen.append((flow, state))
            hand_out(ppv, collection)

        enumerator = FakeCom("IMMDeviceEnumerator", {
            w.IMMDeviceEnumerator_EnumAudioEndpoints: enum,
            w.IMMDeviceEnumerator_GetDefaultAudioEndpoint: lambda flow, role, ppv: hand_out(ppv, headset),
        })
        self.patch_enumerator(enumerator)
        freed = []
        with mock.patch.object(w, "free_string", freed.append), mock.patch.object(w, "clear_propvariant", lambda pv: None):
            self.assertEqual(w.devices("output"), [
                ("{0.0.0.00000000}.{aaa}", "Speakers (Realtek(R) Audio)"),
                ("{0.0.0.00000000}.{bbb}", "Headset (G435 Wireless)"),
            ])
            self.assertEqual(w.default_device("output"), ("{0.0.0.00000000}.{bbb}", "Headset (G435 Wireless)"))
        self.assertEqual(seen, [(w.eRender, w.DEVICE_STATE_ACTIVE)])
        self.assertEqual(len(freed), 3)  # every GetId string goes back to CoTaskMemFree
        self.assertEqual(collection.released, 1)
        with self.assertRaises(ValueError):
            w.devices("speakers")

    def test_set_default_device_for_every_role(self):
        roles = []
        policy = FakeCom("IPolicyConfig", {w.IPolicyConfig_SetDefaultEndpoint: lambda dev_id, role: roles.append((dev_id, role))})
        with mock.patch.object(w, "create_policy_config", lambda: w.ComPtr(policy.address, "IPolicyConfig")):
            w.set_default_device("{0.0.0.00000000}.{bbb}")
        self.assertEqual(roles, [("{0.0.0.00000000}.{bbb}", r) for r in (w.eConsole, w.eMultimedia, w.eCommunications)])
        self.assertEqual(policy.released, 1)

    def test_app_sessions(self):
        changes = []

        def make_session(pid, level, muted):
            def get_level(out):
                out[0] = level

            def get_mute(out):
                out[0] = muted

            volume = FakeCom("ISimpleAudioVolume", {
                w.ISimpleAudioVolume_GetMasterVolume: get_level,
                w.ISimpleAudioVolume_GetMute: get_mute,
                w.ISimpleAudioVolume_SetMasterVolume: lambda v, ctx: changes.append((pid, "volume", round(v, 2))),
                w.ISimpleAudioVolume_SetMute: lambda m, ctx: changes.append((pid, "mute", m)),
            })

            def get_pid(out):
                out[0] = pid

            control2 = FakeCom("IAudioSessionControl2", {w.IAudioSessionControl2_GetProcessId: get_pid})

            def query(iid, ppv):
                wanted = str(iid.contents)
                if wanted == w.IID_IAudioSessionControl2:
                    hand_out(ppv, control2)
                elif wanted == w.IID_ISimpleAudioVolume:
                    hand_out(ppv, volume)
                else:
                    return -2147467262  # E_NOINTERFACE

            return FakeCom("IAudioSessionControl", {w.IUnknown_QueryInterface: query}), volume

        system, _ = make_session(0, 1.0, 0)
        spotify, spotify_volume = make_session(4242, 0.55, 0)
        firefox, _ = make_session(777, 0.9, 1)
        sessions_list = [system, spotify, firefox]

        def count(out):
            out[0] = len(sessions_list)

        sessions = FakeCom("IAudioSessionEnumerator", {
            w.IAudioSessionEnumerator_GetCount: count,
            w.IAudioSessionEnumerator_GetSession: lambda i, ppv: hand_out(ppv, sessions_list[i]),
        })
        manager = FakeCom("IAudioSessionManager2", {w.IAudioSessionManager2_GetSessionEnumerator: lambda ppv: hand_out(ppv, sessions)})

        def activate(iid, clsctx, params, ppv):
            assert str(iid.contents) == w.IID_IAudioSessionManager2
            hand_out(ppv, manager)

        def get_id(out):
            text = ctypes.create_unicode_buffer("{0.0.0.00000000}.{aaa}")
            get_id.keep = text
            out[0] = ctypes.addressof(text)

        def get_value(key, pv):
            pv.contents.vt = 0  # VT_EMPTY: a device without a name

        store = FakeCom("IPropertyStore", {w.IPropertyStore_GetValue: get_value})
        device = FakeCom("IMMDevice", {
            w.IMMDevice_Activate: activate, w.IMMDevice_GetId: get_id,
            w.IMMDevice_OpenPropertyStore: lambda access, ppv: hand_out(ppv, store),
        })

        def one(out):
            out[0] = 1

        collection = FakeCom("IMMDeviceCollection", {
            w.IMMDeviceCollection_GetCount: one,
            w.IMMDeviceCollection_Item: lambda i, ppv: hand_out(ppv, device),
        })
        enumerator = FakeCom("IMMDeviceEnumerator", {w.IMMDeviceEnumerator_EnumAudioEndpoints: lambda f, s, ppv: hand_out(ppv, collection)})
        self.patch_enumerator(enumerator)
        names = {4242: "Spotify.exe", 777: "firefox.exe"}
        with mock.patch.object(w, "process_name", lambda pid: names.get(pid, "")), \
                mock.patch.object(w, "free_string", lambda a: None), mock.patch.object(w, "clear_propvariant", lambda pv: None):
            self.assertEqual(w.get_app_volume("spotify"), (55, False))
            self.assertEqual(w.get_app_volume("FIREFOX"), (90, True))
            self.assertIsNone(w.get_app_volume("discord"))
            self.assertEqual(w.set_app_volume("Spotify.exe", percent=20, muted=True), 1)
            self.assertEqual(w.set_app_volume("discord", percent=20), 0)
        self.assertEqual(changes, [(4242, "volume", 0.2), (4242, "mute", 1)])
        # Stopping early (get_app_volume returns at the first match) still releases.
        self.assertEqual(spotify_volume.released, 2)
        self.assertEqual(sessions.released, manager.released)
        self.assertEqual(collection.released, 5)


if __name__ == "__main__":
    unittest.main()
