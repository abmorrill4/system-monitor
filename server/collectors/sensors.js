"use strict";
/* Hardware sensors (temps/fans/voltages/clocks/load/power) via LibreHardwareMonitor.
   Primary: LHM web server JSON (default http://localhost:8085/data.json).
   Fallback (Windows): WMI namespace root/LibreHardwareMonitor via PowerShell.
   Zero-dependency. Degrades gracefully when LHM is not running. */

const { execFile } = require("child_process");

const LHM_URL = process.env.LHM_URL || "http://localhost:8085/data.json";

// Map LHM group/SensorType labels to a normalized category.
const CATEGORY = {
  temperature: "temperature", temperatures: "temperature",
  fan: "fan", fans: "fan",
  voltage: "voltage", voltages: "voltage",
  clock: "clock", clocks: "clock",
  load: "load",
  power: "power", powers: "power",
  data: "data", throughput: "throughput",
  level: "level", levels: "level", control: "control", controls: "control",
};

/* Pure: parse a LHM value string like "45.0 C" / "1200 RPM" / "1.20 V" -> {value, unit}. */
function parseLhmValue(str) {
  if (str == null) return { value: null, unit: null };
  const m = String(str).trim().match(/^(-?[\d.]+)\s*(.*)$/);
  if (!m) return { value: null, unit: String(str).trim() || null };
  const value = parseFloat(m[1]);
  return { value: Number.isFinite(value) ? value : null, unit: (m[2] || "").trim() || null };
}

/* Pure: flatten the LHM data.json tree into a list of normalized sensors. */
function flattenLhm(tree) {
  const out = [];
  const walk = (node, hardware, category) => {
    if (!node || typeof node !== "object") return;
    const text = node.Text || "";
    const cat = CATEGORY[text.toLowerCase()];
    const nextCategory = cat || category;
    const children = node.Children || [];
    if (children.length === 0 && node.Value != null && node.Value !== "") {
      const { value, unit } = parseLhmValue(node.Value);
      if (value != null) {
        out.push({
          hardware: hardware || null,
          category: nextCategory || null,
          name: text || null,
          value, unit,
          min: parseLhmValue(node.Min).value,
          max: parseLhmValue(node.Max).value,
        });
      }
      return;
    }
    for (const c of children) walk(c, hardware, nextCategory);
  };
  const root = tree && (tree.Children ? tree : { Children: [tree] });
  for (const computer of (root.Children || [])) {
    for (const hw of (computer.Children || [])) {
      walk(hw, hw.Text || null, null);
    }
  }
  return out;
}

/* Pure: normalize WMI Sensor rows (from PowerShell ConvertTo-Json). */
function normalizeWmiSensors(rows) {
  const arr = Array.isArray(rows) ? rows : (rows ? [rows] : []);
  const UNIT = { Temperature: "C", Fan: "RPM", Voltage: "V", Clock: "MHz", Load: "%", Power: "W", Data: "GB", Throughput: "B/s", Level: "%" };
  return arr.map((r) => ({
    hardware: r.Parent || null,
    category: CATEGORY[String(r.SensorType || "").toLowerCase()] || (r.SensorType ? String(r.SensorType).toLowerCase() : null),
    name: r.Name || null,
    value: typeof r.Value === "number" ? +r.Value.toFixed(2) : (Number.isFinite(parseFloat(r.Value)) ? +parseFloat(r.Value).toFixed(2) : null),
    unit: UNIT[r.SensorType] || null,
  })).filter((s) => s.value != null);
}

function groupByCategory(sensors) {
  const g = {};
  for (const s of sensors) { (g[s.category || "other"] ||= []).push(s); }
  return g;
}

async function fetchHttp() {
  if (typeof fetch !== "function") throw Object.assign(new Error("fetch unavailable"), { kind: "no_fetch" });
  const ctrl = new AbortController();
  const t = setTimeout(() => ctrl.abort(), 2500);
  try {
    const res = await fetch(LHM_URL, { signal: ctrl.signal });
    if (!res.ok) throw new Error("HTTP " + res.status);
    return await res.json();
  } finally { clearTimeout(t); }
}

function fetchWmi() {
  return new Promise((resolve, reject) => {
    if (process.platform !== "win32") return reject(Object.assign(new Error("WMI only on Windows"), { kind: "no_wmi" }));
    const ps = "Get-CimInstance -Namespace root/LibreHardwareMonitor -ClassName Sensor -ErrorAction Stop | " +
      "Select-Object Name,SensorType,Value,Parent | ConvertTo-Json -Compress";
    execFile("powershell", ["-NoProfile", "-NonInteractive", "-Command", ps],
      { maxBuffer: 8 * 1024 * 1024, windowsHide: true }, (err, stdout) => {
        if (err) return reject(err);
        try { resolve(JSON.parse(stdout || "[]")); } catch (e) { reject(new Error("WMI parse failed")); }
      });
  });
}

async function getSensors() {
  try {
    const tree = await fetchHttp();
    const list = flattenLhm(tree);
    if (list.length) return { available: true, source: "lhm-web", url: LHM_URL, count: list.length, by_category: groupByCategory(list) };
  } catch (_) { /* fall through */ }
  try {
    const rows = await fetchWmi();
    const list = normalizeWmiSensors(rows);
    if (list.length) return { available: true, source: "lhm-wmi", count: list.length, by_category: groupByCategory(list) };
  } catch (_) { /* fall through */ }
  return {
    available: false,
    reason: "No hardware sensor source found. Install LibreHardwareMonitor, run it as Administrator, and either enable its web server (Options -> Remote Web Server, port 8085) or leave it running for WMI access. Set LHM_URL to override the web server address.",
  };
}

module.exports = { parseLhmValue, flattenLhm, normalizeWmiSensors, getSensors };
