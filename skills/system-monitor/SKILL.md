---
name: system-monitor
description: Monitor local computer health and hardware. Use when the user asks to "check my system", "how's my PC doing", "system health", "check my drives", "is my SSD dying", "CPU/GPU temperature", "how much RAM/disk is free", "what's using my CPU", "top processes", "network usage", "fan speeds", "run a health check", or asks about the condition of any hardware component. Backed by the system-monitor MCP server (sysinfo, smartctl, nvidia-smi, LibreHardwareMonitor).
---

# System Monitor

Report local system and hardware status using the `system-monitor` MCP server. Lead with the answer and a verdict; show only the numbers that matter. Do not dump raw JSON at the user.

## Choosing a tool

- Broad "how's my system" question -> `get_health_report` first (prioritized issues + overall verdict), optionally `get_system_snapshot` for the raw numbers.
- Specific subsystem -> the matching tool: `get_cpu`, `get_memory`, `get_disks`, `get_disk_io`, `get_network`, `get_top_processes`, `get_gpu`, `get_sensors`.
- Drive condition -> `get_drive_health` (SMART verdict); `run_self_test` to start a self-test.
- Overview/specs -> `get_system_info`.

## get_health_report (flagship)

Returns `overall` (OK / WARNING / CRITICAL), a `summary`, a prioritized `issues` array (each with severity, area, detail, recommendation), and a `sections` map showing which tiers were readable. When presenting:

- State the overall verdict first.
- List CRITICAL issues, then WARNINGs, each with its recommendation.
- If a section is unavailable, mention it briefly only if relevant (e.g. "temps need LibreHardwareMonitor running").

## Interpreting verdicts and thresholds

- Drive SMART: `HEALTHY` / `WARNING` (non-zero pending/reallocated sectors, high wear) / `FAILING` (back up now, replace) / `NEEDS_ELEVATION` (run as Admin).
- Temps: CPU/GPU warn at 85C, critical at 95C; drives warn at 60C, critical at 70C.
- Disk space: warn under 10% free or under 20 GB free.
- Memory: warn over 90% used.

## Data sources and their limits

- **General metrics** (cpu, memory, disks, network, processes, host) come from the `sysinfo` crate and work without elevation.
- **CPU/board temps and fans** require **LibreHardwareMonitor** running (web server on port 8085, or WMI). If `get_sensors` is unavailable, fall back to GPU temp (nvidia-smi) and drive temps (smartctl), and tell the user how to enable LHM.
- **GPU** uses `nvidia-smi` (NVIDIA only); AMD/Intel return an unavailable note.
- **Drive health** uses `smartctl` and needs **Administrator** privileges on Windows.

## Privileges

SMART reads and LibreHardwareMonitor sensor reads need the launching app to run as **Administrator** on Windows. If drives scan empty or sensors are unavailable, suggest relaunching elevated.

## Mapping drives to letters

smartctl reports physical devices, not Windows drive letters. Correlate by model and capacity when useful (e.g. C: FireCuda 520 NVMe, D: Seagate 16TB, E: WD Red 4TB, F: two Toshiba 6TB pooled, G: MKNSSD 2TB SSD). A pooled/spanned volume spans multiple physical disks; report each disk separately.
