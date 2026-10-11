"""System monitor: CPU, memory, disks, network, GPU, temperatures, uptime and battery
as live values for key titles and sliders, and a key that opens the system monitor.

Everything is read every 2 seconds; rates (CPU, network) come from the difference
between two readings. Numbers are plain strings ("23"), so sliders can follow the
percentages. Unknown values are "–".

Where the numbers come from:

- Linux: /proc (stat, meminfo, net/dev, uptime) and /sys (temperatures, battery).
- macOS: top (one long-running sampler, for CPU), vm_stat and sysctl (memory,
  uptime), netstat -ib (network), pmset -g batt (battery).
- Windows: kernel32 through ctypes (GetSystemTimes, GlobalMemoryStatusEx,
  GetSystemPowerStatus, GetTickCount64; see winapi.py) and netstat -e (network).
- Everywhere: shutil.disk_usage for disks, nvidia-smi for NVIDIA GPUs.
"""

import glob
import os
import re
import shutil
import subprocess
import sys
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()
NOTHING = "–"
POLL_SECONDS = 2
TIMEOUT = 4  # seconds for any one helper program
CREATE_NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0x08000000)

# Interfaces left out of network totals: loopback, and virtual ones whose traffic
# also crosses a real interface (containers, bridges, VMs, VPN tunnels).
VIRTUAL_NICS = ("lo", "veth", "docker", "br-", "virbr", "vnet", "tun", "tap", "wg", "tailscale", "zt", "utun", "awdl", "llw", "bridge", "gif", "stf", "anpi", "ap")


def run(args, timeout=TIMEOUT):
    out = subprocess.run(
        args,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout,
        creationflags=CREATE_NO_WINDOW if sys.platform == "win32" else 0,
    )
    if out.returncode != 0:
        detail = (out.stderr or "").strip().splitlines()
        raise RuntimeError(detail[-1] if detail else f"{args[0]} failed")
    return out.stdout


# --- Formatting -------------------------------------------------------------------


def pct(value):
    if value is None:
        return NOTHING
    return str(int(round(min(max(value, 0.0), 100.0))))


def size(n, per_second=False):
    """1536 → "1.5 KB"; 1024-based, like Task Manager and Activity Monitor."""
    if n is None:
        return NOTHING
    suffix = "/s" if per_second else ""
    units = ("B", "KB", "MB", "GB", "TB")
    value = float(max(n, 0))
    i = 0
    while value >= 1024 and i < len(units) - 1:
        value /= 1024
        i += 1
    if i <= 1:  # bytes and kilobytes: whole numbers
        return f"{int(round(value))} {units[i]}{suffix}"
    return f"{value:.1f} {units[i]}{suffix}"


