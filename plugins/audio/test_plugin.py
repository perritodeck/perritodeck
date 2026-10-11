import os
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as audio  # noqa: E402

# `wpctl status` from a real PipeWire 1.2.7 system (posted on a public forum; the
# user and host names are replaced). Two sinks, a source, and a stream with ports.
WPCTL_STATUS = """\
PipeWire 'pipewire-0' [1.2.7, user@pc, cookie:1786503093]
 └─ Clients:
        32. WirePlumber                         [1.2.7, user@pc, pid:1128]
        33. pipewire                            [1.2.7, user@pc, pid:1127]
        75. parole                              [1.2.7, user@pc, pid:6323]
        82. wpctl                               [1.2.7, user@pc, pid:6498]

Audio
 ├─ Devices:
 │      47. Built-in Audio                      [alsa]
 │
 ├─ Sinks:
 │  *   51. Built-in Audio Pro                  [vol: 1.24]
 │      52. Built-in Audio Pro 3                [vol: 1.00]
 │
 ├─ Sources:
 │  *   53. Built-in Audio Pro                  [vol: 1.00]
 │
 ├─ Filters:
 │
 └─ Streams:
        89. parole
             76. output_FR       > ALC269 Analog:playback_AUX1	[active]
             83. output_FL       > ALC269 Analog:playback_AUX0	[active]

Video
 ├─ Devices:
 │
 ├─ Sinks:
 │
 ├─ Sources:
 │
 ├─ Filters:
 │
 └─ Streams:

Settings
 └─ Default Configured Devices:
         0. Audio/Sink    alsa_output.pci-0000_00_1b.0.stereo-fallback
"""

# Also real (PipeWire 1.2.6, PulseAudio holding the sound card): no audio nodes at
# all, and a default *video* source that mustn't be taken for a microphone.
WPCTL_STATUS_NO_AUDIO = """\
PipeWire 'pipewire-0' [1.2.6, user@pc, cookie:1685379383]
 └─ Clients:
        33. WirePlumber                         [1.2.6, user@pc, pid:1131]
        45. wpctl                               [1.2.6, user@pc, pid:2539]

Audio
 ├─ Devices:
 │
 ├─ Sinks:
 │
 ├─ Sources:
 │
 ├─ Filters:
 │
 └─ Streams:

Video
 ├─ Devices:
 │      42. USB2.0 VGA UVC WebCam               [v4l2]
 │
 ├─ Sinks:
 │
 ├─ Sources:
 │  *   41. USB2.0 VGA UVC WebCam (V4L2)
 │
 ├─ Filters:
 │
 └─ Streams:

Settings
 └─ Default Configured Devices:
"""

# Real `pactl list sink-inputs` (PulseAudio, Finnish locale, so "kan." and "7,50");
# the tabs are pactl's own.
PACTL_SINK_INPUTS = """\
Sink Input #304
\tDriver: protocol-native.c
\tOwner Module: 13
\tClient: 105
\tSink: 1
\tSample Specification: float32le 2 kan. 44100Hz
\tChannel Map: front-left,front-right
\tFormat: pcm, format.sample_format = "\\"float32le\\""  format.channels = "2"  format.rate = "44100"  format.channel_map = "\\"front-left,front-right\\""
\tCorked: no
\tMute: no
\tVolume: front-left: 49152 /  75% / -7,50 dB,   front-right: 49152 /  75% / -7,50 dB
\t        balance 0,00
\tBuffer Latency: 105374 usec
\tSink Latency: 18205 usec
\tResample method: copy
\tProperties:
\t\tmedia.name = "XIII"
\t\tapplication.name = "Clementine"
\t\tnative-protocol.peer = "UNIX socket client"
\t\tmedia.role = "music"
\t\tapplication.process.id = "1474"
\t\tapplication.process.binary = "clementine"
\t\tapplication.icon_name = "clementine"
\t\tmodule-stream-restore.id = "sink-input-by-media-role:music"
Sink Input #305
\tDriver: protocol-native.c
\tSink: 1
\tCorked: no
\tMute: yes
\tVolume: front-left: 32768 /  50% / -18.06 dB,   front-right: 32768 /  50% / -18.06 dB
\t        balance 0.00
\tProperties:
\t\tapplication.name = "Firefox"
\t\tapplication.process.binary = "firefox"
"""

