// Hardware sensors (temps / fans / voltages / clocks / load / power) via
// LibreHardwareMonitor. Primary: its web server JSON (http://localhost:8085/data.json,
// fetched with a tiny zero-dependency HTTP/1.0 client). Fallback on Windows:
// the root/LibreHardwareMonitor WMI namespace via PowerShell. The parsing
// functions (parse_lhm_value / flatten_lhm / normalize_wmi_sensors) are pure
// and unit-tested. Degrades gracefully when LHM is not running.

use serde_json::{json, Value};

use crate::util::powershell_json;

/// Map an LHM group / WMI SensorType label to a normalized category.
fn category_of(text: &str) -> Option<&'static str> {
    match text.to_lowercase().as_str() {
        "temperature" | "temperatures" => Some("temperature"),
        "fan" | "fans" => Some("fan"),
        "voltage" | "voltages" => Some("voltage"),
        "clock" | "clocks" => Some("clock"),
        "load" => Some("load"),
        "power" | "powers" => Some("power"),
        "data" => Some("data"),
        "throughput" => Some("throughput"),
        "level" | "levels" => Some("level"),
        "control" | "controls" => Some("control"),
        _ => None,
    }
}

#[derive(Debug, PartialEq)]
pub struct LhmValue {
    pub value: Option<f64>,
    pub unit: Option<String>,
}

/// Pure: parse an LHM value string like "45.0 C" / "1200 RPM" / "1.20 V"
/// into a number and a trailing unit.
pub fn parse_lhm_value(s: &str) -> LhmValue {
    let s = s.trim();
    if s.is_empty() {
        return LhmValue { value: None, unit: None };
    }
    let bytes = s.as_bytes();
    let mut i = 0;
    if bytes[0] == b'-' {
        i = 1;
    }
    let digits_start = i;
    while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
        i += 1;
    }
    let parsed = if i > digits_start { s[..i].parse::<f64>().ok() } else { None };
    match parsed {
        Some(v) if v.is_finite() => {
            let unit = s[i..].trim();
            LhmValue {
                value: Some(v),
                unit: if unit.is_empty() { None } else { Some(unit.to_string()) },
            }
        }
        _ => LhmValue { value: None, unit: Some(s.to_string()) },
    }
}

fn str_field<'a>(node: &'a Value, key: &str) -> &'a str {
    node.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

fn walk(node: &Value, hardware: Option<&str>, category: Option<&'static str>, out: &mut Vec<Value>) {
    if !node.is_object() {
        return;
    }
    let text = str_field(node, "Text");
    let cat = category_of(text).or(category);
    let children = node.get("Children").and_then(|v| v.as_array());
    let is_leaf = children.map(|c| c.is_empty()).unwrap_or(true);

    if is_leaf {
        let raw = str_field(node, "Value");
        if !raw.is_empty() {
            let parsed = parse_lhm_value(raw);
            if let Some(v) = parsed.value {
                out.push(json!({
                    "hardware": hardware,
                    "category": cat,
                    "name": if text.is_empty() { Value::Null } else { json!(text) },
                    "value": v,
                    "unit": parsed.unit,
                    "min": parse_lhm_value(str_field(node, "Min")).value,
                    "max": parse_lhm_value(str_field(node, "Max")).value,
                }));
            }
        }
        return;
    }
    for c in children.unwrap() {
        walk(c, hardware, cat, out);
    }
}

/// Pure: flatten the LHM data.json tree (Computer -> Hardware -> groups -> sensors)
/// into a flat list of normalized sensor objects.
pub fn flatten_lhm(tree: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    let computers: Vec<Value> = if tree.get("Children").is_some() {
        tree.get("Children").and_then(|v| v.as_array()).cloned().unwrap_or_default()
    } else {
        vec![tree.clone()]
    };
    for computer in &computers {
        if let Some(hardware_list) = computer.get("Children").and_then(|v| v.as_array()) {
            for hw in hardware_list {
                let name = hw.get("Text").and_then(|v| v.as_str());
                walk(hw, name, None, &mut out);
            }
        }
    }
    out
}

/// Pure: normalize WMI Sensor rows (from PowerShell ConvertTo-Json).
pub fn normalize_wmi_sensors(rows: &Value) -> Vec<Value> {
    let arr: Vec<Value> = match rows {
        Value::Array(a) => a.clone(),
        Value::Null => Vec::new(),
        other => vec![other.clone()],
    };
    let unit_of = |st: &str| -> Option<&'static str> {
        match st {
            "Temperature" => Some("C"),
            "Fan" => Some("RPM"),
            "Voltage" => Some("V"),
            "Clock" => Some("MHz"),
            "Load" => Some("%"),
            "Power" => Some("W"),
            "Data" => Some("GB"),
            "Throughput" => Some("B/s"),
            "Level" => Some("%"),
            _ => None,
        }
    };
    arr.iter()
        .filter_map(|r| {
            let st = r.get("SensorType").and_then(|v| v.as_str()).unwrap_or("");
            let value = r
                .get("Value")
                .and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok())))
                .filter(|x| x.is_finite())
                .map(|x| (x * 100.0).round() / 100.0)?;
            let category = category_of(st)
                .map(|c| c.to_string())
                .or_else(|| if st.is_empty() { None } else { Some(st.to_lowercase()) });
            Some(json!({
                "hardware": r.get("Parent").and_then(|v| v.as_str()),
                "category": category,
                "name": r.get("Name").and_then(|v| v.as_str()),
                "value": value,
                "unit": unit_of(st),
            }))
        })
        .collect()
}

