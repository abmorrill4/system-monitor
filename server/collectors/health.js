"use strict";
/* Unified health roll-up. Gathers from every tier and produces a prioritized
   "what's wrong / what to watch" report. evaluateHealth() is pure and tested. */

const metrics = require("./metrics");
const gpu = require("./gpu");
const sensors = require("./sensors");
const smart = require("./smart");

const THRESHOLDS = {
  diskFreePctWarn: 10,        // < 10% free
  diskFreeGbWarn: 20,         // or < 20 GB free
  memUsedPctWarn: 90,
  cpuTempWarn: 85, cpuTempCrit: 95,
  gpuTempWarn: 85, gpuTempCrit: 95,
  driveTempWarn: 60, driveTempCrit: 70,
};

const RANK = { OK: 0, WARNING: 1, CRITICAL: 2 };
const worse = (a, b) => (RANK[b] > RANK[a] ? b : a);

/* Pure: given collected data, produce { overall, issues[], sections } */
function evaluateHealth(data, t = THRESHOLDS) {
  const issues = [];
  let overall = "OK";
  const add = (severity, area, detail, recommendation) => {
    issues.push({ severity, area, detail, recommendation });
    overall = worse(overall, severity);
  };

  // Drives (SMART)
  if (data.drives && Array.isArray(data.drives.reports)) {
    for (const d of data.drives.reports) {
      const label = d.model || d.device || "drive";
      if (d.verdict === "FAILING") add("CRITICAL", "drive", `${label}: SMART overall-health FAILED`, "Back up immediately and replace the drive.");
      else if (d.verdict === "WARNING") add("WARNING", "drive", `${label}: ${(d.warnings || []).join("; ")}`, "Back up and monitor; consider a long self-test.");
      else if (d.verdict === "NEEDS_ELEVATION") add("WARNING", "drive", `${label}: SMART unreadable (no admin)`, "Run as Administrator to read drive health.");
      if (typeof d.temperature_c === "number") {
        if (d.temperature_c >= t.driveTempCrit) add("CRITICAL", "drive-temp", `${label} at ${d.temperature_c}°C`, "Improve drive cooling/airflow.");
        else if (d.temperature_c >= t.driveTempWarn) add("WARNING", "drive-temp", `${label} at ${d.temperature_c}°C`, "Check airflow around drives.");
      }
    }
  }

  // Disk space
  if (data.disks && Array.isArray(data.disks.volumes)) {
    for (const v of data.disks.volumes) {
      const lowPct = typeof v.used_percent === "number" && v.used_percent >= 100 - t.diskFreePctWarn;
      const lowGb = typeof v.free_gb === "number" && v.free_gb <= t.diskFreeGbWarn;
      if (lowPct || lowGb) add("WARNING", "disk-space", `${v.mount} ${v.used_percent}% used (${v.free_gb} GB free)`, "Free up space or expand storage.");
    }
  }

  // Memory
  if (data.memory && typeof data.memory.used_percent === "number" && data.memory.used_percent >= t.memUsedPctWarn) {
    add("WARNING", "memory", `RAM ${data.memory.used_percent}% used`, "Close memory-heavy apps; check for leaks.");
  }

  // CPU temp (prefer sensors, fall back to cpu.temperature_c)
  const cpuTemp = pickCpuTemp(data);
  if (typeof cpuTemp === "number") {
    if (cpuTemp >= t.cpuTempCrit) add("CRITICAL", "cpu-temp", `CPU at ${cpuTemp}°C`, "Check cooler mount, thermal paste, and case airflow.");
    else if (cpuTemp >= t.cpuTempWarn) add("WARNING", "cpu-temp", `CPU at ${cpuTemp}°C`, "Monitor under load; check cooling.");
  }

  // GPU temp
  if (data.gpu && data.gpu.available && Array.isArray(data.gpu.gpus)) {
    for (const g of data.gpu.gpus) {
      if (typeof g.temperature_c !== "number") continue;
      if (g.temperature_c >= t.gpuTempCrit) add("CRITICAL", "gpu-temp", `${g.name} at ${g.temperature_c}°C`, "Check GPU fans and case airflow.");
      else if (g.temperature_c >= t.gpuTempWarn) add("WARNING", "gpu-temp", `${g.name} at ${g.temperature_c}°C`, "Monitor GPU temps under load.");
    }
  }

  // Section availability notes
  const sections = {
    drives: data.drives ? (data.drives.reports ? "ok" : (data.drives.note || "unavailable")) : "not_collected",
    disks: data.disks ? "ok" : "not_collected",
    memory: data.memory ? "ok" : "not_collected",
    cpu: data.cpu ? "ok" : "not_collected",
    gpu: data.gpu ? (data.gpu.available ? "ok" : data.gpu.reason) : "not_collected",
    sensors: data.sensors ? (data.sensors.available ? "ok" : data.sensors.reason) : "not_collected",
  };

  issues.sort((a, b) => RANK[b.severity] - RANK[a.severity]);
  const summary = overall === "OK"
    ? "All checked systems look healthy."
    : `${issues.length} issue(s) found — highest severity: ${overall}.`;

  return { overall, summary, issues, sections };
}

function pickCpuTemp(data) {
  if (data.sensors && data.sensors.available && data.sensors.by_category && Array.isArray(data.sensors.by_category.temperature)) {
    const temps = data.sensors.by_category.temperature;
    const pkg = temps.find((s) => /package|cpu/i.test(s.name || ""));
    const cand = pkg || temps[0];
    if (cand && typeof cand.value === "number") return cand.value;
  }
  if (data.cpu && typeof data.cpu.temperature_c === "number") return data.cpu.temperature_c;
  return null;
}

async function getHealthReport() {
  const settle = async (p) => { try { return await p; } catch (e) { return { error: e.message }; } };
  const [cpu, memory, disks, drives, gpuData, sensorData] = await Promise.all([
    settle(metrics.getCpu()),
    settle(metrics.getMemory()),
    settle(metrics.getDisks()),
    settle(smart.getDriveHealth({})),
    settle(gpu.getGpu()),
    settle(sensors.getSensors()),
  ]);
  const data = { cpu, memory, disks, drives, gpu: gpuData, sensors: sensorData };
  return { ...evaluateHealth(data), collected: data };
}

module.exports = { evaluateHealth, getHealthReport, THRESHOLDS };