# `pactl list sinks` / `sources` in PulseAudio's layout (fields one tab in, ports
# two); composed from the format, not captured from a machine.
PACTL_SINKS = """\
Sink #0
\tState: SUSPENDED
\tName: alsa_output.pci-0000_00_1f.3.analog-stereo
\tDescription: Built-in Audio Analog Stereo
\tDriver: module-alsa-card.c
\tMute: no
\tVolume: front-left: 26214 /  40% / -23.88 dB,   front-right: 26214 /  40% / -23.88 dB
\t        balance 0.00
\tProperties:
\t\tdevice.description = "Built-in Audio Analog Stereo"
\t\tdevice.class = "sound"
\tPorts:
\t\tanalog-output-speaker: Speakers (type: Speaker, priority: 10000, availability unknown)
\tActive Port: analog-output-speaker

Sink #3
\tState: RUNNING
\tName: alsa_output.usb-Logitech_G435-00.analog-stereo
\tDescription: G435 Wireless Gaming Headset Analog Stereo
\tMute: no
\tProperties:
\t\tdevice.class = "sound"
"""

PACTL_SOURCES = """\
Source #1
\tState: SUSPENDED
\tName: alsa_output.pci-0000_00_1f.3.analog-stereo.monitor
\tDescription: Monitor of Built-in Audio Analog Stereo
\tProperties:
\t\tdevice.class = "monitor"

Source #2
\tState: RUNNING
\tName: alsa_input.usb-Blue_Yeti-00.analog-stereo
\tDescription: Yeti Stereo Microphone Analog Stereo
\tProperties:
\t\tdevice.class = "sound"
"""


class FakeRun:
    """Answers commands from a table keyed by the argv after the program."""

    def __init__(self, answers=None):
        self.answers = dict(answers or {})
        self.calls = []

    def __call__(self, argv, timeout=None):
        self.calls.append(list(argv))
        key = tuple(argv[1:])
        if key in self.answers:
            answer = self.answers[key]
            if isinstance(answer, Exception):
                raise answer
            return answer
        if argv[1:2] and argv[1].startswith("set") or (len(argv) > 2 and argv[1] == "-t"):
            return ""
        if argv[1] == "-e" and argv[2].startswith("set volume"):
            return ""
        raise RuntimeError(f"unexpected command {argv}")

    def changes(self):
        return [c for c in self.calls if c[1].startswith("set") or c[1:2] == ["-t"] or (c[1] == "-e" and c[2].startswith("set"))]


def linux(answers, wpctl=True, pactl=True):
    which = lambda name: {"wpctl": "/usr/bin/wpctl" if wpctl else None, "pactl": "/usr/bin/pactl" if pactl else None}.get(name)  # noqa: E731
    fake = FakeRun(answers)
    return audio.LinuxAudio(which=which, runner=fake), fake


class Base(unittest.TestCase):
    def setUp(self):
        self.sent = []
        audio.plugin._send = self.sent.append
        audio.plugin.watching = set()
        audio._published.clear()
        audio._logged.clear()
        self.addCleanup(setattr, audio, "_backend", None)

    def use(self, backend):
        audio._backend = backend

    def published(self):
        return {m["id"]: m["value"] for m in self.sent if m.get("event") == "value"}


