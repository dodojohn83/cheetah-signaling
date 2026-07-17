# cheetah-plugin-host

Plugin host for Cheetah Signaling. Loads and manages built-in protocol drivers
via the `cheetah-plugin-sdk` ports.

## Public entry points

- [`host.rs`](src/host.rs): `PluginHost` and `HostDriverContext` for driver
  lifecycle, manifest validation, health aggregation and bounded operations.
- [`loader.rs`](src/loader.rs): `ManifestLoader` validates manifests, checksums
  and SDK version negotiation.
- [`registry.rs`](src/registry.rs): `BuiltInRegistry` maps plugin names to
  `ProtocolDriverFactory` implementations.
- [`error.rs`](src/error.rs): `PluginHostError` for host-level failures.

## Design constraints

- The host never exposes database connection pools, NATS clients or global
  configuration to drivers.
- All driver operations accept a `DurationMs` timeout and are wrapped with
  `tokio::time::timeout`.
- Manifest validation, version negotiation and checksum verification run
  before a driver is instantiated.
- A failed activation does not overwrite an existing instance.
