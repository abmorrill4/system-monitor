#!/usr/bin/env node
"use strict";
/*
 * system-monitor MCP server
 * stdio JSON-RPC 2.0 (newline-delimited). Aggregates several collectors:
 *   - metrics  (systeminformation): cpu, memory, disks, disk I/O, network, processes, host
 *   - smart    (smartctl): drive health + self-test
 *   - gpu      (nvidia-smi): NVIDIA GPU metrics
 *   - sensors  (LibreHardwareMonitor): temps/fans/voltages
 *   - health   : unified roll-up
 */

const metrics = require("./collectors/metrics");
const smart = require("./collectors/smart");
const gpu = require("./collectors/gpu");
const sensors = require("./collectors/sensors");
const health = require("./collectors/health");

const SERVER_NAME = "system-monitor";
const SERVER_VERSION = "0.2.0";
const PROTOCOL_VERSION = "2024-11-05";

/* ---------- snapshot ---------- */
async function getSystemSnapshot() {
  const settle = async (p) => { try { return await p; } catch (e) { return { error: e.message }; } };
  const [system, cpu, memory, disks, network, top, gpuData] = await Promise.all([
    settle(metrics.getSystemInfo()),
    settle(metrics.getCpu()),
    settle(metrics.getMemory()),
    settle(metrics.getDisks()),
    settle(metrics.getNetwork()),
    settle(metrics.getTopProcesses({ limit: 5 })),
    settle(gpu.getGpu()),
  ]);
  return { system, cpu, memory, disks, network, top_processes: top, gpu: gpuData };
}

/* ---------- tools ---------- */
const TOOLS = [
  { name: "get_system_info", description: "Host overview: OS, hostname, uptime, motherboard, BIOS, CPU model, core counts.",
    inputSchema: { type: "object", properties: {}, additionalProperties: false } },
  { name: "get_system_snapshot", description: "Everything at once: system info, CPU load, memory, disk usage, network, top processes, GPU. Best single call for a quick overview.",
    inputSchema: { type: "object", properties: {}, additionalProperties: false } },
  { name: "get_health_report", description: "Flagship: a prioritized health roll-up across drives (SMART), disk space, memory, CPU/GPU/drive temperatures. Returns overall OK/WARNING/CRITICAL with specific issues and recommendations.",
    inputSchema: { type: "object", properties: {}, additionalProperties: false } },
  { name: "get_cpu", description: "CPU model, cores, current load (overall and optionally per-core), frequency, and temperature if available.",
    inputSchema: { type: "object", properties: { per_core: { type: "boolean", description: "Include per-core load." } }, additionalProperties: false } },
  { name: "get_memory", description: "RAM and swap usage (total, used, free, percent).",
    inputSchema: { type: "object", properties: {}, additionalProperties: false } },
  { name: "get_disks", description: "Disk space usage per mounted volume (size, used, free, percent).",
    inputSchema: { type: "object", properties: {}, additionalProperties: false } },
  { name: "get_disk_io", description: "Disk I/O throughput (read/write rates). Note: per-disk is partial on Windows.",
    inputSchema: { type: "object", properties: {}, additionalProperties: false } },
  { name: "get_network", description: "Network interfaces: IPs, link state, speed, and current rx/tx throughput.",
    inputSchema: { type: "object", properties: {}, additionalProperties: false } },
  { name: "get_top_processes", description: "Top processes by CPU or memory.",
    inputSchema: { type: "object", properties: { limit: { type: "number", description: "How many (default 10, max 50)." }, sort_by: { type: "string", enum: ["cpu", "memory"], description: "Sort field (default cpu)." } }, additionalProperties: false } },
  { name: "get_gpu", description: "NVIDIA GPU metrics via nvidia-smi: utilization, VRAM, temperature, power, fan, clocks. Degrades gracefully if no NVIDIA GPU.",
    inputSchema: { type: "object", properties: {}, additionalProperties: false } },
  { name: "get_sensors", description: "Hardware sensors (CPU/board temps, fan RPM, voltages, clocks) via LibreHardwareMonitor. Requires LHM running; degrades gracefully if unavailable.",
    inputSchema: { type: "object", properties: {}, additionalProperties: false } },
  { name: "get_drive_health", description: "SMART health for a drive or all drives, with a verdict (HEALTHY/WARNING/FAILING/NEEDS_ELEVATION). Omit device for all.",
    inputSchema: { type: "object", properties: { device: { type: "string" }, type: { type: "string" } }, additionalProperties: false } },
  { name: "run_self_test", description: "Start a SMART self-test (short/long) on a drive; read results later via get_drive_health.",
    inputSchema: { type: "object", properties: { device: { type: "string" }, kind: { type: "string", enum: ["short", "long"] }, type: { type: "string" } }, required: ["device"], additionalProperties: false } },
];

