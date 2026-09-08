//! Only reachable via `platform::active` on a Windows host — on other
//! hosts these functions are exercised solely by `cargo test` (see
//! `platform::mod` for why every OS module is unconditionally compiled),
//! so a non-Windows `cargo build` reports them as dead code; that's
//! expected.
#![allow(dead_code)]

use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;
use tokio::process::Command;

use super::{Hop, ipv4_regex};

fn ping_time_primary_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Covers `time=12ms` (English) and `time<1ms` (sub-millisecond, a case
    // the macOS/Linux regex never needs to handle).
    RE.get_or_init(|| Regex::new(r"time[=<]([\d.]+)\s*ms").unwrap())
}

fn ping_time_fallback_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Non-English Windows locales localize the `time=`/`temps=`/etc. label,
    // but the bare "<number>ms" token is conventionally left untranslated —
    // drop the label requirement entirely and take the first numeric hit.
    RE.get_or_init(|| Regex::new(r"([\d.]+)\s*ms\b").unwrap())
}

fn hop_line_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*(\d+)\s+(.*)$").unwrap())
}

/// `-n 1`: one echo request. `-w <ms>`: timeout in milliseconds, like
/// macOS — clamped to a 100ms floor for consistency with the other OSes.
pub fn ping_command(target: &str, timeout: Duration) -> Command {
    let timeout_ms = (timeout.as_millis() as u64).max(100);
    let mut cmd = Command::new("ping");
    cmd.args(["-n", "1", "-w", &timeout_ms.to_string(), target]);
    cmd
}

/// Two-tier, locale-tolerant RTT parse. `time<1ms` maps to `0.5` ms as an
/// explicit, documented convention (not silently guessed). If neither tier
/// matches — e.g. "Request timed out." or a genuinely unrecognized locale
/// format — this fails closed to `None` (a lost ping) rather than panicking.
pub fn parse_ping_rtt(stdout: &str) -> Option<f64> {
    if let Some(caps) = ping_time_primary_regex().captures(stdout) {
        let raw = &caps[1];
        return if stdout[caps.get(0).unwrap().range()].contains('<') {
            Some(0.5)
        } else {
            raw.parse::<f64>().ok()
        };
    }
    ping_time_fallback_regex()
        .captures(stdout)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<f64>().ok())
}

/// `-d`: don't resolve hostnames (numeric, like `-n` elsewhere). `-h`: max
/// hops. `-w`: per-probe wait, in **milliseconds** — `wait_secs` (the same
/// unit used by every other OS's traceroute command builder) is converted
/// here so the call site can stay unit-agnostic. tracert always sends 3
/// probes per hop; there's no flag to request 1.
pub fn traceroute_command(target: &str, max_hops: u32, wait_secs: u32) -> Command {
    let wait_ms = wait_secs.saturating_mul(1000).max(1);
    let mut cmd = Command::new("tracert");
    cmd.args([
        "-d",
        "-h",
        &max_hops.to_string(),
        "-w",
        &wait_ms.to_string(),
        target,
    ]);
    cmd
}

/// tracert's per-hop lines carry three RTT columns before the responding
/// IP, e.g. `  1     1 ms     1 ms     1 ms  192.168.1.1`, or
/// `  2     *        *        *     Request timed out.` for a timed-out
/// hop. The three RTT columns are discarded — this tool's own repeated
/// pinging of each discovered hop (in `pinger.rs`) is the latency source,
/// not traceroute's one-shot probes — so only the hop number and the last
/// IPv4-looking token on the line matter.
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
            .find_iter(rest)
            .last()
            .and_then(|m| m.as_str().parse::<IpAddr>().ok());
        hops.push((hop_num, ip));
    }
    hops
}

/// Fallback used only if the native `netdev` gateway lookup fails: a
/// best-effort `route print` parser. `route print`'s table layout has
/// varied across Windows versions, so this is intentionally simple (match
/// the classic IPv4 route table row `0.0.0.0  0.0.0.0  <gateway>  ...`) and
/// should be verified against a real Windows host/CI runner before being
/// relied on — see the plan's Windows verification notes.
pub fn parse_gateway_fallback(stdout: &str) -> Option<IpAddr> {
    for line in stdout.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 3
            && cols[0] == "0.0.0.0"
            && cols[1] == "0.0.0.0"
            && let Ok(ip) = cols[2].parse::<IpAddr>()
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
    fn parses_standard_english_ping_rtt() {
        let out = "Reply from 1.1.1.1: bytes=32 time=12ms TTL=59\n";
        assert_eq!(parse_ping_rtt(out), Some(12.0));
    }

    #[test]
    fn parses_sub_millisecond_ping_rtt() {
        let out = "Reply from 192.168.1.1: bytes=32 time<1ms TTL=64\n";
        assert_eq!(parse_ping_rtt(out), Some(0.5));
    }

    #[test]
    fn parses_localized_french_ping_rtt_via_fallback_tier() {
        let out = "Réponse de 1.1.1.1 : octets=32 temps=12ms TTL=59\n";
        assert_eq!(parse_ping_rtt(out), Some(12.0));
    }

    #[test]
    fn unmatchable_line_fails_closed_to_none() {
        assert_eq!(parse_ping_rtt("Request timed out.\n"), None);
        assert_eq!(parse_ping_rtt(""), None);
    }

    #[test]
    fn tracert_three_probe_format_with_timeout_hop() {
        let out = "\
  1     1 ms     1 ms     1 ms  192.168.1.1
  2     *        *        *     Request timed out.
  3    11 ms    10 ms    12 ms  8.8.8.8
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
    fn gateway_fallback_parses_route_print_default_row() {
        let out = "Network Destination        Netmask          Gateway       Interface  Metric\n0.0.0.0          0.0.0.0      192.168.1.1     192.168.1.50     25\n";
        assert_eq!(
            parse_gateway_fallback(out),
            Some("192.168.1.1".parse().unwrap())
        );
    }

    #[test]
    fn gateway_fallback_no_default_row_is_none() {
        assert_eq!(parse_gateway_fallback("some unrelated text\n"), None);
    }
}
