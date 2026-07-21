# cheetah-gb28181-driver-tokio

Tokio-based UDP/TCP driver for the GB28181 protocol core. It owns the network
sockets, transaction timer injection and transport-side NAT address handling, and
forwards parsed SIP messages to any `GbAccessMachine` implementation supplied by
the assembly crate.

## Responsibilities

- UDP and TCP listening sockets for GB28181 SIP traffic, over IPv4 and IPv6,
  across any number of bind addresses (`DriverConfig::with_udp_bind` /
  `with_tcp_bind`).
- Datagram dispatch with a bounded receive buffer; datagrams larger than
  `max_datagram_size` are rejected rather than truncated.
- Incremental TCP stream framing that handles half packets and coalesced
  messages via the core parser's `feed`/`pop_message`.
- Bounded connection lifecycle: global and per-source TCP connection limits,
  per-connection read-buffer bound (via the parser), idle timeout, and
  write backpressure by awaiting the socket instead of buffering unbounded.
- Cancellation and a bounded shutdown drain that stop admission and release
  sockets, permits, per-source slots and timers.
- Generic execution of `GbAccessMachine` implementations from `cheetah-gb28181-core`.

## Allowed dependencies

- `cheetah-gb28181-core` for Sans-I/O SIP parsing/encoding and the `GbAccessMachine` contract.
- `tokio`, `tokio-util` (`CancellationToken`), `tracing`, and standard Rust crates.
- `cheetah-gb28181-module` only as a dev-dependency for tests.

## Forbidden dependencies

- No direct production dependency on `cheetah-gb28181-module` or `cheetah-signal-application`.
- No SQLx, NATS, media, HTTP client or database crates.

## Features

No optional features.

## Public entry

`lib.rs` exposes `Gb28181UdpDriver::bind`, which accepts a pre-constructed
`GbAccessMachine` and a type-erased `EventSink`, plus `DriverConfig` and
`DriverError`. Run the driver with `run` (runs until socket error) or
`run_with_cancellation`, which returns after cancellation once the bounded
shutdown drain completes.
