# cheetah-gb28181-simulator

A multi-device GB28181 simulator for testing Cheetah Signaling's GB28181
protocol driver.  Each device binds its own UDP port, performs digest REGISTER,
sends periodic keepalives, answers `Catalog` queries, and responds to `INVITE`
playback requests.

## Responsibilities

- Spawn one or more GB28181 devices with stable, seed-derived device IDs.
- Perform UDP SIP registration with MD5/SHA-256 digest authentication.
- Send `Keepalive` and optional `Alarm` MESSAGE requests.
- Respond to platform `Catalog` queries with a `Response` MESSAGE.
- Reply to `INVITE` with `100 Trying` and `200 OK` carrying synthetic SDP.
- Inject malformed datagrams and command failures for resilience testing.
- Support vendor profiles (`generic`, `dahua`, `hikvision`) with different
  heartbeat intervals and catalog shapes.
- Simulate periodic disconnect/reconnect with a seeded RNG pause duration and
  forced re-registration.
- Clamp `failure_rate` and `malformed_rate` to `[0.0, 1.0]` and respect a
  configurable initial keepalive delay.

## Allowed dependencies

- `cheetah-gb28181-core` for Sans-I/O SIP encoding/parsing and digest.
- `cheetah-gb28181-module` for GB28181 XML builders/parsers.
- `tokio` UDP and timers, `clap` for CLI, `tracing` for logs.
- `rand` for seeded RNG, `secrecy` for password handling.

## Forbidden dependencies

- No SQLx, NATS, media engine, gRPC client, or cluster registry.
- No `SystemTime::now()` in state machine logic (only used for SIP response
  timestamps if needed).

## Features

- `default` only.  CLI flags control all simulation behavior.

## Public entry points

- `cargo run -p cheetah-gb28181-simulator -- --server 127.0.0.1:5060 --count 10 --seed 42 --disconnect-every-n-heartbeats 3`
