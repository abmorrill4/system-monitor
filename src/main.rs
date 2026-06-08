// Thin binary: start the stdio JSON-RPC loop. All logic lives in the library
// crate so it can be unit- and integration-tested.

fn main() {
    system_monitor::run_stdio_loop();
}