def duration(seconds):
    """Uptime as "3d 4h", "5h 12m" or "12m"."""
    if seconds is None:
        return NOTHING
    minutes = int(seconds // 60)
    days, minutes = divmod(minutes, 24 * 60)
    hours, minutes = divmod(minutes, 60)
    if days:
        return f"{days}d {hours}h"
    if hours:
        return f"{hours}h {minutes}m"
    return f"{minutes}m"


def number(value):
    return NOTHING if value is None else str(int(round(value)))


# --- Rates -------------------------------------------------------------------------


class Counter:
    """Turns a counter that only grows (bytes, CPU ticks) into a rate per second."""

    def __init__(self, clock=time.monotonic):
        self.clock = clock
        self.last = None

    def rate(self, value):
        now = self.clock()
        last, self.last = self.last, (now, value)
        if last is None or now <= last[0] or value < last[1]:
            return None  # first reading, or the counter was reset
        return (value - last[1]) / (now - last[0])


class Busy:
    """CPU busy percent from cumulative (busy, total) times."""

    def __init__(self):
        self.last = None

    def percent(self, busy, total):
        last, self.last = self.last, (busy, total)
        if last is None or total <= last[1]:
            return None
        return 100.0 * (busy - last[0]) / (total - last[1])


# --- Parsers (pure, tested with real sample output) ----------------------------------


def parse_proc_stat(text):
    """(busy, total) ticks from /proc/stat's first line. Guest time is already in
    user and nice, so only the first eight columns count."""
    for line in text.splitlines():
        if line.startswith("cpu "):
            fields = [int(x) for x in line.split()[1:9]]
            fields += [0] * (8 - len(fields))
            user, nice, system, idle, iowait, irq, softirq, steal = fields
            total = sum(fields)
            return total - idle - iowait, total
    raise ValueError("no cpu line in /proc/stat")


def parse_meminfo(text):
    """(used, total) bytes from /proc/meminfo: total minus MemAvailable."""
    info = {}
    for line in text.splitlines():
        key, _, rest = line.partition(":")
        parts = rest.split()
        if parts and parts[0].isdigit():
            info[key.strip()] = int(parts[0]) * 1024
    total = info["MemTotal"]
    available = info.get("MemAvailable")
    if available is None:  # kernels before 3.14
        available = info.get("MemFree", 0) + info.get("Buffers", 0) + info.get("Cached", 0)
    return total - available, total


def real_nic(name):
    return not name.lower().startswith(VIRTUAL_NICS)


def parse_proc_net_dev(text):
    """(received, sent) bytes over real interfaces from /proc/net/dev."""
    rx = tx = 0
    for line in text.splitlines()[2:]:
        name, _, data = line.partition(":")
        fields = data.split()
        if len(fields) < 9 or not real_nic(name.strip()):
            continue
        rx += int(fields[0])
        tx += int(fields[8])
    return rx, tx


def parse_vm_stat(text, memsize):
    """(used, total) bytes the way Activity Monitor counts "Memory Used": app memory
    (anonymous minus purgeable pages) + wired + compressed."""
    m = re.search(r"page size of (\d+) bytes", text)
    page = int(m.group(1)) if m else 4096
    pages = {}
    for line in text.splitlines():
        key, _, value = line.partition(":")
        value = value.strip().rstrip(".")
        if value.isdigit():
            pages[key.strip().strip('"').lower()] = int(value)
    wired = pages.get("pages wired down", 0)
    compressed = pages.get("pages occupied by compressor", 0)
    if "anonymous pages" in pages:
        app = pages["anonymous pages"] - pages.get("pages purgeable", 0)
    else:
        app = pages.get("pages active", 0)
    used = (max(app, 0) + wired + compressed) * page
    return min(used, memsize), memsize


def parse_netstat_ib(text):
    """(received, sent) bytes from macOS `netstat -ib`, one <Link#> row per
    interface. Rows can lack the Address column, so columns are found from the
    right, where the counters always are."""
    lines = text.splitlines()
    if not lines:
        return 0, 0
    header = lines[0].split()
    try:
        ib = len(header) - header.index("Ibytes")
        ob = len(header) - header.index("Obytes")
    except ValueError:
        raise ValueError("unexpected netstat -ib output")
    rx = tx = 0
    for line in lines[1:]:
        parts = line.split()
        if len(parts) < max(ib, ob) + 1 or "<Link#" not in line:
            continue
        name = parts[0].rstrip("*")
        if not real_nic(name):
            continue
        try:
            rx += int(parts[-ib])
            tx += int(parts[-ob])
        except ValueError:
            continue
    return rx, tx


def parse_netstat_e(text):
    """(received, sent) bytes from Windows `netstat -e`: the first row with two
    numbers is Bytes (its label is translated, so it isn't matched by name)."""
    for line in text.splitlines():
        numbers = re.findall(r"\d+", line)
        if len(numbers) == 2:
            return int(numbers[0]), int(numbers[1])
    raise ValueError("unexpected netstat -e output")


def parse_top_cpu(line):
    """Busy percent from a macOS top line "CPU usage: 5.12% user, 8.33% sys, 86.54% idle"."""
    m = re.search(r"([\d.]+)%\s*idle", line)
    return None if not m else 100.0 - float(m.group(1))


def parse_boottime(text, now):
    """Seconds since boot from `sysctl -n kern.boottime`: "{ sec = 1700000000, usec = 0 } …"."""
    m = re.search(r"sec\s*=\s*(\d+)", text)
    return None if not m else max(now - int(m.group(1)), 0)


def parse_pmset(text):
    """Battery percent from `pmset -g batt`, or None on a Mac without one."""
    m = re.search(r"(\d+)%", text)
    return int(m.group(1)) if m and "InternalBattery" in text else None


def parse_nvidia(text):
    """(gpu %, temperature, vram %) for the first GPU from
    nvidia-smi --query-gpu=utilization.gpu,temperature.gpu,memory.used,memory.total
    --format=csv,noheader,nounits. "[N/A]" fields are None."""

    def num(s):
        try:
            return float(s.strip())
        except ValueError:
            return None

    for line in text.splitlines():
        fields = [num(f) for f in line.split(",")]
        if len(fields) >= 4:
            util, temp, used, total = fields[:4]
            vram = 100.0 * used / total if used is not None and total else None
            return util, temp, vram
    return None, None, None


# --- Linux sensors ---------------------------------------------------------------------

CPU_HWMON = ("coretemp", "k10temp", "zenpower", "cpu_thermal", "cpu-thermal", "soc_thermal", "apple_soc")
PREFERRED_LABELS = ("package id 0", "tdie", "tctl", "cpu")
CPU_ZONES = ("x86_pkg_temp", "cpu-thermal", "cpu_thermal", "soc_thermal", "cpu", "acpitz")


def read(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read().strip()


def linux_cpu_temp(sys_root="/sys"):
    """CPU temperature in °C from hwmon (coretemp, k10temp…) or a thermal zone."""
    for name in CPU_HWMON:
        for hw in sorted(glob.glob(os.path.join(sys_root, "class", "hwmon", "hwmon*"))):
            try:
                if read(os.path.join(hw, "name")) != name:
                    continue
            except OSError:
                continue
            inputs = sorted(glob.glob(os.path.join(hw, "temp*_input")))
            labelled = {}
            for path in inputs:
                try:
                    labelled[read(path.replace("_input", "_label")).lower()] = path
                except OSError:
                    pass
            choice = next((labelled[label] for label in PREFERRED_LABELS if label in labelled), None)
            for path in [choice] + inputs if choice else inputs:
                try:
                    return int(read(path)) / 1000.0
                except (OSError, ValueError):
                    continue
    zones = sorted(glob.glob(os.path.join(sys_root, "class", "thermal", "thermal_zone*")))
    for kind in CPU_ZONES:
        for zone in zones:
            try:
                if read(os.path.join(zone, "type")).lower() == kind:
                    return int(read(os.path.join(zone, "temp"))) / 1000.0
            except (OSError, ValueError):
                continue
    return None


def linux_battery(sys_root="/sys"):
    """Average charge of the batteries in /sys/class/power_supply, or None."""
    values = []
    for supply in sorted(glob.glob(os.path.join(sys_root, "class", "power_supply", "*"))):
        try:
            if read(os.path.join(supply, "type")) != "Battery":
                continue
            if read(os.path.join(supply, "scope")).lower() == "device":
                continue  # a mouse or headset, not the computer
        except OSError:
            pass
        try:
            values.append(int(read(os.path.join(supply, "capacity"))))
        except (OSError, ValueError):
            continue
    return sum(values) / len(values) if values else None


# --- Samplers: one per platform ----------------------------------------------------------


class Sampler:
    """cpu() → percent; memory() → (used, total); net() → (rx, tx) bytes so far;
    uptime() → seconds; battery() → percent; cpu_temp() → °C. None when unknown."""

    def __init__(self):
        self.busy = Busy()

    def cpu(self):
        return None

    def cpu_temp(self):
        return None

    def battery(self):
        return None

    def close(self):
        pass


class LinuxSampler(Sampler):
    def __init__(self, proc="/proc", sys_root="/sys"):
        super().__init__()
        self.proc, self.sys = proc, sys_root

    def cpu(self):
        return self.busy.percent(*parse_proc_stat(read(os.path.join(self.proc, "stat"))))

    def memory(self):
        return parse_meminfo(read(os.path.join(self.proc, "meminfo")))

    def net(self):
        return parse_proc_net_dev(read(os.path.join(self.proc, "net", "dev")))

    def uptime(self):
        return float(read(os.path.join(self.proc, "uptime")).split()[0])

    def battery(self):
        return linux_battery(self.sys)

    def cpu_temp(self):
        return linux_cpu_temp(self.sys)


class TopReader:
    """macOS CPU from one long-running `top -l 0 -s 2 -n 0` (no process list): far
    cheaper than starting top every poll, and top's first sample is unreliable."""

    def __init__(self):
        self.proc = None
        self.value = None
        self.samples = 0
        self.lock = threading.Lock()

    def start(self):
        self.proc = subprocess.Popen(
            ["top", "-l", "0", "-s", str(POLL_SECONDS), "-n", "0"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        threading.Thread(target=self.read, args=(self.proc,), daemon=True).start()

    def read(self, proc):
        for line in proc.stdout:
            if line.startswith("CPU usage"):
                value = parse_top_cpu(line)
                with self.lock:
                    self.samples += 1
                    if self.samples > 1:
                        self.value = value

    def cpu(self):
        with self.lock:
            if self.proc is None or self.proc.poll() is not None:
                self.samples, self.value = 0, None
                self.start()
            return self.value

    def close(self):
        if self.proc is not None and self.proc.poll() is None:
            self.proc.terminate()


class MacSampler(Sampler):
    def __init__(self):
        super().__init__()
        self.top = TopReader()
        self.memsize = None

    def cpu(self):
        return self.top.cpu()

    def memory(self):
        if self.memsize is None:
            self.memsize = int(run(["sysctl", "-n", "hw.memsize"]).strip())
        return parse_vm_stat(run(["vm_stat"]), self.memsize)

    def net(self):
        return parse_netstat_ib(run(["netstat", "-ib"]))

    def uptime(self):
        return parse_boottime(run(["sysctl", "-n", "kern.boottime"]), time.time())

    def battery(self):
        return parse_pmset(run(["pmset", "-g", "batt"]))

    def close(self):
        self.top.close()


class WindowsSampler(Sampler):
    def __init__(self):
        super().__init__()
        import winapi  # ctypes.WinDLL: Windows only

        self.api = winapi

    def cpu(self):
        idle, kernel, user = self.api.system_times()
        total = kernel + user  # kernel time includes idle time
        return self.busy.percent(total - idle, total)

    def memory(self):
        total, available = self.api.memory()
        return total - available, total

    def net(self):
        return parse_netstat_e(run(["netstat", "-e"]))

    def uptime(self):
        return self.api.uptime_seconds()

    def battery(self):
        return self.api.battery_percent()


def make_sampler(platform=None):
    platform = platform or sys.platform
    if platform == "win32":
        return WindowsSampler()
    if platform == "darwin":
        return MacSampler()
    return LinuxSampler()


NVIDIA_PATHS = (
    r"C:\Windows\System32\nvidia-smi.exe",
    r"C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe",
)


def find_nvidia_smi():
    found = shutil.which("nvidia-smi")
    if found or sys.platform != "win32":
        return found
    return next((p for p in NVIDIA_PATHS if os.path.exists(p)), None)


def gpu_stats(nvidia_smi):
    if not nvidia_smi:
        return None, None, None
    out = run([nvidia_smi, "--query-gpu=utilization.gpu,temperature.gpu,memory.used,memory.total", "--format=csv,noheader,nounits"])
    return parse_nvidia(out)


def disk_percent(path):
    path = os.path.expanduser(path.strip())
    if sys.platform == "win32" and re.fullmatch(r"[A-Za-z]:", path):
        path += "\\"
    usage = shutil.disk_usage(path)
    # Like df: used out of what's usable (total minus space reserved for root).
    usable = usage.used + usage.free
    return 100.0 * usage.used / usable if usable else None


# --- Polling ---------------------------------------------------------------------------


class Monitor:
    def __init__(self, sampler, nvidia_smi=None, clock=time.monotonic):
        self.sampler = sampler
        self.nvidia_smi = nvidia_smi
        self.rx = Counter(clock)
        self.tx = Counter(clock)
        self.published = {}
        self.failing = set()
        self.primed = False

    def attempt(self, name, fn, *args):
        """fn's result, or None if it failed (logged once until it works again)."""
        try:
            value = fn(*args)
        except Exception as e:
            if name not in self.failing:
                self.failing.add(name)
                plugin.log(f"couldn't read {name}: {e}", "warn")
            return None
        self.failing.discard(name)
        return value

    def publish(self, value_id, value, arg=None):
        key = value_id if arg is None else f"{value_id}:{arg}"
        if self.published.get(key) != value:
            self.published[key] = value
            plugin.set_value(value_id, value, arg=arg)

    def prime(self):
        """Take a first reading of the counters so the first poll has rates."""
        self.attempt("CPU", self.sampler.cpu)
        net = self.attempt("network", self.sampler.net)
        if net:
            self.rx.rate(net[0])
            self.tx.rate(net[1])
        self.primed = True

    def poll(self):
        if not self.primed:
            self.prime()
            time.sleep(0.5)
        s = self.sampler
        self.publish("cpu", pct(self.attempt("CPU", s.cpu)))
        mem = self.attempt("memory", s.memory)
        self.publish("ram", pct(100.0 * mem[0] / mem[1]) if mem and mem[1] else NOTHING)
        self.publish("ram_used", size(mem[0]) if mem else NOTHING)
        net = self.attempt("network", s.net)
        down = self.rx.rate(net[0]) if net else None
        up = self.tx.rate(net[1]) if net else None
        self.publish("net_down", size(down, per_second=True))
        self.publish("net_up", size(up, per_second=True))
        self.publish("uptime", duration(self.attempt("uptime", s.uptime)))
        self.publish("battery", pct(self.attempt("battery", s.battery)))
        self.publish("cpu_temp", number(self.attempt("CPU temperature", s.cpu_temp)))
        gpu = self.attempt("the GPU (nvidia-smi)", gpu_stats, self.nvidia_smi) or (None, None, None)
        self.publish("gpu", pct(gpu[0]))
        self.publish("gpu_temp", number(gpu[1]))
        self.publish("vram", pct(gpu[2]))
        for path in plugin.watched("disk"):
            self.publish("disk", pct(self.attempt(f"disk {path}", disk_percent, path)), arg=path)

    def forget_unwatched(self):
        watched = {f"disk:{p}" for p in plugin.watched("disk")}
        for key in [k for k in self.published if k.startswith("disk:") and k not in watched]:
            del self.published[key]


_monitor = None
_monitor_lock = threading.Lock()


def monitor():
    global _monitor
    with _monitor_lock:
        if _monitor is None:
            _monitor = Monitor(make_sampler(), find_nvidia_smi())
        return _monitor


@plugin.every(POLL_SECONDS)
def poll():
    monitor().poll()


@plugin.on_watch
def watching(_values):
    monitor().forget_unwatched()


# --- Action ------------------------------------------------------------------------------

LINUX_MONITORS = (
    "gnome-system-monitor",
    "plasma-systemmonitor",
    "ksysguard",
    "xfce4-taskmanager",
    "mate-system-monitor",
    "lxtask",
    "qps",
)


def monitor_command(platform=None, which=shutil.which):
    platform = platform or sys.platform
    if platform == "win32":
        return ["taskmgr.exe"]
    if platform == "darwin":
        return ["open", "-a", "Activity Monitor"]
    for name in LINUX_MONITORS:
        path = which(name)
        if path:
            return [path]
    return None


@plugin.action("open")
def open_monitor(settings):
    command = monitor_command()
    if not command:
        raise RuntimeError("no system monitor app found (e.g. install gnome-system-monitor)")
    kwargs = dict(stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, close_fds=True)
    if sys.platform == "win32":
        # Task Manager asks to run as administrator; ShellExecute shows that prompt,
        # CreateProcess would just fail.
        os.startfile(command[0])  # noqa: S606
        return
    kwargs["start_new_session"] = True
    subprocess.Popen(command, **kwargs)


if __name__ == "__main__":
    plugin.run()