fn group_by_category(list: Vec<Value>) -> Value {
    let mut g = serde_json::Map::new();
    for s in list {
        let cat = s.get("category").and_then(|v| v.as_str()).unwrap_or("other").to_string();
        g.entry(cat).or_insert_with(|| json!([])).as_array_mut().unwrap().push(s);
    }
    Value::Object(g)
}

fn lhm_url() -> String {
    std::env::var("LHM_URL").unwrap_or_else(|_| "http://localhost:8085/data.json".to_string())
}

/// Minimal HTTP/1.0 GET to the (localhost) LHM web server. Avoids an HTTP crate.
fn fetch_http() -> Result<Value, String> {
    use std::io::{Read, Write};
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;

    let url = lhm_url();
    let rest = url.strip_prefix("http://").ok_or("LHM_URL must start with http://")?;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let host = hostport.split(':').next().unwrap_or("localhost").to_string();
    let authority = if hostport.contains(':') { hostport.to_string() } else { format!("{}:80", hostport) };

    let addr = authority
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or("could not resolve LHM host")?;

    let timeout = Duration::from_millis(2500);
    let mut stream = TcpStream::connect_timeout(&addr, timeout).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(timeout)).ok();
    stream.set_write_timeout(Some(timeout)).ok();

    let req = format!(
        "GET {} HTTP/1.0\r\nHost: {}\r\nConnection: close\r\nAccept: application/json\r\n\r\n",
        path, host
    );
    stream.write_all(req.as_bytes()).map_err(|e| e.to_string())?;

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let pos = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("malformed HTTP response")?;
    serde_json::from_slice(&buf[pos + 4..]).map_err(|e| e.to_string())
}

fn fetch_wmi() -> Result<Value, String> {
    let script = "Get-CimInstance -Namespace root/LibreHardwareMonitor -ClassName Sensor -ErrorAction Stop | \
Select-Object Name,SensorType,Value,Parent | ConvertTo-Json -Compress";
    powershell_json(script).ok_or_else(|| "WMI query failed".to_string())
}

pub fn get_sensors() -> Value {
    if let Ok(tree) = fetch_http() {
        let list = flatten_lhm(&tree);
        if !list.is_empty() {
            let count = list.len();
            return json!({
                "available": true,
                "source": "lhm-web",
                "url": lhm_url(),
                "count": count,
                "by_category": group_by_category(list),
            });
        }
    }
    if let Ok(rows) = fetch_wmi() {
        let list = normalize_wmi_sensors(&rows);
        if !list.is_empty() {
            let count = list.len();
            return json!({
                "available": true,
                "source": "lhm-wmi",
                "count": count,
                "by_category": group_by_category(list),
            });
        }
    }
    json!({
        "available": false,
        "reason": "No hardware sensor source found. Install LibreHardwareMonitor, run it as Administrator, and either enable its web server (Options -> Remote Web Server, port 8085) or leave it running for WMI access. Set LHM_URL to override the web server address.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_value_splits_number_and_unit() {
        // "\u{b0}" is the degree sign; written as an escape to keep source ASCII.
        assert_eq!(
            parse_lhm_value("45.0 \u{b0}C"),
            LhmValue { value: Some(45.0), unit: Some("\u{b0}C".to_string()) }
        );
        assert_eq!(
            parse_lhm_value("1200 RPM"),
            LhmValue { value: Some(1200.0), unit: Some("RPM".to_string()) }
        );
    }

    #[test]
    fn flatten_walks_the_tree() {
        let tree = json!({
            "Text": "Sensor",
            "Children": [{
                "Text": "THESEUS",
                "Children": [{
                    "Text": "AMD Ryzen 7 5800X",
                    "Children": [
                        { "Text": "Temperatures", "Children": [
                            { "Text": "Core (Tctl/Tdie)", "Value": "52.0 \u{b0}C", "Min": "30.0 \u{b0}C", "Max": "78.0 \u{b0}C", "Children": [] }
                        ]},
                        { "Text": "Clocks", "Children": [
                            { "Text": "Core #1", "Value": "4200.0 MHz", "Children": [] }
                        ]}
                    ]
                }]
            }]
        });
        let flat = flatten_lhm(&tree);
        let cpu_temp = flat.iter().find(|s| s["category"] == "temperature").expect("temp sensor");
        assert_eq!(cpu_temp["hardware"], "AMD Ryzen 7 5800X");
        assert_eq!(cpu_temp["value"], 52.0);
        assert_eq!(cpu_temp["max"], 78.0);
        assert!(flat.iter().any(|s| s["category"] == "clock" && s["value"] == 4200.0));
    }

    #[test]
    fn normalize_wmi_maps_units() {
        let rows = json!([{ "Name": "CPU Package", "SensorType": "Temperature", "Value": 55.5, "Parent": "/amdcpu/0" }]);
        let list = normalize_wmi_sensors(&rows);
        assert_eq!(list[0]["category"], "temperature");
        assert_eq!(list[0]["unit"], "C");
        assert_eq!(list[0]["value"], 55.5);
    }
}
