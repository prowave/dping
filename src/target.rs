//! Builds the ordered ping target list: gateway -> hops -> anchors -> extras.
//! `analyzer::diagnose` depends on this order (walking outward from the
//! local machine toward the internet).

use std::collections::HashSet;

use crate::cli::Args;
use crate::discovery;

pub struct Target {
    pub name: String,
    pub address: String,
}

const DEFAULT_ANCHORS: [(&str, &str); 2] = [
    ("cloudflare(1.1.1.1)", "1.1.1.1"),
    ("google(8.8.8.8)", "8.8.8.8"),
];

pub async fn build_targets(args: &Args) -> Vec<Target> {
    let mut targets = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // Dedup is on the literal string passed to ping, exactly like the
    // Python original — no DNS resolution happens first, so a hostname
    // extra that happens to resolve to an already-added IP is NOT deduped.
    let mut add = |name: String, address: &str| {
        if !address.is_empty() && seen.insert(address.to_string()) {
            targets.push(Target {
                name,
                address: address.to_string(),
            });
        }
    };

    match discovery::default_gateway().await {
        Some(gw) => add("gateway".to_string(), &gw.to_string()),
        None => eprintln!("warning: could not auto-detect default gateway"),
    }

    if !args.no_traceroute {
        let probe_target = args
            .targets
            .as_deref()
            .and_then(|t| t.split(',').next())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_ANCHORS[0].1.to_string());
        eprintln!("discovering path to {probe_target} ...");
        for (hop_num, ip) in discovery::traceroute_hops(&probe_target, 15, 1).await {
            if let Some(ip) = ip {
                add(format!("hop{hop_num}({ip})"), &ip.to_string());
            }
        }
    }

    for (name, ip) in DEFAULT_ANCHORS {
        add(name.to_string(), ip);
    }

    if let Some(extra_targets) = &args.targets {
        for extra in extra_targets.split(',') {
            let extra = extra.trim();
            if !extra.is_empty() {
                add(extra.to_string(), extra);
            }
        }
    }

    targets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_is_literal_string_not_dns_resolved() {
        let mut seen: HashSet<String> = HashSet::new();
        let mut targets: Vec<Target> = Vec::new();
        let mut add = |name: String, address: &str| {
            if !address.is_empty() && seen.insert(address.to_string()) {
                targets.push(Target {
                    name,
                    address: address.to_string(),
                });
            }
        };

        add("cloudflare(1.1.1.1)".to_string(), "1.1.1.1");
        // A literal duplicate of an already-added address is skipped.
        add("dup".to_string(), "1.1.1.1");
        // A hostname that *resolves* to 1.1.1.1 is a different literal
        // string and must NOT be deduped against the IP anchor above.
        add("one.one.one.one".to_string(), "one.one.one.one");

        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].address, "1.1.1.1");
        assert_eq!(targets[1].address, "one.one.one.one");
    }
}
