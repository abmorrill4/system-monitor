# system-monitor - project guide

A Claude plugin (Cowork + Claude Code) that monitors local system & hardware
health and reports it in plain language. Evolved from `drive-health`.

Implemented in Rust as a single static binary (no runtime needed on the user's
machine). Ported from an earlier Node/`systeminformation` implementation.

## Architecture

stdio MCP server (JSON-RPC 2.0, newline-delimited). The binary (`src/main.rs`)
just runs the loop; all logic lives in the library crate (`src/lib.rs`) so it is
unit- and integration-testable. Requests are handled sequentially; the aggregate
tools fan their collectors out across scoped threads.

Modular collectors:

- `src/metrics.rs` - general metrics via the [`sysinfo`](https://docs.rs/sysinfo)
  crate (cpu, memory, disks, disk I/O, network, processes, host). Motherboard /
  BIOS come from WMI (PowerShell) on Windows since sysinfo does not expose them.
- `src/smart.rs`   - drive health via `smartctl` (shell-out). `summarize()` is pure.
- `src/gpu.rs`     - NVIDIA via `nvidia-smi`. `parse_nvidia_smi()` is pure.
- `src/sensors.rs` - LibreHardwareMonitor via HTTP (port 8085, tiny built-in
  HTTP/1.0 client) or WMI (PowerShell). `parse_lhm_value()`, `flatten_lhm()`,
  `normalize_wmi_sensors()` are pure.
- `src/health.rs`  - unified roll-up. `evaluate_health(data)` is pure and tested;
  `get_health_report()` gathers (concurrently) then evaluates.
- `src/server.rs`  - tool catalog, dispatch, JSON-RPC plumbing, snapshot.
- `src/util.rs`    - numeric helpers, `settle()`, the PowerShell-JSON shim.

Collectors pass data as `serde_json::Value` (the JS "data bag" shape), so the
pure parsers and `evaluate_health` are tested against the same fixtures the old
JS used.

Dependencies: `serde`, `serde_json`, `sysinfo` only. The LHM HTTP client and the
WMI/PowerShell shell-outs are hand-rolled to avoid an HTTP/WMI crate.

## Tools

get_health_report, get_system_snapshot, get_system_info, get_cpu, get_memory,
get_disks, get_disk_io, get_network, get_top_processes, get_gpu, get_sensors,
get_drive_health, run_self_test. All read-only except run_self_test.

## Design rules

- Keep collectors independent and individually `settle()`-wrapped so one failing
  source never breaks an aggregate (snapshot / health report).
- Keep parsers pure (string/JSON -> Value) and unit-test them with fixtures.
- ASCII-only in source files. Do NOT use the degree sign or other non-ASCII
  literals in code - prefer "C", and `\u{b0}` escapes in tests when a literal
  degree sign must be exercised.
- Degrade gracefully: every source reports an `available:false` + `reason`
  (or an `error`) rather than panicking when its tool/permission is missing.

## Dev commands

```bash
cargo test              # unit (pure parsers + health) + stdio integration test
cargo run               # run the server on stdio
cargo build --release   # optimized binary at target/release/system-monitor.exe
pwsh scripts/build-plugin.ps1   # dist/system-monitor.plugin (bundles the binary)
```

`cargo test` needs no smartctl/nvidia/LHM installed - the pure parsers run on
fixtures and the integration test only relies on sysinfo + the stdio protocol.

## Toolchain (Windows)

Built with the GNU toolchain (`stable-x86_64-pc-windows-gnu`) to avoid a Visual
Studio dependency. Linking the Windows crates that `sysinfo` pulls in needs a
full mingw-w64 (e.g. WinLibs) on PATH for `dlltool`/`as`; Rust's bundled
self-contained mingw is incomplete for this.

## Runtime requirements (user machine)

- None for the core metrics - the binary is self-contained (no Node).
- smartmontools for drives (`winget install smartmontools.smartmontools`), Admin.
- NVIDIA driver for GPU (nvidia-smi).
- LibreHardwareMonitor for temps/fans (Admin; web server port 8085 or WMI).

## Roadmap (not yet built)

- `monitor` tool: sample a subsystem over a duration, return min/avg/max trend.
- MCP resources for live CPU/mem/sensor streams.
- Scheduled daily health check.
- AMD/Intel GPU support.
- Cross-platform plugin packaging (per-OS binaries; currently Windows x64).
