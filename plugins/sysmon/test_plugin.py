import ctypes
import os
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as sysmon  # noqa: E402
import winapi  # noqa: E402  (must import on every platform)

PROC_STAT_1 = """\
cpu  10132153 290696 3084719 46828483 16683 0 25195 0 175628 0
cpu0 1393280 32966 572056 13343292 6130 0 17875 0 23933 0
intr 1462898 0 0 0
ctxt 3542353
btime 1728600000
"""
# 400 ticks later: 100 busy (user +80, system +20), 300 idle.
PROC_STAT_2 = PROC_STAT_1.replace("10132153", "10132233").replace("3084719", "3084739").replace("46828483", "46828783")

MEMINFO = """\
MemTotal:       16303428 kB
MemFree:         1214328 kB
MemAvailable:    9183676 kB
Buffers:          412700 kB
Cached:          7400128 kB
SwapCached:            0 kB
HugePages_Total:       0
"""

NET_DEV = """\
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 2776770   11307    0    0    0     0          0         0  2776770   11307    0    0    0     0       0          0
  eth0: 1215645    2751    0    0    0     0          0         0  1782404    4324    0    0    0   427       0          0
docker0:     100       1    0    0    0     0          0         0      200       2    0    0    0     0       0          0
 wlan0:    5000      10    0    0    0     0          0         0     7000      12    0    0    0     0       0          0
"""

VM_STAT = """\
Mach Virtual Memory Statistics: (page size of 16384 bytes)
Pages free:                                3632.
Pages active:                            243163.
Pages inactive:                          238495.
Pages speculative:                         3115.
Pages throttled:                              0.
Pages wired down:                        122520.
Pages purgeable:                           2861.
"Translation faults":                 512473196.
Pages copy-on-write:                   15297416.
Pages zero filled:                    236316402.
Pages reactivated:                      3606327.
Pages purged:                           1458133.
File-backed pages:                       167932.
Anonymous pages:                         316841.
Pages stored in compressor:              730281.
Pages occupied by compressor:            164123.
Decompressions:                         6474468.
Compressions:                           9420916.
Pageins:                               37227436.
Pageouts:                                244618.
Swapins:                                      0.
Swapouts:                                     0.
"""

NETSTAT_IB = """\
Name       Mtu   Network       Address            Ipkts Ierrs     Ibytes    Opkts Oerrs     Obytes  Coll
lo0        16384 <Link#1>                         1234     0     567890     1234     0     567890     0
lo0        16384 127           localhost          1234     -     567890     1234     -     567890     -
gif0*      1280  <Link#2>                            0     0          0        0     0          0     0
en0        1500  <Link#6>    a4:83:e7:12:34:56  9876543     0 12345678901  5432109     0  987654321     0
en0        1500  fe80::1c2b: fe80:6::1c2b:3a4d  9876543     - 12345678901  5432109     -  987654321     -
en0        1500  192.168.1     192.168.1.23     9876543     - 12345678901  5432109     -  987654321     -
utun0      1380  <Link#14>                          10     0       1000       12     0       2000     0
"""

NVIDIA = "23, 54, 2048, 8192\n5, 40, 100, 4096\n"
PMSET = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=4653155)\t85%; discharging; 3:12 remaining present: true\n"
PMSET_DESKTOP = "Now drawing from 'AC Power'\n"
BOOTTIME = "{ sec = 1728600000, usec = 123456 } Fri Oct 11 00:00:00 2024\n"


