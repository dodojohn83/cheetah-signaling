# cheetah-gb28181-simulator

A deterministic, fixed-shard GB28181 signalling simulator for load- and
resilience-testing Cheetah Signaling's GB28181 protocol stack.  A run is fully
described by a scenario file plus its seed and produces a reproducible JSON
report (message counts, fault counts, semantic outcomes, resource usage and a
transcript hash).

## Control-plane boundary

This tool is **control-plane only**.  It exercises SIP signalling handshakes
(REGISTER/digest, keepalive, Catalog, INVITE/200/BYE) and produces media
*control* events with synthetic SDP.  It **never** generates, parses, stores or
transmits RTP, RTCP, PS, TS or ES media payloads, and it binds no media ports.

## Architecture

- **Fixed shards, lazy state.** A fixed number of shard workers own many lazy
  device states (`device index % shards`).  There is no per-device Tokio task
  and no per-device timer.
- **Single deterministic clock.** All events (device start, keepalive, scripted
  steps, deliveries) are ordered by one `TimerWheel` keyed by `(due_ms, seq)`.
  Device start and keepalive are staggered by a seeded RNG.
- **Sans-I/O peers.** `Device` and `Platform` are pure state machines that
  consume/produce `SipMessage`s; the harness drives them and the transport.
- **Real parser contract.** Encoding/parsing reuse `cheetah-gb28181-core`
  (`SipParser`/`encode_message`) and `cheetah-gb28181-module` XML builders, so
  the existing golden-fixture and parser contracts are preserved.  UDP parses
  whole datagrams; TCP feeds a streaming parser to reassemble half-packet and
  coalesced framing.
- **Seeded everything.** Time, IDs, jitter and every fault decision derive from
  the master seed via SHA-256 stream/indexed RNGs, so two runs of the same
  scenario are byte-identical (same transcript hash).

## Scenario DSL

Scenarios are TOML (see [`scenarios/`](scenarios/)).  Top-level keys configure
the run; `[profile]` selects the vendor/standard behaviour; `[[steps]]` scripts
platform-initiated actions; `[[faults]]` injects deterministic faults.

```toml
name = "baseline"
seed = 42
shards = 4
device_count = 50
transport = "udp"        # or "tcp"
duration_ms = 180000
register_stagger_ms = 30000
udp_sockets = 8          # bounded shared UDP sockets
tcp_pool = 64            # bounded TCP connection pool

[profile]
id = "generic"
standard = "GB/T 28181-2022"
keepalive_ms = 60000
catalog_items = 2
synthetic_vendor = false # true = behavioural fixture, NOT interop evidence

[[steps]]
kind = "catalog_query"   # invite | bye
at_ms = 90000

[[faults]]
kind = "drop"            # delay | reorder | duplicate | half_packet | malformed | sip_error
rate = 0.02
direction = "device_to_platform"  # platform_to_device | both
target = "any"           # register | keepalive | catalog | media | message
```

Fault kinds:

| kind | effect |
| --- | --- |
| `drop` | discards the frame |
| `delay` | adds `extra_ms` + uniform `jitter_ms` latency |
| `reorder` | holds the frame back by up to `window` delivery slots |
| `duplicate` | delivers an extra byte-identical copy |
| `half_packet` | splits the frame into two chunks (TCP reassembly only) |
| `malformed` | corrupts the frame so parsing fails (no panic) |
| `sip_error` | platform answers the matched request with a SIP error code |

Validation rejects contradictory configuration (zero shards/devices/duration,
`rate` outside `[0,1]`, zero reorder window, `sip_error` code outside
`[300,699]`, `half_packet` on UDP, and steps scheduled after `duration_ms`).

## Usage

```bash
# Run a scenario file and print the JSON report.
cargo run -p cheetah-gb28181-simulator -- --scenario tools/gb28181-simulator/scenarios/baseline.toml

# Quick flag-driven smoke run (no scenario file).
cargo run -p cheetah-gb28181-simulator -- --count 10 --seed 42 --transport udp --drop-rate 0.05

# Write the report to a file instead of stdout.
cargo run -p cheetah-gb28181-simulator -- --scenario scenarios/faults-tcp.toml --report /tmp/report.json
```

The library entry point is `cheetah_gb28181_simulator::run_scenario(scenario)
-> RunReport`.

## Allowed dependencies

- `cheetah-gb28181-core` for Sans-I/O SIP encoding/parsing and digest.
- `cheetah-gb28181-module` for GB28181 XML builders/parsers.
- `serde`/`serde_json`/`toml` for the DSL and report, `sha2` for seeded RNG and
  transcript hashing, `rand` for RNG streams, `secrecy` for passwords,
  `clap` for the CLI, `tracing` for logs.

## Forbidden dependencies

- No SQLx, NATS, media engine, gRPC client, or cluster registry.
- No real media payload handling and no media port binding.
- No `SystemTime::now()`/`Instant::now()` in simulation logic: the virtual clock
  and seeded RNGs drive all timing and randomness.
