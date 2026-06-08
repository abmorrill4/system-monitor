// system-monitor MCP server: stdio JSON-RPC 2.0 (newline-delimited).
// Exposes the tool catalog, dispatches calls to the collectors, and aggregates
// the snapshot. Kept dependency-light: requests are handled sequentially, while
// the aggregate tools fan their collectors out across scoped threads.

use std::io::{BufRead, Write};

use serde_json::{json, Value};

use crate::util::settle;
use crate::{gpu, health, metrics, sensors, smart};

pub const SERVER_NAME: &str = "system-monitor";
pub const SERVER_VERSION: &str = "0.3.0";
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// The MCP tool catalog (names, descriptions, JSON input schemas).
pub fn tools() -> Value {
    let empty = json!({ "type": "object", "properties": {}, "additionalProperties": false });
    json!([
        { "name": "get_system_info",
          "description": "Host overview: OS, hostname, uptime, motherboard, BIOS, CPU model, core counts.",
          "inputSchema": empty },
        { "name": "get_system_snapshot",
          "description": "Everything at once: system info, CPU load, memory, disk usage, network, top processes, GPU. Best single call for a quick overview.",
          "inputSchema": empty },
        { "name": "get_health_report",
          "description": "Flagship: a prioritized health roll-up across drives (SMART), disk space, memory, CPU/GPU/drive temperatures. Returns overall OK/WARNING/CRITICAL with specific issues and recommendations.",
          "inputSchema": empty },
        { "name": "get_cpu",
          "description": "CPU model, cores, current load (overall and optionally per-core), frequency, and temperature if available.",
          "inputSchema": { "type": "object", "properties": { "per_core": { "type": "boolean", "description": "Include per-core load." } }, "additionalProperties": false } },
        { "name": "get_memory",
          "description": "RAM and swap usage (total, used, free, percent).",
          "inputSchema": empty },
        { "name": "get_disks",
          "description": "Disk space usage per mounted volume (size, used, free, percent).",
          "inputSchema": empty },
        { "name": "get_disk_io",
          "description": "Disk I/O throughput (read/write rates). Note: per-disk is partial on Windows.",
          "inputSchema": empty },
        { "name": "get_network",
          "description": "Network interfaces: IPs, MAC, and current rx/tx throughput.",
          "inputSchema": empty },
        { "name": "get_top_processes",
          "description": "Top processes by CPU or memory.",
          "inputSchema": { "type": "object", "properties": { "limit": { "type": "number", "description": "How many (default 10, max 50)." }, "sort_by": { "type": "string", "enum": ["cpu", "memory"], "description": "Sort field (default cpu)." } }, "additionalProperties": false } },
        { "name": "get_gpu",
          "description": "NVIDIA GPU metrics via nvidia-smi: utilization, VRAM, temperature, power, fan, clocks. Degrades gracefully if no NVIDIA GPU.",
          "inputSchema": empty },
        { "name": "get_sensors",
          "description": "Hardware sensors (CPU/board temps, fan RPM, voltages, clocks) via LibreHardwareMonitor. Requires LHM running; degrades gracefully if unavailable.",
          "inputSchema": empty },
        { "name": "get_drive_health",
          "description": "SMART health for a drive or all drives, with a verdict (HEALTHY/WARNING/FAILING/NEEDS_ELEVATION). Omit device for all.",
          "inputSchema": { "type": "object", "properties": { "device": { "type": "string" }, "type": { "type": "string" } }, "additionalProperties": false } },
        { "name": "run_self_test",
          "description": "Start a SMART self-test (short/long) on a drive; read results later via get_drive_health.",
          "inputSchema": { "type": "object", "properties": { "device": { "type": "string" }, "kind": { "type": "string", "enum": ["short", "long"] }, "type": { "type": "string" } }, "required": ["device"], "additionalProperties": false } }
    ])
}

pub fn get_system_snapshot() -> Value {
    let (system, cpu, memory, disks, network, top, gpu_data) = std::thread::scope(|sc| {
        let a = sc.spawn(|| settle(metrics::get_system_info));
        let b = sc.spawn(|| settle(|| metrics::get_cpu(false)));
        let c = sc.spawn(|| settle(metrics::get_memory));
        let d = sc.spawn(|| settle(metrics::get_disks));
        let e = sc.spawn(|| settle(metrics::get_network));
        let f = sc.spawn(|| settle(|| metrics::get_top_processes(5, "cpu")));
        let g = sc.spawn(|| settle(gpu::get_gpu));
        (
            a.join().unwrap(),
            b.join().unwrap(),
            c.join().unwrap(),
            d.join().unwrap(),
            e.join().unwrap(),
            f.join().unwrap(),
            g.join().unwrap(),
        )
    });
    json!({
        "system": system, "cpu": cpu, "memory": memory, "disks": disks,
        "network": network, "top_processes": top, "gpu": gpu_data,
    })
}

pub fn dispatch_tool(name: &str, args: &Value) -> Result<Value, String> {
    Ok(match name {
        "get_system_info" => metrics::get_system_info(),
        "get_system_snapshot" => get_system_snapshot(),
        "get_health_report" => health::get_health_report(),
        "get_cpu" => metrics::get_cpu(args.get("per_core").and_then(|v| v.as_bool()).unwrap_or(false)),
        "get_memory" => metrics::get_memory(),
        "get_disks" => metrics::get_disks(),
        "get_disk_io" => metrics::get_disk_io(),
        "get_network" => metrics::get_network(),
        "get_top_processes" => {
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(10);
            let sort_by = args.get("sort_by").and_then(|v| v.as_str()).unwrap_or("cpu");
            metrics::get_top_processes(limit, sort_by)
        }
        "get_gpu" => gpu::get_gpu(),
        "get_sensors" => sensors::get_sensors(),
        "get_drive_health" => smart::get_drive_health(args).map_err(|e| e.message)?,
        "run_self_test" => smart::run_self_test(args).map_err(|e| e.message)?,
        _ => return Err(format!("Unknown tool: {}", name)),
    })
}

fn has_id(id: &Option<Value>) -> bool {
    matches!(id, Some(v) if !v.is_null())
}

fn handle(msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");

    match method {
        "notifications/initialized" | "initialized" => None,
        "initialize" => Some(json!({
            "jsonrpc": "2.0", "id": id,
            "result": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION }
            }
        })),
        "ping" => Some(json!({ "jsonrpc": "2.0", "id": id, "result": {} })),
        "tools/list" => Some(json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": tools() } })),
        "tools/call" => {
            let tname = msg.pointer("/params/name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let targs = msg.pointer("/params/arguments").cloned().unwrap_or_else(|| json!({}));
            let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dispatch_tool(&tname, &targs))) {
                Ok(Ok(data)) => json!({
                    "content": [{ "type": "text", "text": serde_json::to_string_pretty(&data).unwrap_or_default() }],
                    "isError": false
                }),
                Ok(Err(message)) => json!({ "content": [{ "type": "text", "text": message }], "isError": true }),
                Err(_) => json!({ "content": [{ "type": "text", "text": "internal error: collector panicked" }], "isError": true }),
            };
            Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
        }
        _ => {
            if has_id(&id) {
                Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("Method not found: {}", method) } }))
            } else {
                None
            }
        }
    }
}

/// Read newline-delimited JSON-RPC requests from stdin and write responses to
/// stdout until EOF.
pub fn run_stdio_loop() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(line) else { continue };
        if let Some(response) = handle(&msg) {
            let _ = writeln!(out, "{}", serde_json::to_string(&response).unwrap_or_default());
            let _ = out.flush();
        }
    }
}
