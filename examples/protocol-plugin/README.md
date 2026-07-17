# protocol-plugin-example

A minimal out-of-process protocol plugin that Cheetah hosts can load through the
`PluginRuntime` gRPC bridge.

It demonstrates:

- Reading the listen address and plugin name from environment variables.
- Generating a self-signed certificate whose SAN URI matches the plugin name.
- Exposing the `PluginRuntime` service using `cheetah-plugin-testkit`.
- Graceful shutdown on `SIGINT`/`SIGTERM`.

## Run

```bash
cargo run -p protocol-plugin-example
```

Set `CHEETAH_PLUGIN_LISTEN_ADDRESS` to bind to a specific address, and
`CHEETAH_PLUGIN_NAME` to match the configured plugin identity.

This example is for integration testing and local development; it does not
connect to real devices.
