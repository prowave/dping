//! Per-OS command construction and output parsing for `ping`, `traceroute`
//! and default-gateway discovery.
//!
//! All three OS modules are **unconditionally compiled on every host** —
//! only the `active` alias below is `cfg`-gated. This means every parser
//! (`parse_ping_rtt`, `parse_traceroute`, `parse_gateway_fallback`) is
//! unit-testable from a single `cargo test` run on any host, without needing
//! a 3-OS CI matrix just to exercise parsing logic. The `cfg` gate only
//! decides which module's *command builders* are actually invoked against a
//! real subprocess at runtime.

pub mod linux;
pub mod macos;
pub mod windows;

#[cfg(target_os = "linux")]
pub use linux as active;
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub use linux as active;
#[cfg(target_os = "macos")]
pub use macos as active;
#[cfg(target_os = "windows")]
pub use windows as active;

use std::net::IpAddr;

/// A single traceroute hop: the hop number and the IPv4 address that
/// responded, or `None` if that hop timed out.
pub type Hop = (u32, Option<IpAddr>);

/// Shared IPv4-dotted-quad regex used by every OS's parsers.
pub fn ipv4_regex() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}").unwrap())
}

// Each OS module (macos/linux/windows) exposes the same free-function
// shape — ping_command, parse_ping_rtt, traceroute_command,
// parse_traceroute, parse_gateway_fallback — so callers use
// `platform::active::<fn>(...)` uniformly via the module alias above.
