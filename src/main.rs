// Thin binary. Default: run the stdio JSON-RPC loop. With
// `--refresh-smart-cache` it does a single elevated SMART scan into the shared
// cache (used by the scheduled-task helper) and exits. All logic lives in the
// library crate so it can be unit- and integration-tested.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--refresh-smart-cache") {
        match system_monitor::smart::refresh_cache() {
            Ok(path) => eprintln!("system-monitor: wrote SMART cache to {path}"),
            Err(e) => {
                eprintln!("system-monitor: SMART cache refresh failed: {}", e.message);
                std::process::exit(1);
            }
        }
        return;
    }

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("system-monitor {}", system_monitor::SERVER_VERSION);
        return;
    }

    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "system-monitor {} - local system & hardware monitoring MCP server\n\n\
             Usage:\n  system-monitor                 Run the MCP server on stdio (default)\n  \
             system-monitor --refresh-smart-cache   Elevated SMART scan into the shared cache, then exit\n  \
             system-monitor --version\n  system-monitor --help",
            system_monitor::SERVER_VERSION
        );
        return;
    }

    system_monitor::run_stdio_loop();
}
