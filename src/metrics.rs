// General system metrics via the `sysinfo` crate: host info, CPU, memory,
// disks, disk I/O, network, and top processes. Replaces the old
// systeminformation-backed collector. All functions degrade to nulls rather
// than panicking, and CPU/IO/network rates are sampled over a short interval.

use std::time::Duration;

use serde_json::{json, Value};
use sysinfo::{Components, DiskKind, Disks, Networks, Pid, ProcessRefreshKind, ProcessesToUpdate, System};

use crate::util::{gb, numv, pct, round, powershell_json};

/// Sampling window for rate-based metrics (CPU load, disk I/O, network throughput).
const SAMPLE: Duration = Duration::from_millis(300);

pub fn get_system_info() -> Value {
    let mut sys = System::new();
    sys.refresh_cpu_all();
    let cpus = sys.cpus();
    let brand = cpus.first().map(|c| c.brand().trim().to_string()).unwrap_or_default();
    let hw = win_hardware();

    json!({
        "hostname": System::host_name(),
        "os": System::long_os_version().or_else(System::name),
        "kernel": System::kernel_version(),
        "arch": System::cpu_arch(),
        "uptime_hours": numv(round(System::uptime() as f64 / 3600.0, 1)),
        "manufacturer": hw.get("manufacturer").cloned().unwrap_or(Value::Null),
        "model": hw.get("model").cloned().unwrap_or(Value::Null),
        "motherboard": hw.get("motherboard").cloned().unwrap_or(Value::Null),
        "bios": hw.get("bios").cloned().unwrap_or(Value::Null),
        "cpu_model": brand,
        "cpu_cores": System::physical_core_count(),
        "cpu_threads": cpus.len(),
    })
}

/// Baseboard / BIOS / system manufacturer+model. sysinfo does not expose these,
/// so on Windows we read them from WMI. Returns an empty map elsewhere or on failure.
fn win_hardware() -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    let script = "$cs=Get-CimInstance Win32_ComputerSystem; \
$bb=Get-CimInstance Win32_BaseBoard; $b=Get-CimInstance Win32_BIOS; \
[pscustomobject]@{ manufacturer=$cs.Manufacturer; model=$cs.Model; \
bbMan=$bb.Manufacturer; bbProd=$bb.Product; biosMan=$b.Manufacturer; \
biosVer=$b.SMBIOSBIOSVersion } | ConvertTo-Json -Compress";
    let Some(v) = powershell_json(script) else { return map };
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(|x| x.trim().to_string()).filter(|x| !x.is_empty());
    if let Some(m) = s("manufacturer") { map.insert("manufacturer".into(), json!(m)); }
    if let Some(m) = s("model") { map.insert("model".into(), json!(m)); }
    if let (Some(man), Some(prod)) = (s("bbMan"), s("bbProd")) {
        map.insert("motherboard".into(), json!(format!("{} {}", man, prod)));
    }
    if let Some(ver) = s("biosVer") {
        let man = s("biosMan").map(|m| format!("{} ", m)).unwrap_or_default();
        map.insert("bios".into(), json!(format!("{}{}", man, ver)));
    }
    map
}

pub fn get_cpu(per_core: bool) -> Value {
    let mut sys = System::new();
    sys.refresh_cpu_all();
    std::thread::sleep(SAMPLE);
    sys.refresh_cpu_usage();

    let cpus = sys.cpus();
    let brand = cpus.first().map(|c| c.brand().trim().to_string()).unwrap_or_default();
    let speed_mhz = cpus.first().map(|c| c.frequency()).unwrap_or(0);
    let temp = cpu_temperature();

    let mut out = json!({
        "model": brand,
        "physical_cores": System::physical_core_count(),
        "logical_cores": cpus.len(),
        "speed_ghz": numv(round(speed_mhz as f64 / 1000.0, 2)),
        "load_percent": numv(round(sys.global_cpu_usage() as f64, 1)),
        "temperature_c": temp,
    });

    if per_core {
        let per: Vec<Value> = cpus
            .iter()
            .enumerate()
            .map(|(i, c)| json!({ "core": i, "load_percent": numv(round(c.cpu_usage() as f64, 1)) }))
            .collect();
        out["per_core"] = json!(per);
    }
    if out["temperature_c"].is_null() {
        out["temperature_note"] = json!(
            "CPU temp unavailable from sysinfo on Windows; use get_sensors (LibreHardwareMonitor) for accurate temps."
        );
    }
    out
}

/// Best-effort CPU temperature from OS thermal components. Usually empty on
/// Windows desktops (the WMI thermal zone is unpopulated); get_sensors is the
/// accurate path there.
fn cpu_temperature() -> Value {
    let comps = Components::new_with_refreshed_list();
    let mut fallback: Option<f32> = None;
    for c in &comps {
        let Some(t) = c.temperature() else { continue };
        if !t.is_finite() {
            continue;
        }
        let label = c.label().to_lowercase();
        if label.contains("package") || label.contains("cpu") || label.contains("tctl") || label.contains("tdie") {
            return numv(round(t as f64, 1));
        }
        if fallback.is_none() {
            fallback = Some(t);
        }
    }
    match fallback {
        Some(t) => numv(round(t as f64, 1)),
        None => Value::Null,
    }
}

