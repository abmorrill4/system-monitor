# system-monitor

A Claude plugin (Cowork + Claude Code) that monitors your computer's health and
hardware locally and reports it in plain language. Successor to `drive-health` —
SMART drive health is now one tier of a broader monitor.

All data stays on your machine.

## Install

> Windows x64. Self-contained binary — no Node, no build tools, nothing to compile.

### Claude Code (recommended)

```text
/plugin marketplace add abmorrill4/system-monitor
/plugin install system-monitor@abmorrill4
```

Restart Claude, then ask **"how's my system?"** The plugin bundles the server
binary *and* the skill — nothing else to wire up.

### Optional, for richer data

| Want | Install |
|------|---------|
| Drive health (SMART) | `winget install smartmontools.smartmontools` |
| GPU metrics | the NVIDIA driver (provides `nvidia-smi`); NVIDIA only |
| Temps / fans / voltages | [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor) with its web server on port 8085 |

LibreHardwareMonitor runs elevated on its own, so sensors work even when Claude
is **not** elevated.

### Drive health without running Claude as admin

Reading SMART needs Administrator. Rather than run Claude elevated, install the
helper once (in an **elevated** PowerShell) to keep a drive-health cache fresh in
the background:

```powershell
pwsh -File scripts/install-smart-helper.ps1
```

It registers a SYSTEM scheduled task that refreshes drive health every 15 minutes;
the (non-elevated) server reads that cache. Undo with `scripts/uninstall-smart-helper.ps1`.

### Manual install (other MCP clients)

Download `system-monitor-windows-x64.exe` from
[Releases](https://github.com/abmorrill4/system-monitor/releases) (verify against
`SHA256SUMS.txt`) and register it as a user-scoped stdio MCP server:

```powershell
claude mcp add --transport stdio --scope user system-monitor -- C:/path/to/system-monitor.exe
```

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

- **General metrics**: the [`sysinfo`](https://docs.rs/sysinfo) crate, compiled
  into the binary. No setup, no elevation, no Node runtime.
- **GPU**: `nvidia-smi` (ships with the NVIDIA driver). NVIDIA only.
- **Sensors**: [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor) running as Administrator, with its web server enabled (Options -> Remote Web Server, default port 8085) or accessible via WMI. Set `LHM_URL` to override the web server address.
- **Drive health**: `smartctl` (`winget install smartmontools.smartmontools`). Auto-detected in `C:\Program Files\smartmontools\bin`; override with `SMARTCTL_PATH`.

## Administrator privileges

On Windows, reading **SMART** drive data requires Administrator. The server
inherits the launching app's privileges, so either run the Claude app elevated,
or install the [SMART helper](#drive-health-without-running-claude-as-admin) so a
background SYSTEM task supplies drive health without elevating Claude. **Sensor**
data does not need Claude elevated — LibreHardwareMonitor provides it via its own
elevated service. Everything degrades gracefully when a source is unavailable.

## Development

Written in Rust; ships as a single ~0.7 MB static binary (no Node).

```bash
cargo test                      # pure parsers + health logic + stdio protocol/snapshot
cargo run                       # run the MCP server on stdio
cargo build --release           # optimized binary -> target/release/system-monitor.exe
pwsh scripts/build-plugin.ps1   # dist/system-monitor.plugin (bundles the binary + skill)
```

### Building on Windows

This project uses the GNU Rust toolchain to avoid a Visual Studio dependency:

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu
rustup default stable-x86_64-pc-windows-gnu
winget install BrechtSanders.WinLibs.POSIX.UCRT   # full mingw-w64 (dlltool/as) for linking
```

Make sure the WinLibs `mingw64\bin` directory is on `PATH` before building.

> Currently packaged as a Windows x64 plugin. Other platforms build from source
> with `cargo build --release` (the code is cross-platform via `sysinfo`).
