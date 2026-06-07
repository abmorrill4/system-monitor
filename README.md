# system-monitor

A Claude plugin (Cowork + Claude Code) that monitors your computer's health and
hardware locally and reports it in plain language. Successor to `drive-health` —
SMART drive health is now one tier of a broader monitor.

All data stays on your machine.

## What Claude can do

- "How's my system?" -> a prioritized **health report** (OK / WARNING / CRITICAL)
- CPU / memory / disk / network / process metrics
- **GPU** metrics (NVIDIA): utilization, VRAM, temp, power, clocks
- **Sensors**: CPU/board temps, fan RPM, voltages (via LibreHardwareMonitor)
- **Drive health** (SMART) with verdicts + self-tests

## Tools

| Tool | Purpose |
|------|---------|
| `get_health_report` | Prioritized roll-up across drives, disk space, memory, temps |
| `get_system_snapshot` | System info + CPU + memory + disks + network + top procs + GPU |
| `get_system_info` | OS, host, uptime, motherboard, BIOS, CPU model |
| `get_cpu` | Load (overall/per-core), frequency, temperature |
| `get_memory` | RAM + swap usage |
| `get_disks` | Disk space per volume |
| `get_disk_io` | Disk read/write throughput |
| `get_network` | Interfaces, link state, throughput |
| `get_top_processes` | Top processes by CPU or memory |
| `get_gpu` | NVIDIA GPU metrics via nvidia-smi |
| `get_sensors` | Temps/fans/voltages via LibreHardwareMonitor |
| `get_drive_health` | SMART health + verdict |
| `run_self_test` | Start a SMART self-test |

## Data sources & requirements

- **General metrics**: the bundled [`systeminformation`](https://github.com/sebhildebrandt/systeminformation) library. No setup, no elevation.
- **GPU**: `nvidia-smi` (ships with the NVIDIA driver). NVIDIA only.
- **Sensors**: [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor) running as Administrator, with its web server enabled (Options -> Remote Web Server, default port 8085) or accessible via WMI. Set `LHM_URL` to override the web server address.
- **Drive health**: `smartctl` (`winget install smartmontools.smartmontools`). Auto-detected in `C:\Program Files\smartmontools\bin`; override with `SMARTCTL_PATH`.

## IMPORTANT: Administrator privileges

On Windows, reading **SMART** and **sensor** data requires Administrator. The
server inherits the launching app's privileges, so run the Claude app elevated
for full readings. Everything degrades gracefully when a source is unavailable.

## Development

```bash
npm install      # fetch systeminformation
npm test         # node:test: pure parsers + health logic + protocol/snapshot
npm run build    # produce dist/system-monitor.plugin (bundles node_modules)
```
