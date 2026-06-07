"use strict";
/* GPU metrics via nvidia-smi (NVIDIA only). Zero-dependency. */

const { execFile } = require("child_process");

const QUERY = [
  "index", "name", "utilization.gpu", "memory.used", "memory.total",
  "temperature.gpu", "power.draw", "power.limit", "fan.speed", "clocks.sm",
];

const num = (v) => {
  const n = parseFloat(String(v).trim());
  return Number.isFinite(n) ? n : null;
};

/* Pure: parse nvidia-smi CSV (noheader, nounits) into structured GPU objects. */
function parseNvidiaSmi(csv) {
  return String(csv)
    .trim()
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean)
    .map((line) => {
      const f = line.split(",").map((x) => x.trim());
      return {
        index: num(f[0]),
        name: f[1] || null,
        utilization_percent: num(f[2]),
        memory_used_mb: num(f[3]),
        memory_total_mb: num(f[4]),
        temperature_c: num(f[5]),
        power_draw_w: num(f[6]),
        power_limit_w: num(f[7]),
        fan_percent: num(f[8]),
        clock_sm_mhz: num(f[9]),
      };
    });
}

function runNvidiaSmi() {
  return new Promise((resolve, reject) => {
    execFile(
      process.env.NVIDIA_SMI_PATH || "nvidia-smi",
      ["--query-gpu=" + QUERY.join(","), "--format=csv,noheader,nounits"],
      { maxBuffer: 2 * 1024 * 1024, windowsHide: true },
      (err, stdout) => {
        if (err && err.code === "ENOENT") return reject(Object.assign(new Error("nvidia-smi not found"), { kind: "not_found" }));
        if (err) return reject(err);
        resolve(stdout || "");
      }
    );
  });
}

async function getGpu() {
  try {
    const csv = await runNvidiaSmi();
    const gpus = parseNvidiaSmi(csv);
    return { available: true, vendor: "NVIDIA", count: gpus.length, gpus };
  } catch (e) {
    if (e.kind === "not_found") {
      return { available: false, reason: "No NVIDIA GPU detected (nvidia-smi not found). AMD/Intel GPU metrics are not yet supported." };
    }
    return { available: false, reason: "nvidia-smi error: " + e.message };
  }
}

module.exports = { parseNvidiaSmi, getGpu };
