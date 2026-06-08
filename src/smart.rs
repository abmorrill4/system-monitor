// SMART drive health via smartctl (smartmontools). Zero extra deps: shells out
// to `smartctl -j` and parses the JSON. summarize() is pure and unit-tested.
// Reading SMART data on Windows requires Administrator; the verdict surfaces a
// NEEDS_ELEVATION state when smartctl reports a permission problem.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::util::numv;

pub struct SmartError {
    pub message: String,
    pub kind: String,
}

impl SmartError {
    fn new(message: impl Into<String>, kind: &str) -> Self {
        SmartError { message: message.into(), kind: kind.to_string() }
    }
}

fn smartctl_path() -> String {
    if let Ok(p) = std::env::var("SMARTCTL_PATH") {
        if Path::new(&p).exists() {
            return p;
        }
    }
    const CANDIDATES: &[&str] = &[
        r"C:\Program Files\smartmontools\bin\smartctl.exe",
        r"C:\Program Files (x86)\smartmontools\bin\smartctl.exe",
        "/usr/sbin/smartctl",
        "/usr/local/sbin/smartctl",
        "/usr/bin/smartctl",
        "/opt/homebrew/bin/smartctl",
    ];
    for c in CANDIDATES {
        if Path::new(c).exists() {
            return c.to_string();
        }
    }
    "smartctl".to_string()
}

