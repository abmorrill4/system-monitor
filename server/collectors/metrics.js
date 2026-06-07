"use strict";
/* General system metrics via the systeminformation library. */

const si = require("systeminformation");

const round = (n, d = 1) => (typeof n === "number" ? +n.toFixed(d) : n);
const gb = (bytes) => (typeof bytes === "number" ? +(bytes / 1e9).toFixed(2) : null);
const pct = (part, whole) => (whole ? +((part / whole) * 100).toFixed(1) : null);

async function getSystemInfo() {
  const [osInfo, system, bios, baseboard, cpu, time] = await Promise.all([
    si.osInfo(), si.system(), si.bios(), si.baseboard(), si.cpu(), Promise.resolve(si.time()),
  ]);
  return {
    hostname: osInfo.hostname,
    os: `${osInfo.distro} ${osInfo.release}`.trim(),
    kernel: osInfo.kernel,
    arch: osInfo.arch,
    uptime_hours: round((time.uptime || 0) / 3600, 1),
    manufacturer: system.manufacturer || baseboard.manufacturer || null,
    model: system.model || baseboard.model || null,
    motherboard: baseboard.manufacturer ? `${baseboard.manufacturer} ${baseboard.model}`.trim() : null,
    bios: bios.vendor ? `${bios.vendor} ${bios.version} (${bios.releaseDate})` : null,
    cpu_model: `${cpu.manufacturer} ${cpu.brand}`.trim(),
    cpu_cores: cpu.physicalCores,
    cpu_threads: cpu.cores,
  };
}

async function getCpu(args) {
  args = args || {};
  const [cpu, load, speed, temp] = await Promise.all([
    si.cpu(), si.currentLoad(), si.cpuCurrentSpeed(), si.cpuTemperature().catch(() => ({})),
  ]);
  const out = {
    model: `${cpu.manufacturer} ${cpu.brand}`.trim(),
    physical_cores: cpu.physicalCores,
    logical_cores: cpu.cores,
    speed_ghz: speed.avg || cpu.speed,
    load_percent: round(load.currentLoad),
    load_user_percent: round(load.currentLoadUser),
    load_system_percent: round(load.currentLoadSystem),
    temperature_c: typeof temp.main === "number" && temp.main > 0 ? temp.main : null,
  };
  if (args.per_core) {
    out.per_core = (load.cpus || []).map((c, i) => ({ core: i, load_percent: round(c.load) }));
  }
  if (out.temperature_c == null) out.temperature_note = "CPU temp unavailable from systeminformation on Windows; use get_sensors (LibreHardwareMonitor) for accurate temps.";
  return out;
}

async function getMemory() {
  const m = await si.mem();
  return {
    total_gb: gb(m.total),
    used_gb: gb(m.active),
    free_gb: gb(m.available),
    used_percent: pct(m.active, m.total),
    swap_total_gb: gb(m.swaptotal),
    swap_used_gb: gb(m.swapused),
    swap_used_percent: pct(m.swapused, m.swaptotal),
  };
}

async function getDisks() {
  const fs = await si.fsSize();
  const volumes = fs
    .filter((d) => d.size > 0)
    .map((d) => ({
      mount: d.mount, fs: d.fs, type: d.type,
      size_gb: gb(d.size), used_gb: gb(d.used), free_gb: gb(d.available),
      used_percent: round(d.use),
    }));
  return { volumes, count: volumes.length };
}

async function getDiskIO() {
  const [io, stats] = await Promise.all([si.disksIO().catch(() => ({})), si.fsStats().catch(() => ({}))]);
  return {
    read_per_sec_mb: round((io.rIO_sec || 0) / 1e6, 2),
    write_per_sec_mb: round((io.wIO_sec || 0) / 1e6, 2),
    total_read_mb_per_sec: round((stats.rx_sec || 0) / 1e6, 2),
    total_write_mb_per_sec: round((stats.wx_sec || 0) / 1e6, 2),
    note: "Per-disk I/O is partial on Windows; values are system-wide rates sampled briefly.",
  };
}

async function getNetwork() {
  const [ifaces, stats] = await Promise.all([si.networkInterfaces(), si.networkStats().catch(() => [])]);
  const statByIface = {};
  for (const s of stats) statByIface[s.iface] = s;
  const interfaces = ifaces
    .filter((i) => !i.internal && (i.ip4 || i.ip6))
    .map((i) => {
      const s = statByIface[i.iface] || {};
      return {
        name: i.iface, ip4: i.ip4 || null, ip6: i.ip6 || null, mac: i.mac || null,
        state: i.operstate, speed_mbps: i.speed || null, type: i.type,
        rx_mb_per_sec: round((s.rx_sec || 0) / 1e6, 2),
        tx_mb_per_sec: round((s.tx_sec || 0) / 1e6, 2),
      };
    });
  return { interfaces, count: interfaces.length };
}

async function getTopProcesses(args) {
  args = args || {};
  const limit = Math.min(Math.max(parseInt(args.limit, 10) || 10, 1), 50);
  const sortBy = ["cpu", "memory", "mem"].includes(args.sort_by) ? args.sort_by : "cpu";
  const p = await si.processes();
  const list = (p.list || []).slice();
  list.sort((a, b) => (sortBy === "cpu" ? b.cpu - a.cpu : b.mem - a.mem));
  return {
    total_processes: p.all,
    sorted_by: sortBy,
    processes: list.slice(0, limit).map((x) => ({
      pid: x.pid, name: x.name, cpu_percent: round(x.cpu), mem_percent: round(x.mem), command: x.command,
    })),
  };
}

module.exports = { getSystemInfo, getCpu, getMemory, getDisks, getDiskIO, getNetwork, getTopProcesses };
