# dping

On-demand tool for finding *where* packet loss or latency spikes are
happening between your machine and the internet — your gateway, an ISP hop,
or somewhere out past your ISP.

A single native binary written in Rust (`dping/`) — no Python, no
runtime dependencies to install. It shells out to the system `ping` /
`traceroute` (`tracert` on Windows) / gateway-lookup APIs, so it needs no
root/admin privileges on Linux, macOS, or Windows.

## How it works

1. Auto-detects your default gateway.
2. Runs a traceroute to figure out the hop chain out to the internet.
3. Pings the gateway, every discovered hop, and two stable public anchors
   (1.1.1.1, 8.8.8.8) concurrently and continuously.
4. Shows a live dashboard of loss%, RTT, and jitter per target.
5. On exit, prints a summary and a plain-English diagnosis of which segment
   the loss starts at (your gateway / an ISP hop / the destination).

## Building

```
cd dping
cargo build --release        # binary at target/release/dping
# or:
cargo install --path .       # installs `dping` onto your PATH
```

Requires a recent Rust toolchain (edition 2024, e.g. rustc 1.85+).

### Cross-compiling for all six targets

Native builds only produce a binary for the host you're building on. To
build for every combination of {macOS, Linux, Windows} x {x86_64, arm64}
from a single machine:

```
rustup target add \
  aarch64-apple-darwin x86_64-apple-darwin \
  aarch64-unknown-linux-musl x86_64-unknown-linux-musl \
  aarch64-pc-windows-msvc x86_64-pc-windows-msvc

cargo install cargo-zigbuild cargo-xwin   # needs `zig` on PATH (e.g. `brew install zig`)

cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin
cargo zigbuild --release --target aarch64-unknown-linux-musl
cargo zigbuild --release --target x86_64-unknown-linux-musl
cargo xwin build --release --target aarch64-pc-windows-msvc
cargo xwin build --release --target x86_64-pc-windows-msvc
```

Linux targets use musl (statically linked, no glibc version dependency);
Windows targets use MSVC via `cargo-xwin`, which downloads the Windows
SDK/CRT headers on first use — no real Windows machine needed. Binaries
land under `target/<triple>/release/dping[.exe]`.

If you have `rustup` *and* a Homebrew/system-packaged Rust both installed,
make sure `~/.cargo/bin` (the rustup shims) comes before the system Rust on
`PATH` — otherwise `--target` builds fail with `can't find crate for
core/std` because the wrong `rustc` (missing the cross-compiled std libs)
gets invoked.

## Usage

```
dping                              # live dashboard, run until q / Esc / Ctrl+C
dping --duration 300 --log run.csv # timed 5-minute run, log every sample to CSV
dping --plain                      # plain snapshot output instead of the dashboard
dping --targets example.com,8.8.4.4 # add extra targets
```

Press `q`, `Esc`, or `Ctrl+C` to stop and see the summary + diagnosis.

### Options

| Flag | Default | Meaning |
|---|---|---|
| `--targets` | - | comma-separated extra hosts/IPs to include |
| `--interval` | 1 | seconds between pings per target |
| `--window` | 60 | rolling sample count used for the "recent loss" column |
| `--timeout` | 1 | per-ping timeout in seconds |
| `--no-traceroute` | off | skip hop discovery; just ping gateway + anchors |
| `--duration` | run until quit | auto-stop after N seconds |
| `--log` | - | write every ping sample (timestamp, target, rtt) to a CSV |
| `--plain` | off | print periodic snapshots instead of the live dashboard (automatic when stdout isn't a terminal) |

## Reading the diagnosis

The tool walks the path in order (gateway → hop 2 → hop 3 → … → public
anchors) and reports the *first* point where session packet loss crosses
~2%. Loss starting at the gateway points at your local network; loss
starting a few hops out points at your ISP; loss that only shows up on the
public anchors (with clean hops before them) points at the destination or
peering path rather than your connection. High jitter with no real loss is
called out separately — it can still cause choppy calls/video without hard
drops.

## Platform notes

- Verified end-to-end (build, tests, live dashboard) on macOS.
- All six targets — macOS/Linux/Windows x arm64/x86_64 — build cleanly via
  cross-compilation (see above) and pass the full parser/analyzer unit test
  suite, but live behavior on Linux and Windows hasn't yet been exercised
  on real hardware/CI — see `dping/src/platform/{linux,windows}.rs` for the
  per-OS command/parsing assumptions.
- Windows `ping.exe` output can be localized on non-English installs; RTT
  parsing there falls back to a locale-tolerant match and fails closed to
  "lost" (never crashes) if it can't recognize the line.
