# cheetah-plugin-sdk

Cheetah Signaling protocol plugin SDK. Defines the manifest, negotiation,
checksum validation, driver ports, and restricted host capabilities used by
both built-in and out-of-process protocol plugins.

## Public entry points

- [`manifest`](src/manifest.rs): `PluginManifest`, `PluginName`, `PluginVersion`,
  `SdkVersionReq`, protocol capabilities, permissions, resource budgets and
  configuration schema.
- [`version`](src/version.rs): SDK version negotiation between host and plugin.
- [`checksum`](src/checksum.rs): `sha256` and `hmac-sha256` manifest integrity
  verification.
- [`driver`](src/driver.rs): `ProtocolDriver`, `ProtocolDriverFactory`,
  `DriverContext`, `DeviceSink` and `CommandSource` ports.
- [`error`](src/error.rs): stable `PluginError` classification.

## Design constraints

- The SDK must not expose database connection pools, NATS clients, global
  configuration objects or unbounded buffers to drivers.
- All driver methods accept a deadline/cancellation token through their
  `DriverContext` and return `PluginError`.
- Driver commands and events use typed envelopes with a stable command/event
  type and JSON payload, so the host can route them without parsing
  protocol-specific wire formats.
