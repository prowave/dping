//! Only reachable via `platform::active` on a macOS host — on other hosts
//! these functions are exercised solely by `cargo test` (see
//! `platform::mod` for why every OS module is unconditionally compiled),
//! so a non-macOS `cargo build` reports them as dead code; that's expected.
#![allow(dead_code)]

use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;
use tokio::process::Command;

use super::{Hop, ipv4_regex};

fn ping_time_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"time=([\d.]+)\s*ms").unwrap())
}

fn hop_line_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*(\d+)\s+(.*)$").unwrap())
}

/// `-c 1`: one echo request. `-W <ms>`: BSD/macOS ping's per-packet timeout,
/// in milliseconds — clamped to a 100ms floor, matching the Python original.
pub fn ping_command(target: &str, timeout: Duration) -> Command {
    let timeout_ms = (timeout.as_millis() as u64).max(100);
    let mut cmd = Command::new("ping");
    cmd.args(["-c", "1", "-W", &timeout_ms.to_string(), target]);
    cmd
}

pub fn parse_ping_rtt(stdout: &str) -> Option<f64> {
    ping_time_regex()
        .captures(stdout)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<f64>().ok())
}

/// `-n` numeric (no rDNS), `-m` max hops, `-q 1` one probe/hop, `-w` per-probe
/// wait in seconds. macOS traceroute prints its banner to stderr, not
/// stdout, so stdout here is just hop lines.
pub fn traceroute_command(target: &str, max_hops: u32, wait_secs: u32) -> Command {
    let mut cmd = Command::new("traceroute");
    cmd.args([
        "-n",
        "-m",
        &max_hops.to_string(),
        "-q",
        "1",
        "-w",
        &wait_secs.to_string(),
        target,
    ]);
    cmd
}

pub fn parse_traceroute(stdout: &str) -> Vec<Hop> {
    let mut hops = Vec::new();
    for line in stdout.lines() {
        let Some(caps) = hop_line_regex().captures(line) else {
            continue;
        };
        let Ok(hop_num) = caps[1].parse::<u32>() else {
            continue;
        };
        let rest = &caps[2];
        let ip = ipv4_regex()
            .find(rest)
            .and_then(|m| m.as_str().parse::<IpAddr>().ok());
        hops.push((hop_num, ip));
    }
    hops
}

/// Fallback used only if the native `netdev` gateway lookup fails: parses
/// `route -n get default` output, looking for a `gateway:` line.
pub fn parse_gateway_fallback(stdout: &str) -> Option<IpAddr> {
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("gateway:") {
            let candidate = rest.trim();
            // Full-string parse, not just "contains an IP somewhere" —
            // matches the Python original's IP_RE.fullmatch validation.
            return candidate.parse::<IpAddr>().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_ping_rtt() {
        let out = "PING 1.1.1.1 (1.1.1.1): 56 data bytes\n64 bytes from 1.1.1.1: icmp_seq=0 ttl=59 time=12.345 ms\n";
        assert_eq!(parse_ping_rtt(out), Some(12.345));
    }

    #[test]
    fn no_time_line_is_lost() {
        assert_eq!(parse_ping_rtt("Request timeout for icmp_seq 0\n"), None);
        assert_eq!(parse_ping_rtt(""), None);
    }

    #[test]
    fn traceroute_with_timed_out_hop() {
        let out = "\
 1  192.168.1.1  1.234 ms
 2  *
 3  8.8.8.8  10.5 ms
";
        let hops = parse_traceroute(out);
        assert_eq!(
            hops,
            vec![
                (1, Some("192.168.1.1".parse().unwrap())),
                (2, None),
                (3, Some("8.8.8.8".parse().unwrap())),
            ]
        );
    }

    #[test]
    fn gateway_fallback_parses_gateway_line() {
        let out =
            "   route to: default\ndestination: default\ngateway: 192.168.1.1\ninterface: en0\n";
        assert_eq!(
            parse_gateway_fallback(out),
            Some("192.168.1.1".parse().unwrap())
        );
    }

    #[test]
    fn gateway_fallback_missing_line_is_none() {
        assert_eq!(parse_gateway_fallback("no gateway here\n"), None);
    }
}
