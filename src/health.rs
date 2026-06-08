// Unified health roll-up. Gathers from every collector and produces a
// prioritized "what's wrong / what to watch" report. evaluate_health() is pure
// and unit-tested; get_health_report() gathers (concurrently) then evaluates.

use serde_json::{json, Value};

use crate::util::settle;
use crate::{gpu, metrics, sensors, smart};

pub struct Thresholds {
    pub disk_free_pct_warn: f64,
    pub disk_free_gb_warn: f64,
    pub mem_used_pct_warn: f64,
    pub cpu_temp_warn: f64,
    pub cpu_temp_crit: f64,
    pub gpu_temp_warn: f64,
    pub gpu_temp_crit: f64,
    pub drive_temp_warn: f64,
    pub drive_temp_crit: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            disk_free_pct_warn: 10.0,
            disk_free_gb_warn: 20.0,
            mem_used_pct_warn: 90.0,
            cpu_temp_warn: 85.0,
            cpu_temp_crit: 95.0,
            gpu_temp_warn: 85.0,
            gpu_temp_crit: 95.0,
            drive_temp_warn: 60.0,
            drive_temp_crit: 70.0,
        }
    }
}

fn rank(severity: &str) -> i32 {
    match severity {
        "WARNING" => 1,
        "CRITICAL" => 2,
        _ => 0,
    }
}

/// Format a number without a trailing `.0` when it is whole.
fn fmt(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{}", n as i64)
    } else {
        format!("{}", n)
    }
}

fn fmt_opt(n: Option<f64>) -> String {
    n.map(fmt).unwrap_or_default()
}

fn add(issues: &mut Vec<Value>, overall: &mut String, severity: &str, area: &str, detail: String, rec: &str) {
    issues.push(json!({ "severity": severity, "area": area, "detail": detail, "recommendation": rec }));
    if rank(severity) > rank(overall) {
        *overall = severity.to_string();
    }
}

