"use strict";
const test = require("node:test");
const assert = require("node:assert");
const { spawn } = require("node:child_process");
const path = require("node:path");

const smart = require("../server/collectors/smart");
const gpu = require("../server/collectors/gpu");
const sensors = require("../server/collectors/sensors");
const health = require("../server/collectors/health");
const SERVER = path.join(__dirname, "..", "server", "server.js");

/* ---------- SMART summarize ---------- */
test("smart: healthy NVMe -> HEALTHY", () => {
  const s = smart.summarize({
    smartctl: { messages: [] }, device: { name: "/dev/sda", protocol: "NVMe" },
    model_name: "FireCuda 520", smart_status: { passed: true }, temperature: { current: 41 },
    nvme_smart_health_information_log: { critical_warning: 0, percentage_used: 7, available_spare: 100, available_spare_threshold: 10, media_errors: 0, data_units_written: 48000000 },
  });
  assert.strictEqual(s.verdict, "HEALTHY");
  assert.strictEqual(s.host_written_tb, 24.58);
});
test("smart: pending sectors -> WARNING", () => {
  const s = smart.summarize({ smartctl: { messages: [] }, smart_status: { passed: true },
    ata_smart_attributes: { table: [{ id: 197, name: "Current_Pending_Sector", raw: { value: 8 } }] } });
  assert.strictEqual(s.verdict, "WARNING");
  assert.strictEqual(s.pending_sectors, 8);
});
test("smart: failed status -> FAILING", () => {
  const s = smart.summarize({ smartctl: { messages: [] }, smart_status: { passed: false } });
  assert.strictEqual(s.verdict, "FAILING");
});

/* ---------- nvidia-smi parser ---------- */
test("gpu: parse nvidia-smi CSV", () => {
  const csv = "0, NVIDIA GeForce GTX 1080 Ti, 23, 1024, 11264, 62, 95.5, 250, 41, 1771";
  const [g] = gpu.parseNvidiaSmi(csv);
  assert.strictEqual(g.name, "NVIDIA GeForce GTX 1080 Ti");
  assert.strictEqual(g.utilization_percent, 23);
  assert.strictEqual(g.memory_total_mb, 11264);
  assert.strictEqual(g.temperature_c, 62);
  assert.strictEqual(g.power_draw_w, 95.5);
});

/* ---------- LHM parsers ---------- */
test("sensors: parseLhmValue splits number + unit", () => {
  assert.deepStrictEqual(sensors.parseLhmValue("45.0 °C"), { value: 45, unit: "°C" });
  assert.deepStrictEqual(sensors.parseLhmValue("1200 RPM"), { value: 1200, unit: "RPM" });
});
test("sensors: flattenLhm walks the data.json tree", () => {
  const tree = { Text: "Sensor", Children: [
    { Text: "THESEUS", Children: [
      { Text: "AMD Ryzen 7 5800X", Children: [
        { Text: "Temperatures", Children: [
          { Text: "Core (Tctl/Tdie)", Value: "52.0 °C", Min: "30.0 °C", Max: "78.0 °C", Children: [] },
        ] },
        { Text: "Clocks", Children: [ { Text: "Core #1", Value: "4200.0 MHz", Children: [] } ] },
      ] },
    ] },
  ] };
  const flat = sensors.flattenLhm(tree);
  const cpuTemp = flat.find((s) => s.category === "temperature");
  assert.ok(cpuTemp);
  assert.strictEqual(cpuTemp.hardware, "AMD Ryzen 7 5800X");
  assert.strictEqual(cpuTemp.value, 52);
  assert.strictEqual(cpuTemp.max, 78);
  assert.ok(flat.some((s) => s.category === "clock" && s.value === 4200));
});
test("sensors: normalizeWmiSensors maps SensorType to units", () => {
  const rows = [{ Name: "CPU Package", SensorType: "Temperature", Value: 55.5, Parent: "/amdcpu/0" }];
  const [s] = sensors.normalizeWmiSensors(rows);
  assert.strictEqual(s.category, "temperature");
  assert.strictEqual(s.unit, "C");
  assert.strictEqual(s.value, 55.5);
});

/* ---------- health roll-up (pure) ---------- */
test("health: clean inputs -> OK", () => {
  const r = health.evaluateHealth({
    drives: { reports: [{ model: "SSD", verdict: "HEALTHY", temperature_c: 40 }] },
    disks: { volumes: [{ mount: "C:", used_percent: 50, free_gb: 200 }] },
    memory: { used_percent: 45 },
    cpu: { temperature_c: 50 },
    gpu: { available: true, gpus: [{ name: "GTX 1080 Ti", temperature_c: 60 }] },
    sensors: { available: false, reason: "no LHM" },
  });
  assert.strictEqual(r.overall, "OK");
  assert.strictEqual(r.issues.length, 0);
});
test("health: failing drive + low disk + hot gpu -> CRITICAL, sorted", () => {
  const r = health.evaluateHealth({
    drives: { reports: [{ model: "Old HDD", verdict: "FAILING" }] },
    disks: { volumes: [{ mount: "C:", used_percent: 96, free_gb: 8 }] },
    memory: { used_percent: 95 },
    gpu: { available: true, gpus: [{ name: "GTX 1080 Ti", temperature_c: 97 }] },
  });
  assert.strictEqual(r.overall, "CRITICAL");
  assert.strictEqual(r.issues[0].severity, "CRITICAL");
  assert.ok(r.issues.some((i) => i.area === "disk-space"));
  assert.ok(r.issues.some((i) => i.area === "memory"));
});
test("health: sensors temp preferred over cpu fallback", () => {
  const r = health.evaluateHealth({
    sensors: { available: true, by_category: { temperature: [{ name: "CPU Package", value: 96 }] } },
    cpu: { temperature_c: 40 },
  });
  assert.ok(r.issues.some((i) => i.area === "cpu-temp" && i.severity === "CRITICAL"));
});

/* ---------- protocol + live snapshot ---------- */
function rpc(lines) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [SERVER], { stdio: ["pipe", "pipe", "inherit"] });
    let out = "";
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (d) => { out += d; });
    child.on("error", reject);
    child.on("close", () => resolve(out.trim().split("\n").filter(Boolean).map((l) => JSON.parse(l))));
    for (const l of lines) child.stdin.write(JSON.stringify(l) + "\n");
    child.stdin.end();
  });
}
test("protocol: initialize + tools/list exposes all tools", async () => {
  const msgs = await rpc([
    { jsonrpc: "2.0", id: 1, method: "initialize", params: {} },
    { jsonrpc: "2.0", id: 2, method: "tools/list" },
  ]);
  const list = msgs.find((m) => m.id === 2);
  const names = list.result.tools.map((t) => t.name);
  for (const expected of ["get_system_info", "get_system_snapshot", "get_health_report", "get_cpu", "get_memory", "get_disks", "get_network", "get_top_processes", "get_gpu", "get_sensors", "get_drive_health", "run_self_test"]) {
    assert.ok(names.includes(expected), "missing tool: " + expected);
  }
});
test("integration: get_memory returns real data via systeminformation", async () => {
  const msgs = await rpc([
    { jsonrpc: "2.0", id: 1, method: "initialize", params: {} },
    { jsonrpc: "2.0", id: 2, method: "tools/call", params: { name: "get_memory", arguments: {} } },
  ]);
  const res = msgs.find((m) => m.id === 2);
  assert.strictEqual(res.result.isError, false);
  const mem = JSON.parse(res.result.content[0].text);
  assert.ok(mem.total_gb > 0, "expected positive total memory");
});
