//! Default-gateway and hop-chain discovery. Both functions are deliberately
//! infallible from the caller's point of view — any failure (missing
//! binary, subprocess error, unparseable output) degrades to `None`/`vec![]`
//! rather than propagating an error, matching the Python original's
//! leniency.

use std::net::IpAddr;
use std::time::Duration;

use tokio::time::timeout;

use crate::platform::{Hop, active};

/// Native OS-API gateway lookup (no subprocess) via the `netdev` crate,
/// falling back to a per-OS text-parsing subprocess call only if that
/// fails.
pub async fn default_gateway() -> Option<IpAddr> {
    if let Ok(device) = netdev::get_default_gateway() {
        if let Some(ip) = device.ipv4.first() {
            return Some(IpAddr::V4(*ip));
        }
        if let Some(ip) = device.ipv6.first() {
            return Some(IpAddr::V6(*ip));
        }
    }
    gateway_fallback().await
}

async fn gateway_fallback() -> Option<IpAddr> {
    #[cfg(target_os = "windows")]
    let mut cmd = tokio::process::Command::new("route");
    #[cfg(target_os = "windows")]
    cmd.arg("print");

    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = tokio::process::Command::new("route");
        c.args(["-n", "get", "default"]);
        c
    };

    #[cfg(target_os = "linux")]
    let mut cmd = {
        let mut c = tokio::process::Command::new("ip");
        c.args(["route", "show", "default"]);
        c
    };

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let mut cmd = {
        let mut c = tokio::process::Command::new("ip");
        c.args(["route", "show", "default"]);
        c
    };

    let output = timeout(Duration::from_secs(5), cmd.output())
        .await
        .ok()?
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    active::parse_gateway_fallback(&stdout)
}

/// Runs a traceroute toward `target`, up to `max_hops` hops (1 probe/hop,
/// `wait` seconds per probe). Any subprocess error/missing binary yields an
/// empty hop list — the tool still runs, it just discovers no `hop{N}` rows,
/// identical to passing `--no-traceroute`.
pub async fn traceroute_hops(target: &str, max_hops: u32, wait_secs: u32) -> Vec<Hop> {
    let mut cmd = active::traceroute_command(target, max_hops, wait_secs);
    let overall_timeout = Duration::from_secs((max_hops * wait_secs) as u64 + 10);
    let Ok(Ok(output)) = timeout(overall_timeout, cmd.output()).await else {
        return Vec::new();
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    active::parse_traceroute(&stdout)
}
