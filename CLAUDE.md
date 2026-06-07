# system-monitor - project guide

A Claude plugin (Cowork + Claude Code) that monitors local system & hardware
health and reports it in plain language. Evolved from `drive-health`.

## Architecture

stdio MCP server (`server/server.js`, JSON-RPC 2.0, newline-delimited) that
dispatches to modular collectors:

- `server/collectors/metrics.js` - general metrics via the `systeminformation`
  library (cpu, memory, disks, disk I/O, network, processes, host). The only
  runtime dependency; bundled into the `.plugin`.
- `server/collectors/smart.js`   - drive health via `smartctl` (zero-dep, shell-out). `summarize()` is pure.
- `server/collectors/gpu.js`     - NVIDIA via `nvidia-smi`. `parseNvidiaSmi()` is pure.
- `server/collectors/sensors.js` - LibreHardwareMonitor via HTTP (port 8085) or WMI (PowerShell). `flattenLhm()`, `parseLhmValue()`, `normalizeWmiSensors()` are pure.
- `server/collectors/health.js`  - unified roll-up. `evaluateHealth(data)` is pure and tested; `getHealthReport()` gathers then evaluates.

`server.js` exports `{ TOOLS, dispatchTool, getSystemSnapshot }` and only starts
the stdio loop when run directly (`require.main === module`).

## Tools

get_health_report, get_system_snapshot, get_system_info, get_cpu, get_memory,
get_disks, get_disk_io, get_network, get_top_processes, get_gpu, get_sensors,
get_drive_health, run_self_test. All read-only except run_self_test.

## Design rules

- Keep collectors independent and individually `settle()`-wrapped so one failing
  source never breaks an aggregate (snapshot / health report).
- Keep parsers pure (string/JSON -> object) and unit-test them with fixtures.
- ASCII-only in source files. Do NOT use the degree sign or other non-ASCII
  literals in code - prefer "C". (A prior edit corrupted sensors.js encoding.)
- Degrade gracefully: every source reports an `available:false` + `reason`
  rather than throwing when its tool/permission is missing.

## Dev commands

```bash
npm install     # systeminformation (no transitive deps)
npm test        # node:test
npm start       # run the server on stdio
npm run build   # dist/system-monitor.plugin (bundles node_modules)
```

`npm test` needs no smartctl/nvidia/LHM installed - parsers run on fixtures and
the integration test only relies on systeminformation + the stdio protocol.

## Runtime requirements (user machine)

- Node.js. systeminformation bundled.
- smartmontools for drives (`winget install smartmontools.smartmontools`), Admin.
- NVIDIA driver for GPU (nvidia-smi).
- LibreHardwareMonitor for temps/fans (Admin; web server port 8085 or WMI).

## Roadmap (not yet built)

- `monitor` tool: sample a subsystem over a duration, return min/avg/max trend.
- MCP resources for live CPU/mem/sensor streams.
- Scheduled daily health check.
- AMD/Intel GPU support.
