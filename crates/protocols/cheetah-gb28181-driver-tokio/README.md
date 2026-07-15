# cheetah-gb28181-driver-tokio

Tokio-based UDP/TCP driver for the GB28181 protocol core. It owns the network
sockets, transaction timer injection and transport-side NAT address handling, and
forwards parsed SIP messages to the upper protocol module layer (wired by the
assembly crate).

## Responsibilities

- UDP and TCP listening sockets for GB28181 SIP traffic.
- Byte-stream framing, datagram dispatch and source address tracking.
- SIP transaction timer injection and retransmission handling.
- Transport-side NAT address rewriting before passing inputs to the core state machine.

## Allowed dependencies

- `cheetah-gb28181-core` for Sans-I/O SIP parsing and encoding.
- `tokio`, `tracing`, and standard Rust crates.

## Forbidden dependencies

- No direct dependency on `cheetah-gb28181-module` or `cheetah-signal-application`.
- No SQLx, NATS, media, HTTP client or database crates.

## Features

No optional features.

## Public entry

`lib.rs` exposes the driver builder and UDP/TCP listener entry points once the
module and core wiring is complete.
