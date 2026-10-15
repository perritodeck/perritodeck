# System monitor

Shows how busy the computer is, on key titles and sliders: CPU, memory, disks, network,
NVIDIA GPU, temperatures, uptime and battery. Updated every 2 seconds. A key can also
open the system monitor app.

## Setup

Nothing to fill in or install. GPU values need an NVIDIA card and its driver
(`nvidia-smi`, which comes with it); other GPUs show `–`.

## Values

Percentages are plain numbers (`23`), so a slider can follow them and a title can add
the `%`: `CPU {plugin:dev.perritodeck.sysmon:cpu}%`.

| Title | Shows |
|---|---|
| `{plugin:dev.perritodeck.sysmon:cpu}` | CPU in use, % |
| `{plugin:dev.perritodeck.sysmon:ram}` | memory in use, % |
| `{plugin:dev.perritodeck.sysmon:ram_used}` | memory in use, e.g. `11.2 GB` |
| `{plugin:dev.perritodeck.sysmon:disk:C:}` or `…:disk:/home` | how full that drive is, % |
| `{plugin:dev.perritodeck.sysmon:net_down}`, `…:net_up` | download and upload speed, e.g. `1.4 MB/s` |
| `{plugin:dev.perritodeck.sysmon:gpu}` | NVIDIA GPU in use, % (the first GPU) |
| `{plugin:dev.perritodeck.sysmon:gpu_temp}` | NVIDIA GPU temperature, °C |
| `{plugin:dev.perritodeck.sysmon:vram}` | NVIDIA GPU memory in use, % |
| `{plugin:dev.perritodeck.sysmon:cpu_temp}` | CPU temperature, °C (Linux only; `–` elsewhere) |
| `{plugin:dev.perritodeck.sysmon:uptime}` | time since start-up, e.g. `3d 4h` |
| `{plugin:dev.perritodeck.sysmon:battery}` | battery charge, %; `–` without a battery |

Sizes count in 1024s, like Task Manager and Activity Monitor. Network speeds add up the
real network adapters and leave out loopback and virtual ones (Docker, VM bridges, VPN
tunnels), whose traffic would otherwise be counted twice. On Linux and macOS "memory in
use" is what apps really hold (Linux: total minus *available*; macOS: app memory, wired
and compressed, like Activity Monitor), not counting caches.

The host tells plugins which disks keys show, so only those are checked. It doesn't say
which of the other values are shown, so those are all read every 2 seconds; all of them
are cheap except `nvidia-smi`, which is only run if it's installed.

## Actions

| Action | Opens |
|---|---|
| Open the system monitor | Task Manager (Windows), Activity Monitor (macOS), or the first of GNOME System Monitor, Plasma System Monitor, KSysGuard, Xfce Task Manager, MATE System Monitor, LXTask or qps (Linux) |

## How it works

- Linux: `/proc/stat`, `/proc/meminfo`, `/proc/net/dev`, `/proc/uptime`; CPU
  temperature from `/sys/class/hwmon` (coretemp "Package id 0", k10temp "Tdie"/"Tctl",
  zenpower, Raspberry Pi `cpu_thermal`) or else `/sys/class/thermal`; the battery from
  `/sys/class/power_supply` (mice and headsets are left out).
- macOS: one long-running `top -l 0 -s 2 -n 0` for the CPU, `vm_stat` and
  `sysctl hw.memsize` for memory, `netstat -ib`, `sysctl kern.boottime`, `pmset -g batt`.
- Windows: `kernel32` and `iphlpapi` through ctypes ([winapi.py](winapi.py)):
  GetSystemTimes, GlobalMemoryStatusEx, GetSystemPowerStatus, GetTickCount64, and
  GetIfTable2 for the network: the 64-bit counters of the physical adapters (the ones
  `Get-NetAdapter -Physical` lists). `netstat -e` isn't used: it adds up every row of
  that table, including each adapter's NDIS filter rows (QoS, WFP, Npcap) and Hyper-V's
  vEthernet, which counts each byte four to six times, and it wraps at 4 GB.
- Disks: Python's `shutil.disk_usage` (GetDiskFreeSpaceExW on Windows).

## Verified vs untested

- **Verified** on Linux (a container, over the real plugin protocol): CPU, memory,
  network, uptime and disk values. Battery, CPU temperature and GPU showed `–`, as they
  should there.
- **Unit-tested with real sample output only**: `/proc` and `/sys` files (written to a
  temporary folder), `vm_stat`, `netstat -ib`, `top`, `pmset -g batt`, `kern.boottime`,
  and `nvidia-smi --query-gpu … --format=csv`.
- **Run on Windows 11** (2026-10-12, in the desktop app with a tablet): the values showed
  on keys, and the network totals and speeds matched `Get-NetAdapterStatistics` for the
  physical adapters. The other Windows values weren't compared with Windows' own
  figures. The test suite reads the real interface table on Windows; the other ctypes
  structures are checked for size there and the calls are mocked.
- **Not run on real systems**: macOS, and a real NVIDIA GPU on any system.