/// Pure: given collected data, produce { overall, summary, issues, sections }.
pub fn evaluate_health(data: &Value, t: &Thresholds) -> Value {
    let mut issues: Vec<Value> = Vec::new();
    let mut overall = String::from("OK");

    // Drives (SMART)
    if let Some(reports) = data.pointer("/drives/reports").and_then(|v| v.as_array()) {
        for d in reports {
            let label = d
                .get("model")
                .and_then(|v| v.as_str())
                .or_else(|| d.get("device").and_then(|v| v.as_str()))
                .unwrap_or("drive")
                .to_string();
            match d.get("verdict").and_then(|v| v.as_str()).unwrap_or("") {
                "FAILING" => add(&mut issues, &mut overall, "CRITICAL", "drive",
                    format!("{}: SMART overall-health FAILED", label),
                    "Back up immediately and replace the drive."),
                "WARNING" => {
                    let w = d.get("warnings").and_then(|v| v.as_array())
                        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join("; "))
                        .unwrap_or_default();
                    add(&mut issues, &mut overall, "WARNING", "drive",
                        format!("{}: {}", label, w),
                        "Back up and monitor; consider a long self-test.");
                }
                "NEEDS_ELEVATION" => add(&mut issues, &mut overall, "WARNING", "drive",
                    format!("{}: SMART unreadable (no admin)", label),
                    "Run as Administrator to read drive health."),
                _ => {}
            }
            if let Some(temp) = d.get("temperature_c").and_then(|v| v.as_f64()) {
                if temp >= t.drive_temp_crit {
                    add(&mut issues, &mut overall, "CRITICAL", "drive-temp",
                        format!("{} at {} C", label, fmt(temp)), "Improve drive cooling/airflow.");
                } else if temp >= t.drive_temp_warn {
                    add(&mut issues, &mut overall, "WARNING", "drive-temp",
                        format!("{} at {} C", label, fmt(temp)), "Check airflow around drives.");
                }
            }
        }
    }

    // Disk space
    if let Some(volumes) = data.pointer("/disks/volumes").and_then(|v| v.as_array()) {
        for v in volumes {
            let used_pct = v.get("used_percent").and_then(|x| x.as_f64());
            let free_gb = v.get("free_gb").and_then(|x| x.as_f64());
            let low_pct = used_pct.map(|p| p >= 100.0 - t.disk_free_pct_warn).unwrap_or(false);
            let low_gb = free_gb.map(|g| g <= t.disk_free_gb_warn).unwrap_or(false);
            if low_pct || low_gb {
                let mount = v.get("mount").and_then(|x| x.as_str()).unwrap_or("");
                add(&mut issues, &mut overall, "WARNING", "disk-space",
                    format!("{} {}% used ({} GB free)", mount, fmt_opt(used_pct), fmt_opt(free_gb)),
                    "Free up space or expand storage.");
            }
        }
    }

    // Memory
    if let Some(p) = data.pointer("/memory/used_percent").and_then(|v| v.as_f64()) {
        if p >= t.mem_used_pct_warn {
            add(&mut issues, &mut overall, "WARNING", "memory",
                format!("RAM {}% used", fmt(p)), "Close memory-heavy apps; check for leaks.");
        }
    }

    // CPU temp (prefer sensors, fall back to cpu.temperature_c)
    if let Some(ct) = pick_cpu_temp(data) {
        if ct >= t.cpu_temp_crit {
            add(&mut issues, &mut overall, "CRITICAL", "cpu-temp",
                format!("CPU at {} C", fmt(ct)), "Check cooler mount, thermal paste, and case airflow.");
        } else if ct >= t.cpu_temp_warn {
            add(&mut issues, &mut overall, "WARNING", "cpu-temp",
                format!("CPU at {} C", fmt(ct)), "Monitor under load; check cooling.");
        }
    }

    // GPU temp
    if data.pointer("/gpu/available").and_then(|v| v.as_bool()) == Some(true) {
        if let Some(gpus) = data.pointer("/gpu/gpus").and_then(|v| v.as_array()) {
            for g in gpus {
                let Some(temp) = g.get("temperature_c").and_then(|v| v.as_f64()) else { continue };
                let name = g.get("name").and_then(|v| v.as_str()).unwrap_or("GPU");
                if temp >= t.gpu_temp_crit {
                    add(&mut issues, &mut overall, "CRITICAL", "gpu-temp",
                        format!("{} at {} C", name, fmt(temp)), "Check GPU fans and case airflow.");
                } else if temp >= t.gpu_temp_warn {
                    add(&mut issues, &mut overall, "WARNING", "gpu-temp",
                        format!("{} at {} C", name, fmt(temp)), "Monitor GPU temps under load.");
                }
            }
        }
    }

    let sections = json!({
        "drives": section_drives(data),
        "disks": present_status(data.get("disks")),
        "memory": present_status(data.get("memory")),
        "cpu": present_status(data.get("cpu")),
        "gpu": section_available(data.get("gpu")),
        "sensors": section_available(data.get("sensors")),
    });

    issues.sort_by(|a, b| {
        let ra = rank(a.get("severity").and_then(|v| v.as_str()).unwrap_or(""));
        let rb = rank(b.get("severity").and_then(|v| v.as_str()).unwrap_or(""));
        rb.cmp(&ra)
    });

    let summary = if overall == "OK" {
        "All checked systems look healthy.".to_string()
    } else {
        format!("{} issue(s) found - highest severity: {}.", issues.len(), overall)
    };

    json!({ "overall": overall, "summary": summary, "issues": issues, "sections": sections })
}

fn present_status(v: Option<&Value>) -> Value {
    match v {
        Some(x) if !x.is_null() => json!("ok"),
        _ => json!("not_collected"),
    }
}

fn section_drives(data: &Value) -> Value {
    match data.get("drives") {
        Some(d) if !d.is_null() => {
            if d.get("reports").is_some() {
                json!("ok")
            } else {
                d.get("note").cloned().unwrap_or_else(|| json!("unavailable"))
            }
        }
        _ => json!("not_collected"),
    }
}

fn section_available(v: Option<&Value>) -> Value {
    match v {
        Some(x) if !x.is_null() => {
            if x.get("available").and_then(|a| a.as_bool()) == Some(true) {
                json!("ok")
            } else {
                x.get("reason").cloned().unwrap_or_else(|| json!("unavailable"))
            }
        }
        _ => json!("not_collected"),
    }
}

