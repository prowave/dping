//! Only reachable via `platform::active` on a Linux host — on other hosts
//! these functions are exercised solely by `cargo test` (see
//! `platform::mod` for why every OS module is unconditionally compiled),
//! so a non-Linux `cargo build` reports them as dead code; that's expected.
#![allow(dead_code)]

use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;
use tokio::process::Command;

use super::{Hop, ipv4_regex};

fn ping_time_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // iputils' `time=13.5 ms` matches the exact same shape as macOS/BSD ping.
    RE.get_or_init(|| Regex::new(r"time=([\d.]+)\s*ms").unwrap())
}

fn hop_line_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*(\d+)\s+(.*)$").unwrap())
}

fn via_gateway_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"via\s+(\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3})").unwrap())
}

/// `-c 1`: one echo request. iputils' `-W <timeout>` is **integer seconds**
/// (unlike BSD/macOS's milliseconds) and has no sub-second granularity, so
/// this is clamped to a 1-second floor — the real sub-second timeout
/// enforcement comes from the outer async timeout wrapper in `pinger.rs`,
/// not from this flag alone.
pub fn ping_command(target: &str, timeout: Duration) -> Command {
    let timeout_secs = (timeout.as_secs_f64().ceil() as u64).max(1);
    let mut cmd = Command::new("ping");
    cmd.args(["-c", "1", "-W", &timeout_secs.to_string(), target]);
    cmd
}

pub fn parse_ping_rtt(stdout: &str) -> Option<f64> {
    ping_time_regex()
        .captures(stdout)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<f64>().ok())
}

/// Same flag set as macOS/BSD traceroute — both the classic and inetutils
/// traceroute packages accept `-n -m -q -w`. If the binary is missing
/// entirely (common on minimal images), the caller's subprocess-spawn
/// failure handling degrades to an empty hop list, exactly like any other
/// platform.
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
/// `ip route show default` output, e.g.
/// `default via 192.168.1.1 dev eth0 proto dhcp metric 100`.
pub fn parse_gateway_fallback(stdout: &str) -> Option<IpAddr> {
    for line in stdout.lines() {
        if let Some(caps) = via_gateway_regex().captures(line)
            && let Ok(ip) = caps[1].parse::<IpAddr>()
        {
            return Some(ip);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_iputils_ping_rtt() {
        let out = "PING 1.1.1.1 (1.1.1.1) 56(84) bytes of data.\n64 bytes from 1.1.1.1: icmp_seq=1 ttl=59 time=13.5 ms\n";
        assert_eq!(parse_ping_rtt(out), Some(13.5));
    }

    #[test]
    fn no_time_line_is_lost() {
        assert_eq!(parse_ping_rtt(""), None);
    }

    #[test]
    fn traceroute_with_timed_out_hop() {
        let out = "\
 1  192.168.1.1  1.234 ms
 2  * * *
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
    fn missing_traceroute_binary_yields_empty_hops_via_empty_stdout() {
        // A subprocess spawn failure is handled entirely in discovery.rs by
        // returning vec![] before ever calling this parser; this test just
        // documents that empty/garbage stdout also parses to no hops.
        assert_eq!(parse_traceroute(""), Vec::<Hop>::new());
    }

    #[test]
    fn gateway_fallback_parses_via_line() {
        let out = "default via 192.168.1.1 dev eth0 proto dhcp metric 100\n";
        assert_eq!(
            parse_gateway_fallback(out),
            Some("192.168.1.1".parse().unwrap())
        );
    }

    #[test]
    fn gateway_fallback_missing_via_is_none() {
        assert_eq!(
            parse_gateway_fallback("10.0.0.0/8 dev eth0 scope link\n"),
            None
        );
    }
}
