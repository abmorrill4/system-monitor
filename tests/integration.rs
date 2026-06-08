// Protocol + live-data integration test: spawn the built server binary, speak
// newline-delimited JSON-RPC over stdio, and assert on the responses.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::Value;

fn rpc(lines: &[&str]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_system-monitor"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn server");
    {
        let mut stdin = child.stdin.take().unwrap();
        for l in lines {
            stdin.write_all(l.as_bytes()).unwrap();
            stdin.write_all(b"\n").unwrap();
        }
    } // dropping stdin closes it -> server sees EOF and exits
    let output = child.wait_with_output().expect("wait for server");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("valid JSON line"))
        .collect()
}

#[test]
fn initialize_and_tools_list_exposes_all_tools() {
    let msgs = rpc(&[
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    ]);
    let list = msgs.iter().find(|m| m["id"] == 2).expect("tools/list response");
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for expected in [
        "get_system_info", "get_system_snapshot", "get_health_report", "get_cpu", "get_memory",
        "get_disks", "get_network", "get_top_processes", "get_gpu", "get_sensors",
        "get_drive_health", "run_self_test",
    ] {
        assert!(names.contains(&expected), "missing tool: {}", expected);
    }
}

#[test]
fn get_memory_returns_real_data() {
    let msgs = rpc(&[
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_memory","arguments":{}}}"#,
    ]);
    let res = msgs.iter().find(|m| m["id"] == 2).expect("tools/call response");
    assert_eq!(res["result"]["isError"], false);
    let text = res["result"]["content"][0]["text"].as_str().unwrap();
    let mem: Value = serde_json::from_str(text).unwrap();
    assert!(mem["total_gb"].as_f64().unwrap() > 0.0, "expected positive total memory");
}