def write(root, rel, text):
    path = os.path.join(root, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def done(stdout="", returncode=0, stderr=""):
    return subprocess.CompletedProcess([], returncode, stdout=stdout, stderr=stderr)


class FormatTest(unittest.TestCase):
    def test_pct(self):
        self.assertEqual(sysmon.pct(23.4), "23")
        self.assertEqual(sysmon.pct(100.7), "100")
        self.assertEqual(sysmon.pct(None), "–")

    def test_size(self):
        self.assertEqual(sysmon.size(512, per_second=True), "512 B/s")
        self.assertEqual(sysmon.size(820 * 1024, per_second=True), "820 KB/s")
        self.assertEqual(sysmon.size(1.4 * 1024 ** 2, per_second=True), "1.4 MB/s")
        self.assertEqual(sysmon.size(11.2 * 1024 ** 3), "11.2 GB")
        self.assertEqual(sysmon.size(None), "–")

    def test_duration(self):
        self.assertEqual(sysmon.duration(3 * 86400 + 4 * 3600 + 59), "3d 4h")
        self.assertEqual(sysmon.duration(5 * 3600 + 12 * 60 + 5), "5h 12m")
        self.assertEqual(sysmon.duration(59), "0m")


class ParseTest(unittest.TestCase):
    def test_proc_stat(self):
        busy1, total1 = sysmon.parse_proc_stat(PROC_STAT_1)
        busy2, total2 = sysmon.parse_proc_stat(PROC_STAT_2)
        self.assertEqual(total2 - total1, 400)
        self.assertEqual(busy2 - busy1, 100)
        b = sysmon.Busy()
        self.assertIsNone(b.percent(busy1, total1))
        self.assertEqual(b.percent(busy2, total2), 25.0)

    def test_meminfo(self):
        used, total = sysmon.parse_meminfo(MEMINFO)
        self.assertEqual(total, 16303428 * 1024)
        self.assertEqual(used, (16303428 - 9183676) * 1024)
        old = MEMINFO.replace("MemAvailable:    9183676 kB\n", "")
        self.assertEqual(sysmon.parse_meminfo(old)[0], (16303428 - 1214328 - 412700 - 7400128) * 1024)

    def test_net_dev_skips_loopback_and_virtual(self):
        self.assertEqual(sysmon.parse_proc_net_dev(NET_DEV), (1215645 + 5000, 1782404 + 7000))

    def test_vm_stat(self):
        used, total = sysmon.parse_vm_stat(VM_STAT, 17179869184)
        self.assertEqual(used, (316841 - 2861 + 122520 + 164123) * 16384)
        self.assertEqual(sysmon.size(used), "9.2 GB")
        older = VM_STAT.replace("Anonymous pages", "Something else")
        self.assertEqual(sysmon.parse_vm_stat(older, 17179869184)[0], (243163 + 122520 + 164123) * 16384)

    def test_netstat_ib(self):
        self.assertEqual(sysmon.parse_netstat_ib(NETSTAT_IB), (12345678901, 987654321))
        with self.assertRaises(ValueError):
            sysmon.parse_netstat_ib("garbage here\n")

    def test_top(self):
        self.assertAlmostEqual(sysmon.parse_top_cpu("CPU usage: 5.12% user, 8.33% sys, 86.54% idle \n"), 13.46)
        self.assertIsNone(sysmon.parse_top_cpu("Load Avg: 1.92, 2.03, 2.10"))

    def test_boottime_and_pmset(self):
        self.assertEqual(sysmon.parse_boottime(BOOTTIME, 1728600000 + 3600), 3600)
        self.assertEqual(sysmon.parse_pmset(PMSET), 85)
        self.assertIsNone(sysmon.parse_pmset(PMSET_DESKTOP))

    def test_nvidia(self):
        self.assertEqual(sysmon.parse_nvidia(NVIDIA), (23.0, 54.0, 25.0))
        self.assertEqual(sysmon.parse_nvidia("[N/A], 45, [N/A], 4096\n"), (None, 45.0, None))
        self.assertEqual(sysmon.parse_nvidia(""), (None, None, None))

    def test_gpu_stats_runs_nvidia_smi(self):
        with mock.patch.object(sysmon.subprocess, "run", return_value=done(NVIDIA)) as run:
            self.assertEqual(sysmon.gpu_stats("/usr/bin/nvidia-smi"), (23.0, 54.0, 25.0))
        self.assertEqual(
            run.call_args.args[0],
            ["/usr/bin/nvidia-smi", "--query-gpu=utilization.gpu,temperature.gpu,memory.used,memory.total", "--format=csv,noheader,nounits"],
        )
        self.assertEqual(sysmon.gpu_stats(None), (None, None, None))


class LinuxFixtureTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.proc = os.path.join(self.tmp.name, "proc")
        self.sys = os.path.join(self.tmp.name, "sys")

    def test_sampler_reads_proc(self):
        write(self.proc, "stat", PROC_STAT_1)
        write(self.proc, "meminfo", MEMINFO)
        write(self.proc, "net/dev", NET_DEV)
        write(self.proc, "uptime", "273618.53 1047581.22\n")
        s = sysmon.LinuxSampler(self.proc, self.sys)
        self.assertIsNone(s.cpu())
        write(self.proc, "stat", PROC_STAT_2)
        self.assertEqual(s.cpu(), 25.0)
        self.assertEqual(sysmon.duration(s.uptime()), "3d 4h")
        self.assertEqual(s.net(), (1220645, 1789404))
        self.assertIsNone(s.battery())
        self.assertIsNone(s.cpu_temp())

    def test_cpu_temp_prefers_the_package_sensor(self):
        write(self.sys, "class/hwmon/hwmon0/name", "nvme\n")
        write(self.sys, "class/hwmon/hwmon0/temp1_input", "38850\n")
        write(self.sys, "class/hwmon/hwmon3/name", "coretemp\n")
        write(self.sys, "class/hwmon/hwmon3/temp2_input", "52000\n")
        write(self.sys, "class/hwmon/hwmon3/temp2_label", "Core 0\n")
        write(self.sys, "class/hwmon/hwmon3/temp1_input", "61000\n")
        write(self.sys, "class/hwmon/hwmon3/temp1_label", "Package id 0\n")
        self.assertEqual(sysmon.linux_cpu_temp(self.sys), 61.0)

    def test_cpu_temp_amd_and_thermal_zone(self):
        write(self.sys, "class/hwmon/hwmon1/name", "k10temp\n")
        write(self.sys, "class/hwmon/hwmon1/temp1_input", "47375\n")
        write(self.sys, "class/hwmon/hwmon1/temp1_label", "Tctl\n")
        self.assertEqual(sysmon.linux_cpu_temp(self.sys), 47.375)
        other = os.path.join(self.tmp.name, "pi")
        write(other, "class/thermal/thermal_zone0/type", "acpitz\n")
        write(other, "class/thermal/thermal_zone0/temp", "27800\n")
        write(other, "class/thermal/thermal_zone1/type", "x86_pkg_temp\n")
        write(other, "class/thermal/thermal_zone1/temp", "55000\n")
        self.assertEqual(sysmon.linux_cpu_temp(other), 55.0)

    def test_battery(self):
        write(self.sys, "class/power_supply/AC/type", "Mains\n")
        write(self.sys, "class/power_supply/BAT0/type", "Battery\n")
        write(self.sys, "class/power_supply/BAT0/capacity", "80\n")
        write(self.sys, "class/power_supply/BAT1/type", "Battery\n")
        write(self.sys, "class/power_supply/BAT1/capacity", "60\n")
        write(self.sys, "class/power_supply/hidpp_battery_0/type", "Battery\n")
        write(self.sys, "class/power_supply/hidpp_battery_0/scope", "Device\n")
        write(self.sys, "class/power_supply/hidpp_battery_0/capacity", "5\n")
        self.assertEqual(sysmon.linux_battery(self.sys), 70.0)

    def test_disk(self):
        self.assertTrue(0 <= sysmon.disk_percent(self.tmp.name) <= 100)
        with self.assertRaises(OSError):
            sysmon.disk_percent(os.path.join(self.tmp.name, "missing"))


class FakeSampler(sysmon.Sampler):
    def __init__(self):
        super().__init__()
        self.rx = 1000
        self.mem_error = None

    def cpu(self):
        return 12.6

    def memory(self):
        if self.mem_error:
            raise self.mem_error
        return 8 * 1024 ** 3, 16 * 1024 ** 3

    def net(self):
        self.rx += 2 * 1024 ** 2
        return self.rx, 5000

    def uptime(self):
        return 3700

    def battery(self):
        return None


class MonitorTest(unittest.TestCase):
    def setUp(self):
        self.now = [100.0]
        self.sampler = FakeSampler()
        self.monitor = sysmon.Monitor(self.sampler, None, clock=lambda: self.now[0])
        self.monitor.primed = True
        patcher = mock.patch.object(sysmon.plugin, "set_value")
        self.set_value = patcher.start()
        self.addCleanup(patcher.stop)
        patcher = mock.patch.object(sysmon.plugin, "log")
        self.log = patcher.start()
        self.addCleanup(patcher.stop)
        sysmon.plugin.watching = set()
        self.addCleanup(setattr, sysmon.plugin, "watching", set())

    def values(self):
        out = {}
        for c in self.set_value.call_args_list:
            arg = c.kwargs.get("arg")
            out[c.args[0] if arg is None else f"{c.args[0]}:{arg}"] = c.args[1]
        return out

    def test_poll_publishes_everything_once(self):
        self.monitor.poll()
        self.now[0] += 2
        self.monitor.poll()
        v = self.values()
        self.assertEqual(v["cpu"], "13")
        self.assertEqual(v["ram"], "50")
        self.assertEqual(v["ram_used"], "8.0 GB")
        self.assertEqual(v["net_down"], "1.0 MB/s")  # 2 MB over 2 s
        self.assertEqual(v["net_up"], "0 B/s")
        self.assertEqual(v["uptime"], "1h 1m")
        for key in ("battery", "gpu", "gpu_temp", "vram", "cpu_temp"):
            self.assertEqual(v[key], "–")
        self.assertFalse(any(k.startswith("disk") for k in v))
        count = self.set_value.call_count
        self.now[0] += 2
        self.monitor.poll()
        self.assertEqual(self.set_value.call_count, count)  # nothing changed

    def test_disk_only_when_watched(self):
        sysmon.plugin.watching = {"disk:/", "disk:/nope/missing"}
        self.monitor.poll()
        v = self.values()
        self.assertRegex(v["disk:/"], r"^\d+$")
        self.assertEqual(v["disk:/nope/missing"], "–")
        sysmon.plugin.watching = set()
        with mock.patch.object(sysmon, "_monitor", self.monitor):
            sysmon.watching([])
        self.assertEqual([k for k in self.monitor.published if k.startswith("disk")], [])

    def test_failures_logged_once(self):
        self.sampler.mem_error = OSError("no /proc/meminfo")
        self.monitor.poll()
        self.monitor.poll()
        self.assertEqual(self.values()["ram"], "–")
        self.assertEqual(sum("memory" in c.args[0] for c in self.log.call_args_list), 1)
        self.sampler.mem_error = None
        self.monitor.poll()
        self.assertEqual(self.values()["ram"], "50")

    def test_gpu_from_nvidia_smi(self):
        self.monitor.nvidia_smi = "/usr/bin/nvidia-smi"
        with mock.patch.object(sysmon.subprocess, "run", return_value=done(NVIDIA)):
            self.monitor.poll()
        v = self.values()
        self.assertEqual((v["gpu"], v["gpu_temp"], v["vram"]), ("23", "54", "25"))

    def test_counter_reset(self):
        c = sysmon.Counter(clock=lambda: self.now[0])
        c.rate(1000)
        self.now[0] += 1
        self.assertIsNone(c.rate(10))  # an interface went away: no negative speed
        self.now[0] += 1
        self.assertEqual(c.rate(1034), 1024.0)

    def test_priming(self):
        m = sysmon.Monitor(FakeSampler(), None, clock=lambda: self.now[0])
        with mock.patch.object(sysmon.time, "sleep", side_effect=lambda s: self.now.__setitem__(0, self.now[0] + s)):
            m.poll()
        self.assertEqual(self.values()["net_down"], "4.0 MB/s")  # 2 MB in 0.5 s


class MacTest(unittest.TestCase):
    def test_sampler(self):
        answers = {
            ("sysctl", "-n", "hw.memsize"): "17179869184\n",
            ("vm_stat",): VM_STAT,
            ("netstat", "-ib"): NETSTAT_IB,
            ("sysctl", "-n", "kern.boottime"): BOOTTIME,
            ("pmset", "-g", "batt"): PMSET,
        }
        with mock.patch.object(sysmon.subprocess, "run", side_effect=lambda a, **k: done(answers[tuple(a)])):
            s = sysmon.MacSampler()
            self.assertEqual(sysmon.size(s.memory()[0]), "9.2 GB")
            self.assertEqual(s.net(), (12345678901, 987654321))
            self.assertEqual(s.battery(), 85)
            with mock.patch.object(sysmon.time, "time", return_value=1728600000 + 7200):
                self.assertEqual(s.uptime(), 7200)

    def test_top_reader_skips_the_first_sample(self):
        lines = [
            "Processes: 512 total, 3 running\n",
            "CPU usage: 50.0% user, 50.0% sys, 0.0% idle \n",
            "PhysMem: 15G used (2G wired), 300M unused.\n",
            "CPU usage: 10.0% user, 5.0% sys, 85.0% idle \n",
        ]
        proc = mock.Mock(stdout=iter(lines))
        proc.poll.return_value = None
        with mock.patch.object(sysmon.subprocess, "Popen", return_value=proc) as popen, mock.patch.object(
            sysmon.threading, "Thread"
        ):
            top = sysmon.TopReader()
            self.assertIsNone(top.cpu())  # starts top
        self.assertEqual(popen.call_args.args[0], ["top", "-l", "0", "-s", "2", "-n", "0"])
        top.read(proc)
        self.assertEqual(top.cpu(), 15.0)
        top.close()
        proc.terminate.assert_called_once()


class WindowsTest(unittest.TestCase):
    def test_struct_sizes(self):
        self.assertEqual(ctypes.sizeof(winapi.MEMORYSTATUSEX), 64)
        self.assertEqual(ctypes.sizeof(winapi.SYSTEM_POWER_STATUS), 12)
        ft = winapi.FILETIME(5, 2)
        self.assertEqual(ft.value(), (2 << 32) | 5)

    def test_battery_flags(self):
        status = winapi.SYSTEM_POWER_STATUS(BatteryFlag=8, BatteryLifePercent=77)  # charging
        self.assertEqual(winapi.battery_percent(status), 77)
        self.assertIsNone(winapi.battery_percent(winapi.SYSTEM_POWER_STATUS(BatteryFlag=128, BatteryLifePercent=255)))
        self.assertIsNone(winapi.battery_percent(winapi.SYSTEM_POWER_STATUS(BatteryFlag=255, BatteryLifePercent=50)))

    def test_sampler(self):
        api = mock.Mock()
        api.system_times.side_effect = [(1000, 1500, 500), (1300, 1900, 500)]  # +300 idle of +400
        api.memory.return_value = (16 * 1024 ** 3, 4 * 1024 ** 3)
        api.uptime_seconds.return_value = 90061.0
        api.battery_percent.return_value = None
        api.network_bytes.return_value = (3123456789, 456789012)
        s = sysmon.WindowsSampler.__new__(sysmon.WindowsSampler)
        sysmon.Sampler.__init__(s)
        s.api = api
        self.assertIsNone(s.cpu())
        self.assertEqual(s.cpu(), 25.0)
        self.assertEqual(s.memory(), (12 * 1024 ** 3, 16 * 1024 ** 3))
        self.assertEqual(sysmon.duration(s.uptime()), "1d 1h")
        with mock.patch.object(sysmon.subprocess, "run") as run:
            self.assertEqual(s.net(), (3123456789, 456789012))
        run.assert_not_called()  # no netstat every two seconds

    def test_interface_table_layout(self):
        # The size and offsets of the Windows SDK's MIB_IF_ROW2 (netioapi.h).
        self.assertEqual(ctypes.sizeof(winapi.MIB_IF_ROW2), 1352)
        self.assertEqual(winapi.MIB_IF_ROW2.InterfaceAndOperStatusFlags.offset, 1152)
        self.assertEqual(winapi.MIB_IF_ROW2.InOctets.offset, 1208)
        self.assertEqual(winapi.MIB_IF_ROW2.OutOctets.offset, 1280)
        self.assertEqual(winapi.MIB_IF_TABLE2.Table.offset, 8)

    def test_only_physical_adapters_count(self):
        def row(flags, received, sent):
            return winapi.MIB_IF_ROW2(InterfaceAndOperStatusFlags=flags, InOctets=received, OutOctets=sent)

        hardware, connector, ndis_filter = winapi.IF_HARDWARE, winapi.IF_CONNECTOR, winapi.IF_FILTER
        rows = [
            row(hardware | connector, 2**40, 5000),  # the Ethernet adapter, past 32 bits
            row(ndis_filter, 2**40, 5000),  # it again, as the QoS Packet Scheduler sees it
            row(ndis_filter, 2**40, 5000),  # and a WFP layer
            row(0, 700, 800),  # vEthernet (Default Switch): its traffic crosses the adapter too
            row(hardware, 9, 9),  # the KM-TEST loopback adapter: "hardware", but no connector
            row(hardware | connector | 0x10, 300, 400),  # Wi-Fi with no network: still counts
        ]
        self.assertEqual(winapi.traffic(rows), (2**40 + 300, 5400))
        self.assertEqual(winapi.traffic([]), (0, 0))

    @unittest.skipUnless(sys.platform == "win32", "reads this computer's network adapters")
    def test_network_bytes_on_this_computer(self):
        first = winapi.network_bytes()
        second = winapi.network_bytes()
        self.assertTrue(all(isinstance(n, int) and n >= 0 for n in first))
        self.assertGreaterEqual(second[0], first[0])
        self.assertGreaterEqual(second[1], first[1])


class ActionTest(unittest.TestCase):
    def test_monitor_command(self):
        self.assertEqual(sysmon.monitor_command("win32"), ["taskmgr.exe"])
        self.assertEqual(sysmon.monitor_command("darwin"), ["open", "-a", "Activity Monitor"])
        which = lambda n: "/usr/bin/" + n if n in ("ksysguard", "lxtask") else None  # noqa: E731
        self.assertEqual(sysmon.monitor_command("linux", which), ["/usr/bin/ksysguard"])
        self.assertIsNone(sysmon.monitor_command("linux", lambda n: None))

    def test_open(self):
        # Started for the user, so the monitor stays open when the plugin restarts.
        with mock.patch.object(sysmon.sys, "platform", "linux"), mock.patch.object(
            sysmon, "monitor_command", return_value=["/usr/bin/gnome-system-monitor"]
        ), mock.patch.object(sysmon, "start_for_user") as start:
            sysmon.open_monitor({})
        start.assert_called_once_with(["/usr/bin/gnome-system-monitor"])
        # Task Manager through ShellExecute, which can ask to run it as administrator.
        with mock.patch.object(sysmon.sys, "platform", "win32"), mock.patch.object(sysmon, "open_for_user") as opened:
            sysmon.open_monitor({})
        opened.assert_called_once_with("taskmgr.exe")
        with mock.patch.object(sysmon, "monitor_command", return_value=None):
            with self.assertRaisesRegex(RuntimeError, "no system monitor"):
                sysmon.open_monitor({})


if __name__ == "__main__":
    unittest.main()