fn run_smartctl(args: &[String]) -> Result<(String, String), SmartError> {
    match std::process::Command::new(smartctl_path()).args(args).output() {
        Ok(out) => Ok((
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(SmartError::new(
            "smartctl not found. Install smartmontools (`winget install smartmontools.smartmontools`) or set SMARTCTL_PATH.",
            "not_installed",
        )),
        Err(e) => Err(SmartError::new(e.to_string(), "error")),
    }
}

fn run_smartctl_json(args: &[String]) -> Result<Value, SmartError> {
    let mut full = vec!["-j".to_string()];
    full.extend_from_slice(args);
    let (stdout, _) = run_smartctl(&full)?;
    serde_json::from_str(&stdout).map_err(|_| {
        let snippet: String = stdout.chars().take(300).collect();
        SmartError::new(format!("Could not parse smartctl JSON: {}", snippet), "error")
    })
}

fn collect_messages(json: &Value) -> Vec<String> {
    json.pointer("/smartctl/messages")
        .and_then(|v| v.as_array())
        .map(|msgs| {
            msgs.iter()
                .filter_map(|m| {
                    let string = m.get("string").and_then(|v| v.as_str()).unwrap_or("");
                    if string.is_empty() {
                        return None;
                    }
                    let sev = m.get("severity").and_then(|v| v.as_str());
                    Some(match sev {
                        Some(s) => format!("{}: {}", s, string),
                        None => string.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn looks_like_permission_issue(messages: &[String]) -> bool {
    let b = messages.join(" ").to_lowercase();
    b.contains("permission")
        || b.contains("administrator")
        || b.contains("access is denied")
        || b.contains("operation not permitted")
        || (b.contains("requires") && b.contains("privile"))
}

fn attr_by_id<'a>(table: &'a [Value], id: i64) -> Option<&'a Value> {
    table.iter().find(|a| a.get("id").and_then(|v| v.as_i64()) == Some(id))
}

fn attr_by_name<'a>(table: &'a [Value], names: &[&str]) -> Option<&'a Value> {
    let lc: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
    table.iter().find(|a| {
        let name = a.get("name").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
        lc.iter().any(|n| n == &name)
    })
}

fn rawval(attr: Option<&Value>) -> Option<f64> {
    attr?.pointer("/raw/value").and_then(|v| v.as_f64())
}

fn as_int(v: f64) -> i64 {
    v as i64
}

/// Pure: summarize a `smartctl -a -j` JSON document into a normalized health report.
pub fn summarize(json: &Value) -> Value {
    let messages = collect_messages(json);
    let mut warnings: Vec<String> = Vec::new();

    let smart_passed = json.pointer("/smart_status/passed").and_then(|v| v.as_bool());

    let mut s = json!({
        "device": json.pointer("/device/name").and_then(|v| v.as_str()),
        "protocol": json.pointer("/device/protocol").and_then(|v| v.as_str()),
        "model": json.get("model_name").and_then(|v| v.as_str())
            .or_else(|| json.get("scsi_model_name").and_then(|v| v.as_str())),
        "serial": json.get("serial_number").and_then(|v| v.as_str()),
        "firmware": json.get("firmware_version").and_then(|v| v.as_str()),
        "capacity_bytes": json.pointer("/user_capacity/bytes").and_then(|v| v.as_i64()),
        "smart_passed": smart_passed,
        "temperature_c": json.pointer("/temperature/current").and_then(|v| v.as_f64()).map(numv),
        "power_on_hours": json.pointer("/power_on_time/hours").and_then(|v| v.as_i64()),
        "power_cycles": json.get("power_cycle_count").and_then(|v| v.as_i64()),
        "messages": messages.clone(),
    });

    // NVMe health log
    if let Some(n) = json.get("nvme_smart_health_information_log") {
        s["kind"] = json!("NVMe");
        if let Some(v) = n.get("percentage_used").and_then(|v| v.as_f64()) {
            s["wear_percent_used"] = numv(v);
            if v >= 90.0 {
                warnings.push(format!("Wear at {}% of rated endurance", as_int(v)));
            }
        }
        let spare = n.get("available_spare").and_then(|v| v.as_f64());
        let spare_thresh = n.get("available_spare_threshold").and_then(|v| v.as_f64());
        if let Some(v) = spare {
            s["available_spare_pct"] = numv(v);
        }
        if let Some(v) = spare_thresh {
            s["available_spare_threshold_pct"] = numv(v);
        }
        if let (Some(sp), Some(th)) = (spare, spare_thresh) {
            if sp < th {
                warnings.push("Available spare below threshold".to_string());
            }
        }
        if let Some(v) = n.get("media_errors").and_then(|v| v.as_f64()) {
            s["media_errors"] = numv(v);
            if v > 0.0 {
                warnings.push(format!("{} media/data-integrity errors", as_int(v)));
            }
        }
        if let Some(v) = n.get("unsafe_shutdowns").and_then(|v| v.as_f64()) {
            s["unsafe_shutdowns"] = numv(v);
        }
        if let Some(cw) = n.get("critical_warning").and_then(|v| v.as_i64()) {
            s["critical_warning"] = json!(cw);
            if cw != 0 {
                warnings.push(format!("NVMe critical_warning flag set (0x{:x})", cw));
            }
        }
        if let Some(duw) = n.get("data_units_written").and_then(|v| v.as_f64()) {
            s["host_written_tb"] = numv((duw * 512000.0 / 1e12 * 100.0).round() / 100.0);
        }
        if s["temperature_c"].is_null() {
            if let Some(v) = n.get("temperature").and_then(|v| v.as_f64()) {
                s["temperature_c"] = numv(v);
            }
        }
    }

    // ATA/SATA attribute table
    if let Some(table) = json.pointer("/ata_smart_attributes/table").and_then(|v| v.as_array()) {
        if s.get("kind").map(|v| v.is_null()).unwrap_or(true) {
            s["kind"] = json!("SATA/ATA");
        }
        let realloc = rawval(attr_by_id(table, 5)).or_else(|| rawval(attr_by_name(table, &["Reallocated_Sector_Ct"])));
        let pending = rawval(attr_by_id(table, 197)).or_else(|| rawval(attr_by_name(table, &["Current_Pending_Sector"])));
        let uncorr = rawval(attr_by_id(table, 198)).or_else(|| rawval(attr_by_name(table, &["Offline_Uncorrectable"])));
        let crc = rawval(attr_by_id(table, 199)).or_else(|| rawval(attr_by_name(table, &["UDMA_CRC_Error_Count"])));
        let wear = rawval(attr_by_name(table, &["Wear_Leveling_Count", "Media_Wearout_Indicator", "SSD_Life_Left", "Percent_Lifetime_Remain"]));

        if let Some(v) = realloc {
            s["reallocated_sectors"] = numv(v);
            if v > 0.0 {
                warnings.push(format!("{} reallocated sectors", as_int(v)));
            }
        }
        if let Some(v) = pending {
            s["pending_sectors"] = numv(v);
            if v > 0.0 {
                warnings.push(format!("{} current pending sectors", as_int(v)));
            }
        }
        if let Some(v) = uncorr {
            s["offline_uncorrectable"] = numv(v);
            if v > 0.0 {
                warnings.push(format!("{} offline-uncorrectable sectors", as_int(v)));
            }
        }
        if let Some(v) = crc {
            s["crc_errors"] = numv(v);
            if v > 0.0 {
                warnings.push(format!("{} interface CRC errors (check cable)", as_int(v)));
            }
        }
        if let Some(v) = wear {
            s["ssd_wear_indicator"] = numv(v);
        }
    }

    let verdict = if looks_like_permission_issue(&messages) && smart_passed.is_none() {
        "NEEDS_ELEVATION"
    } else if smart_passed == Some(false) {
        "FAILING"
    } else if !warnings.is_empty() {
        "WARNING"
    } else if smart_passed == Some(true) {
        "HEALTHY"
    } else {
        "UNKNOWN"
    };

    s["warnings"] = json!(warnings);
    s["verdict"] = json!(verdict);
    s
}

// ---- Elevated-helper cache ------------------------------------------------
//
// Reading SMART on Windows needs Administrator. So a non-elevated server can
// still report drive health, an elevated scheduled task periodically runs
// `system-monitor --refresh-smart-cache`, which writes a full scan to a shared
// cache file. get_drive_health() falls back to that cache when a live scan
// can't see the drives.

fn cache_path() -> PathBuf {
    if let Ok(p) = std::env::var("SMART_CACHE_PATH") {
        return PathBuf::from(p);
    }
    let base = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".to_string());
    Path::new(&base).join("system-monitor").join("smart-cache.json")
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Run a full drive-health scan and write it to the shared cache file. Invoked
/// by the elevated scheduled-task helper. Returns the path written.
pub fn refresh_cache() -> Result<String, SmartError> {
    let report = get_drive_health_live(&json!({}))?;
    let path = cache_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| SmartError::new(e.to_string(), "error"))?;
    }
    let doc = json!({ "generated_at_unix": now_unix(), "report": report });
    std::fs::write(&path, serde_json::to_vec_pretty(&doc).unwrap_or_default())
        .map_err(|e| SmartError::new(e.to_string(), "error"))?;
    Ok(path.to_string_lossy().into_owned())
}

fn read_cache() -> Option<(Value, u64)> {
    let bytes = std::fs::read(cache_path()).ok()?;
    let doc: Value = serde_json::from_slice(&bytes).ok()?;
    let ts = doc.get("generated_at_unix").and_then(|v| v.as_u64()).unwrap_or(0);
    let report = doc.get("report")?.clone();
    Some((report, now_unix().saturating_sub(ts)))
}

/// True when a live scan actually saw at least one readable drive (not empty,
/// not all NEEDS_ELEVATION / ERROR).
fn live_has_real_reports(v: &Value) -> bool {
    v.get("reports")
        .and_then(|r| r.as_array())
        .map(|reports| {
            !reports.is_empty()
                && reports.iter().any(|r| {
                    !matches!(
                        r.get("verdict").and_then(|x| x.as_str()),
                        Some("NEEDS_ELEVATION") | Some("ERROR") | None
                    )
                })
        })
        .unwrap_or(false)
}

fn annotate_cache(cached: &mut Value, age: u64) {
    cached["source"] = json!("elevated-helper-cache");
    cached["cache_age_seconds"] = json!(age);
    if age > 6 * 3600 {
        cached["cache_note"] = json!(format!(
            "Cached SMART data is ~{} h old; the elevated helper may not be running. Run scripts/install-smart-helper.ps1 as Administrator, or run Claude elevated.",
            age / 3600
        ));
    }
}

pub fn get_drive_health(args: &Value) -> Result<Value, SmartError> {
    // A specific device is always read live.
    if args.get("device").and_then(|v| v.as_str()).is_some() {
        return get_drive_health_live(args);
    }
    // All-drives: try live; if it can't see drives (not elevated) or errors,
    // fall back to the elevated helper's cache when present.
    match get_drive_health_live(args) {
        Ok(live) if live_has_real_reports(&live) => Ok(live),
        Ok(live) => match read_cache() {
            Some((mut cached, age)) => {
                annotate_cache(&mut cached, age);
                Ok(cached)
            }
            None => Ok(live),
        },
        Err(e) => match read_cache() {
            Some((mut cached, age)) => {
                annotate_cache(&mut cached, age);
                Ok(cached)
            }
            None => Err(e),
        },
    }
}

fn get_drive_health_live(args: &Value) -> Result<Value, SmartError> {
    let device = args.get("device").and_then(|v| v.as_str());
    let dtype = args.get("type").and_then(|v| v.as_str());

    let targets: Vec<(String, Option<String>)> = if let Some(dev) = device {
        vec![(dev.to_string(), dtype.map(|s| s.to_string()))]
    } else {
        let scan = run_smartctl_json(&["--scan".to_string()])?;
        let devs: Vec<(String, Option<String>)> = scan
            .get("devices")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|d| {
                        let name = d.get("name").and_then(|v| v.as_str())?;
                        Some((name.to_string(), d.get("type").and_then(|v| v.as_str()).map(|s| s.to_string())))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if devs.is_empty() {
            return Ok(json!({
                "reports": [],
                "note": "smartctl --scan found no devices - usually means the server is not running as Administrator.",
                "smartctl_path": smartctl_path(),
            }));
        }
        devs
    };

    let mut reports = Vec::new();
    for (name, dtype) in &targets {
        let mut a = vec!["-a".to_string()];
        if let Some(t) = dtype {
            a.push("-d".to_string());
            a.push(t.clone());
        }
        a.push(name.clone());
        match run_smartctl_json(&a) {
            Ok(j) => reports.push(summarize(&j)),
            Err(e) => reports.push(json!({ "device": name, "verdict": "ERROR", "error": e.message })),
        }
    }
    let count = reports.len();
    Ok(json!({ "reports": reports, "count": count, "smartctl_path": smartctl_path() }))
}

pub fn run_self_test(args: &Value) -> Result<Value, SmartError> {
    let device = args
        .get("device")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SmartError::new("`device` is required for run_self_test.", "error"))?;
    let kind = if args.get("kind").and_then(|v| v.as_str()) == Some("long") { "long" } else { "short" };
    let mut a = vec!["-t".to_string(), kind.to_string()];
    if let Some(t) = args.get("type").and_then(|v| v.as_str()) {
        a.push("-d".to_string());
        a.push(t.to_string());
    }
    a.push(device.to_string());
    let (stdout, _) = run_smartctl(&a)?;
    Ok(json!({
        "device": device,
        "test": kind,
        "output": stdout.trim(),
        "note": "Self-test runs in the background on the drive. Re-run get_drive_health later to read results.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthy_nvme_is_healthy() {
        let s = summarize(&json!({
            "smartctl": { "messages": [] },
            "device": { "name": "/dev/sda", "protocol": "NVMe" },
            "model_name": "FireCuda 520",
            "smart_status": { "passed": true },
            "temperature": { "current": 41 },
            "nvme_smart_health_information_log": {
                "critical_warning": 0, "percentage_used": 7, "available_spare": 100,
                "available_spare_threshold": 10, "media_errors": 0, "data_units_written": 48000000
            }
        }));
        assert_eq!(s["verdict"], "HEALTHY");
        assert_eq!(s["host_written_tb"], 24.58);
    }

    #[test]
    fn pending_sectors_warns() {
        let s = summarize(&json!({
            "smartctl": { "messages": [] },
            "smart_status": { "passed": true },
            "ata_smart_attributes": { "table": [{ "id": 197, "name": "Current_Pending_Sector", "raw": { "value": 8 } }] }
        }));
        assert_eq!(s["verdict"], "WARNING");
        assert_eq!(s["pending_sectors"], 8);
    }

    #[test]
    fn failed_status_is_failing() {
        let s = summarize(&json!({ "smartctl": { "messages": [] }, "smart_status": { "passed": false } }));
        assert_eq!(s["verdict"], "FAILING");
    }
}
