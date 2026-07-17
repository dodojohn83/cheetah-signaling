# protocol-plugin-example

A minimal out-of-process protocol plugin that Cheetah hosts can load through the
`PluginRuntime` gRPC bridge.

It demonstrates:

- Reading the listen address and plugin name from environment variables.
- Generating a self-signed certificate whose SAN URI matches the plugin name.
- Exposing the `PluginRuntime` service using `cheetah-plugin-testkit`.
- Graceful shutdown on `SIGINT`/`SIGTERM`.

## Public entry point

- `src/main.rs` — the `protocol-plugin-example` binary.

## Features

None.

## Allowed dependencies

- `cheetah-plugin-testkit`, `cheetah-plugin-sdk`, `cheetah-signal-contracts`
- `tokio`, `tonic`, `tracing`, `tempfile`

## Prohibited dependencies

No real device SDKs, no `unsafe` FFI, and no production secrets, databases,
NATS clients, or media clients. This example is only for local testing and
integration against the plugin gRPC contract.

## Run

```bash
cargo run -p protocol-plugin-example
```

Set `CHEETAH_PLUGIN_LISTEN_ADDRESS` to bind to a specific address, and
`CHEETAH_PLUGIN_NAME` to match the configured plugin identity.

This example is for integration testing and local development; it does not
connect to real devices.