async function dispatchTool(name, args) {
  switch (name) {
    case "get_system_info": return await metrics.getSystemInfo();
    case "get_system_snapshot": return await getSystemSnapshot();
    case "get_health_report": return await health.getHealthReport();
    case "get_cpu": return await metrics.getCpu(args);
    case "get_memory": return await metrics.getMemory();
    case "get_disks": return await metrics.getDisks();
    case "get_disk_io": return await metrics.getDiskIO();
    case "get_network": return await metrics.getNetwork();
    case "get_top_processes": return await metrics.getTopProcesses(args);
    case "get_gpu": return await gpu.getGpu();
    case "get_sensors": return await sensors.getSensors();
    case "get_drive_health": return await smart.getDriveHealth(args);
    case "run_self_test": return await smart.runSelfTest(args);
    default: throw new Error("Unknown tool: " + name);
  }
}

/* ---------- JSON-RPC plumbing ---------- */
function send(msg) { process.stdout.write(JSON.stringify(msg) + "\n"); }
function sendResult(id, result) { send({ jsonrpc: "2.0", id, result }); }
function sendError(id, code, message) { send({ jsonrpc: "2.0", id, error: { code, message } }); }

async function handle(msg) {
  const { id, method, params } = msg;
  if (method === "notifications/initialized" || method === "initialized") return;
  if (method === "initialize") {
    return sendResult(id, { protocolVersion: PROTOCOL_VERSION, capabilities: { tools: {} }, serverInfo: { name: SERVER_NAME, version: SERVER_VERSION } });
  }
  if (method === "ping") return sendResult(id, {});
  if (method === "tools/list") return sendResult(id, { tools: TOOLS });
  if (method === "tools/call") {
    const toolName = params && params.name;
    const toolArgs = (params && params.arguments) || {};
    try {
      const data = await dispatchTool(toolName, toolArgs);
      return sendResult(id, { content: [{ type: "text", text: JSON.stringify(data, null, 2) }], isError: false });
    } catch (e) {
      return sendResult(id, { content: [{ type: "text", text: (e && e.message) ? e.message : String(e) }], isError: true });
    }
  }
  if (typeof id !== "undefined") sendError(id, -32601, "Method not found: " + method);
}

/* ---------- stdin loop ---------- */
function startStdioLoop() {
  let buf = "";
  let pending = 0;
  let stdinEnded = false;
  const maybeExit = () => { if (stdinEnded && pending === 0) process.exit(0); };
  process.stdin.setEncoding("utf8");
  process.stdin.on("data", (chunk) => {
    buf += chunk;
    let nl;
    while ((nl = buf.indexOf("\n")) >= 0) {
      const line = buf.slice(0, nl).trim();
      buf = buf.slice(nl + 1);
      if (!line) continue;
      let msg;
      try { msg = JSON.parse(line); } catch (_) { continue; }
      pending++;
      Promise.resolve(handle(msg))
        .catch((e) => { if (msg && typeof msg.id !== "undefined") sendError(msg.id, -32603, String(e && e.message || e)); })
        .finally(() => { pending--; maybeExit(); });
    }
  });
  process.stdin.on("end", () => { stdinEnded = true; maybeExit(); });
}

if (require.main === module) startStdioLoop();

module.exports = { TOOLS, dispatchTool, getSystemSnapshot, SERVER_NAME, SERVER_VERSION, PROTOCOL_VERSION };
