// NVIDIA GPU metrics via nvidia-smi (CSV, no header, no units). Zero extra deps.
// parse_nvidia_smi() is pure and unit-tested. Degrades gracefully when no
// NVIDIA GPU / nvidia-smi is present.

use std::process::Command;

use serde_json::{json, Value};

use crate::util::numv;

const QUERY: &[&str] = &[
    "index",
    "name",
    "utilization.gpu",
    "memory.used",
    "memory.total",
    "temperature.gpu",
    "power.draw",
    "power.limit",
    "fan.speed",
    "clocks.sm",
];

/// Parse a numeric field; null when not a finite number.
fn num(field: &str) -> Value {
    match field.trim().parse::<f64>() {
        Ok(n) if n.is_finite() => numv(n),
        _ => Value::Null,
    }
}

/// Pure: parse nvidia-smi CSV (noheader, nounits) into structured GPU objects.
pub fn parse_nvidia_smi(csv: &str) -> Vec<Value> {
    csv.trim()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|line| {
            let f: Vec<&str> = line.split(',').map(str::trim).collect();
            let g = |i: usize| f.get(i).copied().unwrap_or("");
            json!({
                "index": num(g(0)),
                "name": if g(1).is_empty() { Value::Null } else { json!(g(1)) },
                "utilization_percent": num(g(2)),
                "memory_used_mb": num(g(3)),
                "memory_total_mb": num(g(4)),
                "temperature_c": num(g(5)),
                "power_draw_w": num(g(6)),
                "power_limit_w": num(g(7)),
                "fan_percent": num(g(8)),
                "clock_sm_mhz": num(g(9)),
            })
        })
        .collect()
}

enum GpuError {
    NotFound,
    Other(String),
}

fn run_nvidia_smi() -> Result<String, GpuError> {
    let exe = std::env::var("NVIDIA_SMI_PATH").unwrap_or_else(|_| "nvidia-smi".to_string());
    let query = format!("--query-gpu={}", QUERY.join(","));
    match Command::new(&exe).args([&query, "--format=csv,noheader,nounits"]).output() {
        Ok(out) => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(GpuError::NotFound),
        Err(e) => Err(GpuError::Other(e.to_string())),
    }
}

pub fn get_gpu() -> Value {
    match run_nvidia_smi() {
        Ok(csv) => {
            let gpus = parse_nvidia_smi(&csv);
            json!({ "available": true, "vendor": "NVIDIA", "count": gpus.len(), "gpus": gpus })
        }
        Err(GpuError::NotFound) => json!({
            "available": false,
            "reason": "No NVIDIA GPU detected (nvidia-smi not found). AMD/Intel GPU metrics are not yet supported.",
        }),
        Err(GpuError::Other(msg)) => json!({
            "available": false,
            "reason": format!("nvidia-smi error: {}", msg),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nvidia_smi_csv() {
        let csv = "0, NVIDIA GeForce GTX 1080 Ti, 23, 1024, 11264, 62, 95.5, 250, 41, 1771";
        let gpus = parse_nvidia_smi(csv);
        let g = &gpus[0];
        assert_eq!(g["name"], "NVIDIA GeForce GTX 1080 Ti");
        assert_eq!(g["utilization_percent"], 23);
        assert_eq!(g["memory_total_mb"], 11264);
        assert_eq!(g["temperature_c"], 62);
        assert_eq!(g["power_draw_w"], 95.5);
    }
}
