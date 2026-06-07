"use strict";
/* SMART drive health via smartctl (smartmontools). Zero-dependency. */

const { execFile } = require("child_process");
const fs = require("fs");

function findSmartctl() {
  if (process.env.SMARTCTL_PATH && fs.existsSync(process.env.SMARTCTL_PATH)) return process.env.SMARTCTL_PATH;
  const candidates = [
    "C:\\Program Files\\smartmontools\\bin\\smartctl.exe",
    "C:\\Program Files (x86)\\smartmontools\\bin\\smartctl.exe",
    "/usr/sbin/smartctl", "/usr/local/sbin/smartctl", "/usr/bin/smartctl", "/opt/homebrew/bin/smartctl",
  ];
  for (const c of candidates) { try { if (fs.existsSync(c)) return c; } catch (_) {} }
  return "smartctl";
}
const SMARTCTL = findSmartctl();

class SmartctlError extends Error {
  constructor(message, kind) { super(message); this.kind = kind || "error"; }
}

function runSmartctl(args) {
  return new Promise((resolve, reject) => {
    execFile(SMARTCTL, args, { maxBuffer: 8 * 1024 * 1024, windowsHide: true }, (err, stdout, stderr) => {
      if (err && err.code === "ENOENT") {
        return reject(new SmartctlError(
          "smartctl not found. Install smartmontools (`winget install smartmontools.smartmontools`) or set SMARTCTL_PATH.",
          "not_installed"));
      }
      resolve({ stdout: stdout || "", stderr: stderr || "" });
    });
  });
}
async function runSmartctlJSON(args) {
  const { stdout } = await runSmartctl(["-j", ...args]);
  try { return JSON.parse(stdout); }
  catch (_) { throw new SmartctlError("Could not parse smartctl JSON: " + stdout.slice(0, 300)); }
}

function collectMessages(json) {
  const msgs = (json && json.smartctl && json.smartctl.messages) || [];
  return msgs.map((m) => (m.severity ? m.severity + ": " : "") + (m.string || "")).filter(Boolean);
}
function looksLikePermissionIssue(messages) {
  const b = messages.join(" ").toLowerCase();
  return b.includes("permission") || b.includes("administrator") || b.includes("access is denied") ||
    b.includes("operation not permitted") || (b.includes("requires") && b.includes("privile"));
}
function attrById(t, id) { return Array.isArray(t) ? t.find((a) => a.id === id) : undefined; }
function attrByName(t, names) {
  if (!Array.isArray(t)) return undefined;
  const lc = names.map((n) => n.toLowerCase());
  return t.find((a) => lc.includes((a.name || "").toLowerCase()));
}
function rawval(a) { return a && a.raw && typeof a.raw.value === "number" ? a.raw.value : undefined; }

