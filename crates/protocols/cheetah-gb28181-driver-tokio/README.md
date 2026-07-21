# cheetah-gb28181-driver-tokio

Tokio-based UDP/TCP driver for the GB28181 protocol core. It owns the network
sockets, transaction timer injection and transport-side NAT address handling, and
forwards parsed SIP messages to any `GbAccessMachine` implementation supplied by
the assembly crate.

## Responsibilities

- UDP and TCP listening sockets for GB28181 SIP traffic.
- Byte-stream framing, datagram dispatch and source address tracking.
- SIP transaction timer injection and retransmission handling.
- Transport-side NAT address rewriting before passing inputs to the core state machine.
- Generic execution of `GbAccessMachine` implementations from `cheetah-gb28181-core`.

## Allowed dependencies

- `cheetah-gb28181-core` for Sans-I/O SIP parsing/encoding and the `GbAccessMachine` contract.
- `tokio`, `tracing`, and standard Rust crates.
- `cheetah-gb28181-module` only as a dev-dependency for tests.

## Forbidden dependencies

- No direct production dependency on `cheetah-gb28181-module` or `cheetah-signal-application`.
- No SQLx, NATS, media, HTTP client or database crates.

## Features

No optional features.

## Public entry

`lib.rs` exposes `Gb28181UdpDriver::bind`, which accepts a pre-constructed
`GbAccessMachine` and a type-erased `EventSink`, and `DriverConfig`.