class LinuxParsingTest(Base):
    def test_wpctl_volume(self):
        self.assertEqual(audio.parse_wpctl_volume("Volume: 0.40\n"), (40, False))
        self.assertEqual(audio.parse_wpctl_volume("Volume: 0.40 [MUTED]\n"), (40, True))
        self.assertEqual(audio.parse_wpctl_volume("Volume: 1.24\n"), (100, False))  # boosted past 100 %
        with self.assertRaisesRegex(RuntimeError, "unexpected"):
            audio.parse_wpctl_volume("Object not found\n")

    def test_wpctl_status(self):
        status = audio.parse_wpctl_status(WPCTL_STATUS)
        self.assertEqual(
            status["sinks"],
            [{"id": "51", "name": "Built-in Audio Pro", "default": True},
             {"id": "52", "name": "Built-in Audio Pro 3", "default": False}],
        )
        self.assertEqual(status["sources"], [{"id": "53", "name": "Built-in Audio Pro", "default": True}])
        self.assertEqual(status["streams"], [{"id": "89", "name": "parole", "default": False}])

    def test_wpctl_status_ignores_video(self):
        status = audio.parse_wpctl_status(WPCTL_STATUS_NO_AUDIO)
        self.assertEqual(status, {"sinks": [], "sources": [], "streams": []})

    def test_wpctl_status_muted_sink(self):
        text = WPCTL_STATUS.replace("[vol: 1.00]\n │  \n ├─ Sources", "[vol: 1.00 MUTED]\n │  \n ├─ Sources")
        names = [s["name"] for s in audio.parse_wpctl_status(text)["sinks"]]
        self.assertEqual(names, ["Built-in Audio Pro", "Built-in Audio Pro 3"])

    def test_pactl_volume_and_mute(self):
        self.assertEqual(audio.parse_pactl_volume(
            "Volume: front-left: 26214 /  40% / -23.88 dB,   front-right: 26214 /  40% / -23.88 dB\n        balance 0.00\n"), 40)
        self.assertEqual(audio.parse_pactl_volume("Volume: mono: 98304 / 150% / 10.57 dB\n"), 100)
        self.assertTrue(audio.parse_pactl_mute("Mute: yes\n"))
        self.assertFalse(audio.parse_pactl_mute("Mute: no\n"))

    def test_pactl_sink_inputs(self):
        streams = audio.parse_pactl_sink_inputs(PACTL_SINK_INPUTS)
        self.assertEqual(streams, [
            {"id": "304", "names": ["clementine", "Clementine"], "volume": 75, "muted": False},
            {"id": "305", "names": ["firefox", "Firefox"], "volume": 50, "muted": True},
        ])

    def test_pactl_output_pasted_with_spaces(self):
        streams = audio.parse_pactl_sink_inputs(PACTL_SINK_INPUTS.replace("\t", " " * 8))
        self.assertEqual([s["id"] for s in streams], ["304", "305"])
        self.assertEqual(streams[0]["names"], ["clementine", "Clementine"])

    def test_pactl_devices(self):
        self.assertEqual(audio.parse_pactl_devices(PACTL_SINKS, "Sink"), [
            ("alsa_output.pci-0000_00_1f.3.analog-stereo", "Built-in Audio Analog Stereo"),
            ("alsa_output.usb-Logitech_G435-00.analog-stereo", "G435 Wireless Gaming Headset Analog Stereo"),
        ])
        self.assertEqual(audio.parse_pactl_devices(PACTL_SOURCES, "Source"), [
            ("alsa_input.usb-Blue_Yeti-00.analog-stereo", "Yeti Stereo Microphone Analog Stereo"),
        ])
        # "Sink Input #" blocks aren't sinks.
        self.assertEqual(audio.parse_pactl_devices(PACTL_SINK_INPUTS, "Sink"), [])