function summarize(json) {
  const messages = collectMessages(json);
  const s = {
    device: (json.device && json.device.name) || null,
    protocol: (json.device && json.device.protocol) || null,
    model: json.model_name || json.scsi_model_name || null,
    serial: json.serial_number || null,
    firmware: json.firmware_version || null,
    capacity_bytes: (json.user_capacity && json.user_capacity.bytes) || null,
    smart_passed: (json.smart_status && typeof json.smart_status.passed === "boolean") ? json.smart_status.passed : null,
    temperature_c: (json.temperature && typeof json.temperature.current === "number") ? json.temperature.current : null,
    power_on_hours: (json.power_on_time && typeof json.power_on_time.hours === "number") ? json.power_on_time.hours : null,
    power_cycles: (typeof json.power_cycle_count === "number") ? json.power_cycle_count : null,
    messages, warnings: [], verdict: "UNKNOWN",
  };

  if (json.nvme_smart_health_information_log) {
    const n = json.nvme_smart_health_information_log;
    s.kind = "NVMe";
    if (typeof n.percentage_used === "number") s.wear_percent_used = n.percentage_used;
    if (typeof n.available_spare === "number") s.available_spare_pct = n.available_spare;
    if (typeof n.available_spare_threshold === "number") s.available_spare_threshold_pct = n.available_spare_threshold;
    if (typeof n.media_errors === "number") s.media_errors = n.media_errors;
    if (typeof n.unsafe_shutdowns === "number") s.unsafe_shutdowns = n.unsafe_shutdowns;
    if (typeof n.critical_warning === "number") s.critical_warning = n.critical_warning;
    if (typeof n.data_units_written === "number") s.host_written_tb = +((n.data_units_written * 512000) / 1e12).toFixed(2);
    if (s.temperature_c == null && typeof n.temperature === "number") s.temperature_c = n.temperature;
    if (s.critical_warning) s.warnings.push("NVMe critical_warning flag set (0x" + s.critical_warning.toString(16) + ")");
    if (s.wear_percent_used >= 90) s.warnings.push("Wear at " + s.wear_percent_used + "% of rated endurance");
    if (typeof s.available_spare_pct === "number" && s.available_spare_pct < s.available_spare_threshold_pct) s.warnings.push("Available spare below threshold");
    if (s.media_errors > 0) s.warnings.push(s.media_errors + " media/data-integrity errors");
  }

  if (json.ata_smart_attributes && json.ata_smart_attributes.table) {
    const t = json.ata_smart_attributes.table;
    s.kind = s.kind || "SATA/ATA";
    const realloc = rawval(attrById(t, 5)) ?? rawval(attrByName(t, ["Reallocated_Sector_Ct"]));
    const pending = rawval(attrById(t, 197)) ?? rawval(attrByName(t, ["Current_Pending_Sector"]));
    const uncorr = rawval(attrById(t, 198)) ?? rawval(attrByName(t, ["Offline_Uncorrectable"]));
    const crc = rawval(attrById(t, 199)) ?? rawval(attrByName(t, ["UDMA_CRC_Error_Count"]));
    const wear = rawval(attrByName(t, ["Wear_Leveling_Count", "Media_Wearout_Indicator", "SSD_Life_Left", "Percent_Lifetime_Remain"]));
    if (realloc != null) s.reallocated_sectors = realloc;
    if (pending != null) s.pending_sectors = pending;
    if (uncorr != null) s.offline_uncorrectable = uncorr;
    if (crc != null) s.crc_errors = crc;
    if (wear != null) s.ssd_wear_indicator = wear;
    if (realloc > 0) s.warnings.push(realloc + " reallocated sectors");
    if (pending > 0) s.warnings.push(pending + " current pending sectors");
    if (uncorr > 0) s.warnings.push(uncorr + " offline-uncorrectable sectors");
    if (crc > 0) s.warnings.push(crc + " interface CRC errors (check cable)");
  }

  if (looksLikePermissionIssue(messages) && s.smart_passed == null) s.verdict = "NEEDS_ELEVATION";
  else if (s.smart_passed === false) s.verdict = "FAILING";
  else if (s.warnings.length > 0) s.verdict = "WARNING";
  else if (s.smart_passed === true) s.verdict = "HEALTHY";
  else s.verdict = "UNKNOWN";
  return s;
}

async function listDrives() {
  const scan = await runSmartctlJSON(["--scan"]);
  const devices = (scan.devices || []).map((d) => ({ name: d.name, type: d.type, protocol: d.protocol || null, info_name: d.info_name || null }));
  return { devices, count: devices.length, messages: collectMessages(scan), smartctl_path: SMARTCTL };
}

async function getDriveHealth(args) {
  args = args || {};
  let targets;
  if (args.device) targets = [{ name: args.device, type: args.type || null }];
  else {
    const scan = await runSmartctlJSON(["--scan"]);
    targets = (scan.devices || []).map((d) => ({ name: d.name, type: d.type }));
    if (targets.length === 0) return { reports: [], note: "smartctl --scan found no devices — usually means the server is not running as Administrator.", smartctl_path: SMARTCTL };
  }
  const reports = [];
  for (const t of targets) {
    const a = ["-a"]; if (t.type) a.push("-d", t.type); a.push(t.name);
    try { reports.push(summarize(await runSmartctlJSON(a))); }
    catch (e) { reports.push({ device: t.name, verdict: "ERROR", error: e.message }); }
  }
  return { reports, count: reports.length, smartctl_path: SMARTCTL };
}

async function runSelfTest(args) {
  args = args || {};
  if (!args.device) throw new SmartctlError("`device` is required for run_self_test.");
  const kind = args.kind === "long" ? "long" : "short";
  const a = ["-t", kind]; if (args.type) a.push("-d", args.type); a.push(args.device);
  const { stdout } = await runSmartctl(a);
  return { device: args.device, test: kind, output: stdout.trim(),
    note: "Self-test runs in the background on the drive. Re-run get_drive_health later to read results." };
}

module.exports = { findSmartctl, summarize, listDrives, getDriveHealth, runSelfTest, SmartctlError, SMARTCTL };