pub fn get_memory() -> Value {
    let mut sys = System::new();
    sys.refresh_memory();
    let total = sys.total_memory() as f64;
    let used = sys.used_memory() as f64;
    let avail = sys.available_memory() as f64;
    let swap_total = sys.total_swap() as f64;
    let swap_used = sys.used_swap() as f64;
    json!({
        "total_gb": gb(total),
        "used_gb": gb(used),
        "free_gb": gb(avail),
        "used_percent": pct(used, total),
        "swap_total_gb": gb(swap_total),
        "swap_used_gb": gb(swap_used),
        "swap_used_percent": pct(swap_used, swap_total),
    })
}

pub fn get_disks() -> Value {
    let disks = Disks::new_with_refreshed_list();
    let mut volumes = Vec::new();
    for d in disks.list() {
        let size = d.total_space() as f64;
        if size <= 0.0 {
            continue;
        }
        let avail = d.available_space() as f64;
        let used = size - avail;
        volumes.push(json!({
            "mount": d.mount_point().to_string_lossy(),
            "fs": d.file_system().to_string_lossy(),
            "type": disk_kind(d.kind()),
            "size_gb": gb(size),
            "used_gb": gb(used),
            "free_gb": gb(avail),
            "used_percent": pct(used, size),
        }));
    }
    let count = volumes.len();
    json!({ "volumes": volumes, "count": count })
}

fn disk_kind(kind: DiskKind) -> &'static str {
    match kind {
        DiskKind::HDD => "HDD",
        DiskKind::SSD => "SSD",
        _ => "Unknown",
    }
}

pub fn get_disk_io() -> Value {
    let mut disks = Disks::new_with_refreshed_list();
    std::thread::sleep(SAMPLE);
    disks.refresh(false);
    let (mut read, mut written) = (0u64, 0u64);
    for d in disks.list() {
        let u = d.usage();
        read += u.read_bytes;
        written += u.written_bytes;
    }
    let secs = SAMPLE.as_secs_f64();
    json!({
        "read_per_sec_mb": numv(round((read as f64 / secs) / 1e6, 2)),
        "write_per_sec_mb": numv(round((written as f64 / secs) / 1e6, 2)),
        "note": "System-wide disk I/O sampled over a brief interval; per-disk attribution is partial on Windows.",
    })
}

pub fn get_network() -> Value {
    let mut nets = Networks::new_with_refreshed_list();
    std::thread::sleep(SAMPLE);
    nets.refresh(false);
    let secs = SAMPLE.as_secs_f64();

    let mut interfaces = Vec::new();
    for (name, data) in nets.list() {
        let (mut ip4, mut ip6): (Option<String>, Option<String>) = (None, None);
        for ipn in data.ip_networks() {
            match ipn.addr {
                std::net::IpAddr::V4(a) if !a.is_loopback() && ip4.is_none() => ip4 = Some(a.to_string()),
                std::net::IpAddr::V6(a) if !a.is_loopback() && ip6.is_none() => ip6 = Some(a.to_string()),
                _ => {}
            }
        }
        if ip4.is_none() && ip6.is_none() {
            continue;
        }
        interfaces.push(json!({
            "name": name,
            "ip4": ip4,
            "ip6": ip6,
            "mac": data.mac_address().to_string(),
            "rx_mb_per_sec": numv(round((data.received() as f64 / secs) / 1e6, 2)),
            "tx_mb_per_sec": numv(round((data.transmitted() as f64 / secs) / 1e6, 2)),
        }));
    }
    let count = interfaces.len();
    json!({ "interfaces": interfaces, "count": count })
}

pub fn get_top_processes(limit: usize, sort_by: &str) -> Value {
    let limit = limit.clamp(1, 50);
    let by_mem = sort_by == "memory" || sort_by == "mem";

    // CPU usage per process sums across cores in sysinfo; normalize by logical
    // core count so it reads as a share of the whole machine (0-100), like the
    // overall CPU load and Task Manager.
    let ncpu = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64;

    let mut sys = System::new();
    sys.refresh_memory();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::everything());
    std::thread::sleep(SAMPLE);
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::everything());

    let total_mem = sys.total_memory() as f64;
    let mut list: Vec<(&Pid, _)> = sys.processes().iter().collect();
    if by_mem {
        list.sort_by(|a, b| b.1.memory().cmp(&a.1.memory()));
    } else {
        list.sort_by(|a, b| {
            b.1.cpu_usage()
                .partial_cmp(&a.1.cpu_usage())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    let processes: Vec<Value> = list
        .iter()
        .take(limit)
        .map(|(pid, p)| {
            let cmd = p.cmd().iter().map(|s| s.to_string_lossy()).collect::<Vec<_>>().join(" ");
            let command = if cmd.is_empty() {
                p.exe().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()
            } else {
                cmd
            };
            json!({
                "pid": pid.as_u32(),
                "name": p.name().to_string_lossy(),
                "cpu_percent": numv(round(p.cpu_usage() as f64 / ncpu, 1)),
                "mem_percent": pct(p.memory() as f64, total_mem),
                "command": command,
            })
        })
        .collect();

    json!({
        "total_processes": sys.processes().len(),
        "sorted_by": if by_mem { "memory" } else { "cpu" },
        "processes": processes,
    })
}