class LinuxBackendTest(Base):
    def wpctl_answers(self, sink="Volume: 0.40\n", source="Volume: 1.00 [MUTED]\n"):
        return {
            ("get-volume", "@DEFAULT_AUDIO_SINK@"): sink,
            ("get-volume", "@DEFAULT_AUDIO_SOURCE@"): source,
            ("status",): WPCTL_STATUS,
            ("list", "sink-inputs"): PACTL_SINK_INPUTS,
        }

    def test_snapshot_and_values(self):
        backend, fake = linux(self.wpctl_answers())
        self.use(backend)
        audio.plugin.watching = {"app_volume:clementine", "app_volume:spotify"}
        audio.refresh()
        self.assertEqual(self.published(), {
            "volume": "40", "output_muted": "off", "mic_muted": "on",
            "output_device": "Built-in Audio Pro", "input_device": "Built-in Audio Pro",
            "app_volume:clementine": "75", "app_volume:spotify": "–",
        })
        self.assertEqual(sum(1 for c in fake.calls if c[1:] == ["status"]), 1)
        # Nothing changed: nothing is sent again.
        self.sent.clear()
        audio.refresh()
        self.assertEqual(self.sent, [])

    def test_unwatched_apps_are_not_polled(self):
        backend, fake = linux(self.wpctl_answers())
        self.use(backend)
        audio.refresh()
        self.assertNotIn(["/usr/bin/pactl", "list", "sink-inputs"], fake.calls)

    def test_watch_change_republishes(self):
        backend, _ = linux(self.wpctl_answers())
        self.use(backend)
        audio.plugin.watching = {"app_volume:firefox"}
        audio.refresh()
        audio.plugin.watching = set()
        audio.watch_changed([])
        self.assertNotIn("app_volume:firefox", audio._published)

    def test_no_microphone_shows_unknown(self):
        answers = self.wpctl_answers()
        answers[("get-volume", "@DEFAULT_AUDIO_SOURCE@")] = RuntimeError("wpctl: Object '@DEFAULT_AUDIO_SOURCE@' not found")
        answers[("status",)] = WPCTL_STATUS_NO_AUDIO
        backend, _ = linux(answers)
        self.use(backend)
        audio.refresh()
        values = self.published()
        self.assertEqual((values["mic_muted"], values["input_device"], values["output_device"]), ("–", "–", "–"))
        self.assertTrue(any("microphone" in m["message"] for m in self.sent if m.get("event") == "log"))

    def test_mute_and_volume_actions(self):
        backend, fake = linux(self.wpctl_answers())
        self.use(backend)
        audio.mic_mute({"mode": "toggle"})  # muted now, so unmute
        audio.output_mute({})  # toggle by default; not muted, so mute
        audio.volume_up({"step": 10})
        audio.volume_down({})
        audio.volume({"value": 63})
        self.assertEqual(fake.changes(), [
            ["/usr/bin/wpctl", "set-mute", "@DEFAULT_AUDIO_SOURCE@", "0"],
            ["/usr/bin/wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "1"],
            ["/usr/bin/wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "0.50"],
            ["/usr/bin/wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "0.35"],
            ["/usr/bin/wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "0.63"],
        ])

    def test_volume_stays_in_range(self):
        backend, fake = linux(self.wpctl_answers(sink="Volume: 0.98\n"))
        self.use(backend)
        audio.volume_up({"step": "5"})
        audio.volume({"value": 150})
        self.assertEqual([c[-1] for c in fake.changes()], ["1.00", "1.00"])
        with self.assertRaisesRegex(ValueError, "isn't a number"):
            audio.volume_up({"step": "lots"})

    def test_switch_output_device_cycles(self):
        backend, fake = linux(self.wpctl_answers())
        self.use(backend)
        # The current default is "Built-in Audio Pro": the next in the list is "… Pro 3".
        self.assertIsNone(audio.output_device({"devices": "built-in audio pro, Pro 3"}))
        self.assertEqual(fake.changes(), [["/usr/bin/wpctl", "set-default", "52"]])

    def test_switch_device_not_found(self):
        backend, _ = linux(self.wpctl_answers())
        self.use(backend)
        with self.assertRaisesRegex(RuntimeError, "no output device called “HDMI” \\(found: Built-in Audio Pro, Built-in Audio Pro 3\\)"):
            audio.output_device({"devices": "HDMI"})
        with self.assertRaisesRegex(ValueError, "device name"):
            audio.input_device({"devices": " , "})

    def test_switch_device_warns_about_missing_names(self):
        backend, fake = linux(self.wpctl_answers())
        self.use(backend)
        self.assertEqual(audio.input_device({"devices": "Yeti, Built-in"}), "not found: Yeti")
        self.assertEqual(fake.changes(), [["/usr/bin/wpctl", "set-default", "53"]])

    def test_app_volume_with_pactl(self):
        backend, fake = linux(self.wpctl_answers())
        self.use(backend)
        audio.app_volume({"app": "Clementine", "value": 30})
        audio.app_mute({"app": "firefox.exe", "mode": "toggle"})  # muted: unmute
        self.assertEqual(fake.changes(), [
            ["/usr/bin/pactl", "set-sink-input-volume", "304", "30%"],
            ["/usr/bin/pactl", "set-sink-input-mute", "305", "0"],
        ])
        self.assertEqual(audio.app_volume({"app": "spotify", "value": 30}), "spotify isn't playing any sound right now")
        self.assertEqual(audio.app_mute({"app": "spotify"}), "spotify isn't playing any sound right now")
        with self.assertRaisesRegex(ValueError, "program name"):
            audio.app_volume({"app": " ", "value": 1})

    def test_app_volume_with_wpctl_only(self):
        answers = self.wpctl_answers()
        answers[("get-volume", "89")] = "Volume: 0.80 [MUTED]\n"
        backend, fake = linux(answers, pactl=False)
        self.assertEqual(backend.app_volume("Parole"), (80, True))
        self.assertIsNone(backend.app_volume("spotify"))
        self.assertEqual(backend.set_app("parole", percent=25, muted=False), 1)
        self.assertEqual(fake.changes(), [
            ["/usr/bin/wpctl", "set-volume", "89", "0.25"],
            ["/usr/bin/wpctl", "set-mute", "89", "0"],
        ])

    def test_pactl_only(self):
        answers = {
            ("get-sink-volume", "@DEFAULT_SINK@"): "Volume: front-left: 26214 /  40% / -23.88 dB,   front-right: 26214 /  40% / -23.88 dB\n",
            ("get-sink-mute", "@DEFAULT_SINK@"): "Mute: no\n",
            ("get-source-mute", "@DEFAULT_SOURCE@"): "Mute: yes\n",
            ("list", "sinks"): PACTL_SINKS,
            ("list", "sources"): PACTL_SOURCES,
            ("get-default-sink",): "alsa_output.usb-Logitech_G435-00.analog-stereo\n",
            ("get-default-source",): "alsa_input.usb-Blue_Yeti-00.analog-stereo\n",
        }
        backend, fake = linux(answers, wpctl=False)
        self.use(backend)
        audio.refresh()
        self.assertEqual(self.published(), {
            "volume": "40", "output_muted": "off", "mic_muted": "on",
            "output_device": "G435 Wireless Gaming Headset Analog Stereo",
            "input_device": "Yeti Stereo Microphone Analog Stereo",
        })
        audio.output_device({"devices": "Built-in, G435"})  # G435 is current: back to the first
        audio.mic_mute({"mode": "unmute"})
        audio.output_mute({"mode": "mute"})
        audio.volume({"value": 20})
        self.assertEqual(fake.changes(), [
            ["/usr/bin/pactl", "set-default-sink", "alsa_output.pci-0000_00_1f.3.analog-stereo"],
            ["/usr/bin/pactl", "set-source-mute", "@DEFAULT_SOURCE@", "0"],
            ["/usr/bin/pactl", "set-sink-mute", "@DEFAULT_SINK@", "1"],
            ["/usr/bin/pactl", "set-sink-volume", "@DEFAULT_SINK@", "20%"],
        ])

    def test_neither_tool(self):
        backend, _ = linux({}, wpctl=False, pactl=False)
        self.use(backend)
        with self.assertRaisesRegex(RuntimeError, "wpctl .* or pactl"):
            audio.volume({"value": 10})
        audio.refresh()
        self.assertEqual(self.published()["volume"], "–")

    def test_run_reports_the_error_line(self):
        done = audio.subprocess.CompletedProcess([], 1, stdout="", stderr="warning\nObject not found\n")
        with mock.patch.object(audio.subprocess, "run", return_value=done) as run:
            with self.assertRaisesRegex(RuntimeError, "^wpctl: Object not found$"):
                audio.run(["/usr/bin/wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"])
        self.assertEqual(run.call_args.kwargs["env"]["LC_ALL"], "C")
        self.assertEqual(run.call_args.kwargs["stdin"], audio.subprocess.DEVNULL)


class SharedLogicTest(Base):
    def test_choose_device(self):
        devices = [("1", "Speakers (Realtek(R) Audio)"), ("2", "Headphones (G435)"), ("3", "LG TV (NVIDIA High Definition Audio)")]
        self.assertEqual(audio.choose_device(devices, None, "headphones"), (("2", "Headphones (G435)"), []))
        self.assertEqual(audio.choose_device(devices, "Speakers (Realtek(R) Audio)", "Speakers, Headphones, TV")[0][0], "2")
        self.assertEqual(audio.choose_device(devices, "LG TV (NVIDIA High Definition Audio)", "Speakers, Headphones, TV")[0][0], "1")
        self.assertEqual(audio.choose_device(devices, "Headphones (G435)", "Speakers, TV")[0][0], "1")
        # An exact name wins over one that merely contains it.
        self.assertEqual(audio.choose_device([("a", "Mic 2"), ("b", "Mic")], None, "mic")[0][0], "b")
        # A single name always switches to it, even when it's already current.
        self.assertEqual(audio.choose_device(devices, "Headphones (G435)", "Headphones")[0][0], "2")

    def test_want_muted(self):
        self.assertTrue(audio.want_muted("toggle", False))
        self.assertFalse(audio.want_muted(None, True))
        self.assertTrue(audio.want_muted("mute", True))
        self.assertFalse(audio.want_muted("unmute", False))
        with self.assertRaisesRegex(ValueError, "unknown mode"):
            audio.want_muted("flip", False)

    def test_app_matches(self):
        self.assertTrue(audio.app_matches(["Spotify.exe"], "spotify"))
        self.assertTrue(audio.app_matches([None, "Discord"], "discord.exe"))
        self.assertFalse(audio.app_matches(["firefox"], "fire"))
        self.assertFalse(audio.app_matches(["firefox"], ""))

    def test_backend_by_platform(self):
        self.assertIsInstance(audio.make_backend("linux"), audio.LinuxAudio)
        self.assertIsInstance(audio.make_backend("darwin"), audio.MacAudio)
        self.assertIsInstance(audio.make_backend("win32"), audio.WindowsAudio)  # importing win_audio works anywhere


class MacTest(Base):
    def mac(self, settings="output volume:44, input volume:75, alert volume:100, output muted:false", switch=True, extra=None):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        which = lambda name: {"osascript": "/usr/bin/osascript", "SwitchAudioSource": "/opt/homebrew/bin/SwitchAudioSource" if switch else None}.get(name)  # noqa: E731
        answers = {("-e", "get volume settings"): settings + "\n"}
        answers.update(extra or {})
        fake = FakeRun(answers)
        with mock.patch.object(audio.os, "access", return_value=False):
            backend = audio.MacAudio(which=which, runner=fake, state_file=os.path.join(self.tmp.name, "level.txt"))
        self.use(backend)
        return backend, fake

    def test_parse_settings(self):
        self.assertEqual(
            audio.parse_mac_settings("output volume:44, input volume:75, alert volume:100, output muted:false\n"),
            {"output volume": 44, "input volume": 75, "alert volume": 100, "output muted": False},
        )
        self.assertEqual(
            audio.parse_mac_settings("output volume:missing value, input volume:50, alert volume:100, output muted:missing value"),
            {"output volume": None, "input volume": 50, "alert volume": 100, "output muted": None},
        )
        with self.assertRaisesRegex(RuntimeError, "unexpected"):
            audio.parse_mac_settings("execution error")

    def test_parse_switch_audio_list(self):
        self.assertEqual(audio.parse_switch_audio_list("MacBook Pro Speakers\nAirPods Pro\n", "output"),
                         ["MacBook Pro Speakers", "AirPods Pro"])
        old = "Built-in Microphone (input)\nBuilt-in Output (output)\nAirPods (output)\n"
        self.assertEqual(audio.parse_switch_audio_list(old, "output"), ["Built-in Output", "AirPods"])
        self.assertEqual(audio.parse_switch_audio_list(old, "input"), ["Built-in Microphone"])

    def test_values(self):
        self.mac(extra={("-c", "-t", "output"): "AirPods Pro\n", ("-c", "-t", "input"): "MacBook Pro Microphone\n"})
        audio.refresh()
        self.assertEqual(self.published(), {
            "volume": "44", "output_muted": "off", "mic_muted": "off",
            "output_device": "AirPods Pro", "input_device": "MacBook Pro Microphone",
        })

    def test_osascript_commands(self):
        _, fake = self.mac()
        audio.volume({"value": 30})
        audio.output_mute({"mode": "toggle"})
        audio.volume_down({"step": 4})
        self.assertEqual(fake.changes(), [
            ["/usr/bin/osascript", "-e", "set volume output volume 30"],
            ["/usr/bin/osascript", "-e", "set volume output muted true"],
            ["/usr/bin/osascript", "-e", "set volume output volume 40"],
        ])

    def test_mic_mute_remembers_the_level(self):
        backend, fake = self.mac()
        audio.mic_mute({"mode": "toggle"})
        self.assertEqual(fake.changes(), [["/usr/bin/osascript", "-e", "set volume input volume 0"]])
        with open(backend.state_file()) as f:
            self.assertEqual(f.read(), "75")
        # Now the mic is at 0: unmuting puts the old level back.
        backend2, fake2 = self.mac(settings="output volume:44, input volume:0, alert volume:100, output muted:false")
        backend2._state_file = backend.state_file()
        audio.mic_mute({})
        self.assertEqual(fake2.changes(), [["/usr/bin/osascript", "-e", "set volume input volume 75"]])

    def test_mic_unmute_without_a_saved_level(self):
        _, fake = self.mac(settings="output volume:44, input volume:0, alert volume:100, output muted:false")
        audio.mic_mute({"mode": "unmute"})
        self.assertEqual(fake.changes(), [["/usr/bin/osascript", "-e", f"set volume input volume {audio.MAC_DEFAULT_INPUT}"]])
        self.assertEqual(self.published()["mic_muted"], "on")  # the fake doesn't change its answer

    def test_device_without_volume_control(self):
        self.mac(settings="output volume:missing value, input volume:50, alert volume:100, output muted:missing value")
        self.assertEqual(audio.volume_up({}), "macOS can't change this device's volume")

    def test_switch_devices(self):
        _, fake = self.mac(extra={
            ("-a", "-t", "output"): "MacBook Pro Speakers\nAirPods Pro\nLG HDR 4K\n",
            ("-c", "-t", "output"): "AirPods Pro\n",
            ("-c", "-t", "input"): "MacBook Pro Microphone\n",
        })
        self.assertIsNone(audio.output_device({"devices": "Speakers, AirPods"}))
        self.assertEqual(fake.changes(), [["/opt/homebrew/bin/SwitchAudioSource", "-t", "output", "-s", "MacBook Pro Speakers"]])

    def test_switch_needs_switch_audio_source(self):
        self.mac(switch=False)
        self.assertIn("SwitchAudioSource", audio.output_device({"devices": "AirPods"}))
        self.assertIn("brew install switchaudio-osx", audio.input_device({"devices": "Yeti"}))
        audio.refresh()
        self.assertEqual(self.published()["output_device"], "–")
        self.assertEqual(self.published()["volume"], "44")

    def test_no_app_volume(self):
        self.mac()
        self.assertEqual(audio.app_volume({"app": "Spotify", "value": 20}), "macOS has no volume for each app")
        self.assertEqual(audio.app_mute({"app": "Spotify"}), "macOS has no volume for each app")
        audio.plugin.watching = {"app_volume:Spotify"}
        audio.refresh()
        self.assertEqual(self.published()["app_volume:Spotify"], "–")


class FakeWinAudio:
    """Stands in for win_audio, to test WindowsAudio's wiring."""

    def __init__(self):
        self.calls = []
        self.volume, self.muted, self.mic_muted = 30, False, True
        self.outputs = [("{0.0.0.00000000}.{aaa}", "Speakers (Realtek(R) Audio)"), ("{0.0.0.00000000}.{bbb}", "Headphones (G435)")]
        self.default = self.outputs[0]

    def get_volume(self, kind):
        return self.volume

    def get_mute(self, kind):
        return self.muted if kind == "output" else self.mic_muted

    def set_volume(self, percent, kind):
        self.calls.append(("set_volume", percent, kind))

    def set_mute(self, muted, kind):
        self.calls.append(("set_mute", muted, kind))

    def devices(self, kind):
        return list(self.outputs)

    def default_device(self, kind):
        if kind == "input":
            raise OSError("no microphone")
        return self.default

    def set_default_device(self, device_id):
        self.calls.append(("set_default_device", device_id))

    def get_app_volume(self, app):
        return (55, False) if app.lower().startswith("spotify") else None

    def set_app_volume(self, app, percent=None, muted=None):
        self.calls.append(("set_app_volume", app, percent, muted))
        return 1 if app.lower().startswith("spotify") else 0


class WindowsWiringTest(Base):
    def test_actions_and_values(self):
        fake = FakeWinAudio()
        self.use(audio.WindowsAudio(fake))
        audio.plugin.watching = {"app_volume:Spotify.exe", "app_volume:discord"}
        audio.refresh()
        self.assertEqual(self.published(), {
            "volume": "30", "output_muted": "off", "mic_muted": "on",
            "output_device": "Speakers (Realtek(R) Audio)", "input_device": "–",
            "app_volume:Spotify.exe": "55", "app_volume:discord": "–",
        })
        audio.mic_mute({})
        audio.volume_up({"step": 5})
        audio.output_device({"devices": "speakers, headphones"})
        audio.app_volume({"app": "Spotify.exe", "value": 12})
        audio.app_mute({"app": "spotify", "mode": "mute"})
        self.assertEqual(fake.calls, [
            ("set_mute", False, "input"),
            ("set_volume", 35, "output"),
            ("set_default_device", "{0.0.0.00000000}.{bbb}"),
            ("set_app_volume", "Spotify.exe", 12, None),
            ("set_app_volume", "spotify", None, True),
        ])
        self.assertEqual(audio.app_mute({"app": "discord"}), "discord isn't playing any sound right now")


if __name__ == "__main__":
    unittest.main()
