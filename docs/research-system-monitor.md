# Research: expanding `drive-health` into a full system-monitoring MCP server

_Working doc — June 2026. Goal: decide what a "full system monitor" MCP server should do on THESEUS (Windows 11, Ryzen 7 5800X, GTX 1080 Ti, 6 drives), what already exists, and how to build it._

## 1. What's already out there

System-monitoring MCP servers exist, but they're all early-stage hobby projects with overlapping, shallow feature sets. None are Windows-first, none do disk SMART health, and none integrate real hardware sensors. That's the gap we can own.

| Project | Lang / stack | Tools | Notable | Weakness |
|---|---|---|---|---|
| [seekrays/mcp-monitor](https://github.com/seekrays/mcp-monitor) | Go (gopsutil) | cpu, memory, disk, network, host, process | Most popular (~80★), clean tool design, single binary | No GPU, no temps, no SMART |
| [huhabla/mcp-system-monitor](https://github.com/huhabla/mcp-system-monitor) | Python (psutil + FastMCP) | 15 tools incl. GPU, I/O perf, load, enhanced mem/net, snapshots; also MCP _resources_ | Most comprehensive; good caching + test architecture | Temps "limited" on Windows; NVIDIA-only GPU; no SMART |
| [dknell/mcp-system-info](https://github.com/dknell/mcp-system-info) | Python (psutil) | cpu, memory, disk, network, processes | Simple, readable | Basic; same Windows temp gap |
| [hungtrungthinh/mcp-system-monitor](https://github.com/hungtrungthinh/mcp-system-monitor) | Python | cpu, memory, disk, network, process + REST | Dual MCP + HTTP API | Linux-server focused |

**Patterns worth copying:**
- A consistent `get_<subsystem>_info` tool naming convention.
- A single `get_system_snapshot` that returns everything in one call (saves round-trips).
- `monitor_*` tools that sample over a duration and return a trend, distinct from point-in-time reads.
- Per-collector **caching** (~2s) so rapid calls don't hammer the system.
- **Graceful degradation**: skip what the platform/permissions can't provide rather than erroring.

**What none of them have (our differentiation):**
- Disk **SMART health** with a verdict — we already built this (smartctl).
- Real **sensor data** (temps/fans/voltages) on Windows via LibreHardwareMonitor.
- A roll-up **health report** that turns raw metrics into "here's what's wrong / what to watch."

## 2. The Windows data-source problem

This is the crux. Different metrics come from different places on Windows, with very different reliability.

| Metric class | Best Windows source | Reliability | Notes |
|---|---|---|---|
| CPU load, freq, per-core | `systeminformation` (Node) | High | Cross-platform, no shelling needed |
| Memory / swap | `systeminformation` | High | |
| Disk usage per volume | `systeminformation` | High | Maps to your C:/D:/E:/F:/G: |
| Disk I/O rates | `systeminformation` `fsStats` | Medium | Per-disk on Windows is partial |
| Network throughput | `systeminformation` `networkStats` | High | |
| Processes (top by CPU/mem) | `systeminformation` | High | |
| Host/OS/uptime/BIOS | `systeminformation` | High | |
| **CPU/GPU/board temps, fans, voltages** | **LibreHardwareMonitor** | High _if running_ | `systeminformation.cpuTemperature()` is [unreliable on Windows](https://github.com/sebhildebrandt/systeminformation); LHM is the real answer |
| **GPU (GTX 1080 Ti)** util/mem/temp/power/clocks | **`nvidia-smi`** | High | Ships with NVIDIA driver; CSV query mode is trivial to parse |
| **Drive SMART health** | **`smartctl`** | High | Already implemented |

### Sensors: LibreHardwareMonitor (LHM)

[LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor) is the de-facto open-source sensor reader on Windows. Once it's running (as admin, in the background) it exposes sensor data two ways we can consume from Node:

1. **WMI** namespace `root\LibreHardwareMonitor` — query the `Sensor` class (Temperature/Fan/Voltage/Load/Clock).
2. **HTTP web server** (Options → enable, default port **8085**) — returns a JSON sensor tree. Simplest to consume from Node (`fetch http://localhost:8085/data.json`), no WMI bindings needed.

**Decision:** treat sensors as an _optional capability_. If LHM's web server or WMI is reachable, surface full temps/fans/voltages. If not, degrade to whatever `nvidia-smi` (GPU temp) and `smartctl` (drive temp) already give us, and tell the user how to enable LHM.

## 3. Architecture decision: dependency vs. zero-dep

Our current `drive-health` server is deliberately **zero-dependency** (hand-rolled JSON-RPC, shells to `smartctl`). A full monitor changes that calculus.

**Option A — adopt `systeminformation` (recommended).**
[`systeminformation`](https://www.npmjs.com/package/systeminformation) is a mature (MIT, 50+ functions, actively maintained) Node library that covers CPU/mem/disk/net/process/host/battery/graphics across Windows/macOS/Linux, and is itself **dependency-free** (no transitive packages). Adopting it replaces hundreds of lines of brittle WMI/PowerShell parsing with one well-tested library.
- _Cost:_ we break the "zero runtime deps" rule and must **bundle `node_modules` into the `.plugin`** (Cowork plugins don't run `npm install`). Because systeminformation has no transitive deps, that's a single self-contained folder — manageable.

**Option B — stay zero-dep, shell to PowerShell/CIM.**
Keep the hand-rolled approach, gather metrics via `Get-CimInstance` / `Get-Counter`. Keeps the plugin tiny and audit-friendly, but it's a lot of fragile parsing and Windows-only.

**Recommendation:** Option A for the general metrics tier; keep our own shell-outs for the specialist tiers (`smartctl`, `nvidia-smi`, LHM). This gives broad coverage fast while keeping the differentiated parts under our control. Note the bundling requirement in the build script.

## 4. Proposed capability set & tool surface

Grouped by tier. Read-only except `run_self_test`.

**Host / overview**
- `get_system_info` — OS, hostname, uptime, motherboard, BIOS, CPU model
- `get_system_snapshot` — everything below in one call (cached)
- `get_health_report` — **the headline feature**: roll up drive verdicts, temp thresholds, low-disk-space, high-load into one prioritized "what's wrong / what to watch" summary

**Compute**
- `get_cpu` — model, cores, per-core load, frequency, current load %
- `get_gpu` — via `nvidia-smi`: util, VRAM used/total, temp, power, fan, clocks
- `get_memory` — RAM + swap usage

**Storage** (extends current plugin)
- `get_disks` — usage per volume (maps to drive letters)
- `get_disk_io` — read/write rates
- `get_drive_health` — SMART verdict _(done)_
- `run_self_test` — SMART self-test _(done)_

**Network**
- `get_network` — interfaces, link state, throughput (rx/tx per sec)

**Processes**
- `get_top_processes` — top N by CPU or memory

**Sensors** (LHM-dependent)
- `get_sensors` — temps/fans/voltages tree when LibreHardwareMonitor is available

**Sampling**
- `monitor` — sample a chosen subsystem over a duration, return min/avg/max + trend

## 5. Phased build plan

- **Phase 0 — done.** Drive health via smartctl (verdicts, self-test).
- **Phase 1 — core metrics.** Add `systeminformation`; implement `get_cpu`, `get_memory`, `get_disks`, `get_disk_io`, `get_network`, `get_top_processes`, `get_system_info`, `get_system_snapshot`. Update build to bundle `node_modules`. Rename plugin (e.g. `system-monitor`), keep drive tools as a tier.
- **Phase 2 — GPU.** `get_gpu` via `nvidia-smi` CSV query; graceful "no NVIDIA GPU" path.
- **Phase 3 — sensors.** `get_sensors` via LHM web server (HTTP JSON) with WMI fallback; detection + setup guidance.
- **Phase 4 — intelligence.** `get_health_report` roll-up with thresholds; `monitor` sampling; optionally MCP _resources_ for live data and a scheduled daily health check.

## 6. Cross-cutting considerations

- **Privileges:** SMART (smartctl) and LHM sensor reads need **Administrator**; `nvidia-smi` and most `systeminformation` calls do not. Report a clear per-tool "needs elevation" state rather than failing silently.
- **Safety:** keep everything read-only by default. `run_self_test` is the only action and is already explicit.
- **Performance:** adopt the per-collector caching pattern (~2s) so `get_system_snapshot` and rapid polling stay cheap.
- **Naming/scope:** renaming `drive-health` → `system-monitor` means updating `plugin.json`, `.mcp.json` server key, the skill, and the build artifact name. Drive tools become one tier of a larger surface.
- **Testing:** the existing pattern holds well — unit-test pure parsers against fixtures, plus the stdio protocol smoke test. Mock `nvidia-smi`/LHM/`smartctl` outputs as fixtures.

## 7. Open questions for Arthur

1. **Dependency call:** OK to adopt `systeminformation` (Option A) and bundle `node_modules` in the plugin, or do you want to stay strictly zero-dep (Option B)?
2. **Sensors:** worth the Phase 3 LibreHardwareMonitor integration (it's the only way to get real CPU/board temps on Windows), or defer it?
3. **Scope of v1:** ship Phases 1–2 first (broad metrics + GPU), then iterate? Or go straight for the `get_health_report` roll-up as the flagship?
4. **Rename:** keep building under `drive-health`, or rename the project/plugin to `system-monitor` now before the surface grows?

## Sources

- [seekrays/mcp-monitor](https://github.com/seekrays/mcp-monitor)
- [huhabla/mcp-system-monitor](https://github.com/huhabla/mcp-system-monitor)
- [dknell/mcp-system-info](https://github.com/dknell/mcp-system-info)
- [hungtrungthinh/mcp-system-monitor](https://github.com/hungtrungthinh/mcp-system-monitor)
- [systeminformation (npm)](https://www.npmjs.com/package/systeminformation) · [GitHub](https://github.com/sebhildebrandt/systeminformation)
- [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor) · [project site](https://librehardwaremonitor.net/)
- [Example MCP servers](https://modelcontextprotocol.io/examples)