fn pick_cpu_temp(data: &Value) -> Option<f64> {
    if data.pointer("/sensors/available").and_then(|v| v.as_bool()) == Some(true) {
        if let Some(temps) = data.pointer("/sensors/by_category/temperature").and_then(|v| v.as_array()) {
            let pkg = temps.iter().find(|s| {
                let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
                name.contains("package") || name.contains("cpu")
            });
            if let Some(c) = pkg.or_else(|| temps.first()) {
                if let Some(v) = c.get("value").and_then(|v| v.as_f64()) {
                    return Some(v);
                }
            }
        }
    }
    data.pointer("/cpu/temperature_c").and_then(|v| v.as_f64())
}

pub fn get_health_report() -> Value {
    let (cpu, memory, disks, drives, gpu_data, sensor_data) = std::thread::scope(|sc| {
        let a = sc.spawn(|| settle(|| metrics::get_cpu(false)));
        let b = sc.spawn(|| settle(metrics::get_memory));
        let c = sc.spawn(|| settle(metrics::get_disks));
        let d = sc.spawn(|| settle(|| smart::get_drive_health(&json!({})).unwrap_or_else(|e| json!({ "error": e.message }))));
        let e = sc.spawn(|| settle(gpu::get_gpu));
        let f = sc.spawn(|| settle(sensors::get_sensors));
        (
            a.join().unwrap(),
            b.join().unwrap(),
            c.join().unwrap(),
            d.join().unwrap(),
            e.join().unwrap(),
            f.join().unwrap(),
        )
    });

    let data = json!({
        "cpu": cpu, "memory": memory, "disks": disks,
        "drives": drives, "gpu": gpu_data, "sensors": sensor_data,
    });
    let mut report = evaluate_health(&data, &Thresholds::default());
    report["collected"] = data;
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_inputs_are_ok() {
        let r = evaluate_health(
            &json!({
                "drives": { "reports": [{ "model": "SSD", "verdict": "HEALTHY", "temperature_c": 40 }] },
                "disks": { "volumes": [{ "mount": "C:", "used_percent": 50, "free_gb": 200 }] },
                "memory": { "used_percent": 45 },
                "cpu": { "temperature_c": 50 },
                "gpu": { "available": true, "gpus": [{ "name": "GTX 1080 Ti", "temperature_c": 60 }] },
                "sensors": { "available": false, "reason": "no LHM" }
            }),
            &Thresholds::default(),
        );
        assert_eq!(r["overall"], "OK");
        assert_eq!(r["issues"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn failing_drive_low_disk_hot_gpu_is_critical_sorted() {
        let r = evaluate_health(
            &json!({
                "drives": { "reports": [{ "model": "Old HDD", "verdict": "FAILING" }] },
                "disks": { "volumes": [{ "mount": "C:", "used_percent": 96, "free_gb": 8 }] },
                "memory": { "used_percent": 95 },
                "gpu": { "available": true, "gpus": [{ "name": "GTX 1080 Ti", "temperature_c": 97 }] }
            }),
            &Thresholds::default(),
        );
        assert_eq!(r["overall"], "CRITICAL");
        assert_eq!(r["issues"][0]["severity"], "CRITICAL");
        let issues = r["issues"].as_array().unwrap();
        assert!(issues.iter().any(|i| i["area"] == "disk-space"));
        assert!(issues.iter().any(|i| i["area"] == "memory"));
    }

    #[test]
    fn sensors_temp_preferred_over_cpu_fallback() {
        let r = evaluate_health(
            &json!({
                "sensors": { "available": true, "by_category": { "temperature": [{ "name": "CPU Package", "value": 96 }] } },
                "cpu": { "temperature_c": 40 }
            }),
            &Thresholds::default(),
        );
        let issues = r["issues"].as_array().unwrap();
        assert!(issues.iter().any(|i| i["area"] == "cpu-temp" && i["severity"] == "CRITICAL"));
    }
}
