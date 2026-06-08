// Small shared helpers: numeric rounding/formatting and a PowerShell JSON shim.
// Keeping numbers integral when whole keeps the JSON output close to the old JS.

use serde_json::{json, Value};

/// Round `n` to `d` decimal places.
pub fn round(n: f64, d: u32) -> f64 {
    let f = 10f64.powi(d as i32);
    (n * f).round() / f
}

/// A JSON number that drops a trailing `.0` when the value is whole
/// (so 40.0 serializes as `40`, matching the old JS output), or null if non-finite.
pub fn numv(n: f64) -> Value {
    if !n.is_finite() {
        return Value::Null;
    }
    if n.fract() == 0.0 && n.abs() < 9.007e15 {
        json!(n as i64)
    } else {
        json!(n)
    }
}

/// Bytes -> GB (2 decimals), as a JSON number or null.
pub fn gb(bytes: f64) -> Value {
    if bytes.is_finite() {
        numv(round(bytes / 1e9, 2))
    } else {
        Value::Null
    }
}

/// part/whole as a percentage (1 decimal), or null when whole is zero.
pub fn pct(part: f64, whole: f64) -> Value {
    if whole > 0.0 {
        numv(round(part / whole * 100.0, 1))
    } else {
        Value::Null
    }
}

/// Run a collector closure, converting a panic into an `{ "error": ... }` value
/// instead of unwinding. Mirrors the JS `settle()` so one failing source never
/// breaks an aggregate (snapshot / health report).
pub fn settle<F: FnOnce() -> Value + std::panic::UnwindSafe>(f: F) -> Value {
    std::panic::catch_unwind(f).unwrap_or_else(|_| json!({ "error": "collector panicked" }))
}

/// Run a PowerShell one-liner and parse its stdout as JSON. Windows only;
/// returns None on any failure so callers can degrade gracefully.
#[cfg(windows)]
pub fn powershell_json(script: &str) -> Option<Value> {
    use std::process::Command;
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str(trimmed).ok()
}

#[cfg(not(windows))]
pub fn powershell_json(_script: &str) -> Option<Value> {
    None
}
