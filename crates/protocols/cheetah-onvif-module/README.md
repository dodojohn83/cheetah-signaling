# cheetah-onvif-module

ONVIF protocol module for Cheetah Signaling. This crate maps ONVIF service
requests and responses to the internal signaling model, defines ports used by
the Tokio driver, and exposes a built-in plugin-host driver factory.

## Scope

- ONVIF Device service request builders and response parsers (GetServices,
  GetCapabilities, GetDeviceInformation, GetSystemDateAndTime, hostname and
  network interfaces).
- Provisioning workflow state and capability probing results.
- Plugin-host [`ProtocolDriver`] adapter (lifecycle + Unsupported command
  surface until full command dispatch is wired).
- Sans-I/O business logic: no UDP sockets, HTTP clients, clocks or random
  sources in the provisioning/service modules.

## Allowed dependencies

- `cheetah-onvif-core` for wire-level builders, parsers and security helpers.
- `cheetah-plugin-sdk` for the built-in driver factory port.
- `cheetah-signal-types` for shared identifiers, timestamps and ports.
- `quick-xml` for additional XML helpers.
- `url` for stream/snapshot URI normalization and validation.
- `secrecy` for password handling.
- `async-trait`, `serde_json`, `thiserror`, `tokio` (sync only for driver trait).

## Dev dependencies

- `uuid` for deterministic test fixtures.
- `tokio` test runtime for driver lifecycle tests.

## Forbidden dependencies

No reqwest, socket2, database clients, NATS or media clients in service/workflow
modules. Network I/O belongs to `cheetah-onvif-driver-tokio`.
