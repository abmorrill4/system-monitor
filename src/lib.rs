//! system-monitor: a local system & hardware monitoring MCP server.
//!
//! Modular collectors (metrics, smart, gpu, sensors) feed a unified health
//! roll-up. Parsers are pure and unit-tested; every source degrades to an
//! `available:false` / `error` value rather than panicking, so one failing
//! source never breaks an aggregate.

pub mod gpu;
pub mod health;
pub mod metrics;
pub mod sensors;
pub mod server;
pub mod smart;
pub mod util;

pub use server::{
    dispatch_tool, get_system_snapshot, run_stdio_loop, tools, PROTOCOL_VERSION, SERVER_NAME,
    SERVER_VERSION,
};
