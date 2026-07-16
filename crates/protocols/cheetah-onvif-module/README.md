# cheetah-onvif-module

ONVIF protocol module for Cheetah Signaling. This crate maps ONVIF service
requests and responses to the internal signaling model and defines the ports
used by the Tokio driver.

## Scope

- ONVIF Device, Media/Media2, PTZ and Events service request builders and
  response parsers.
- Mapping of device information, profiles, stream URIs, presets and events to
  internal value objects.
- Provisioning workflow state and capability probing results.
- Sans-I/O: no UDP sockets, HTTP clients, clocks or random sources.

## Allowed dependencies

- `cheetah-onvif-core` for wire-level builders, parsers and security helpers.
- `cheetah-signal-types` for shared identifiers, timestamps and ports.
- `quick-xml` for additional XML helpers.
- `url` for stream/snapshot URI normalization and validation.
- `secrecy` for password handling.
- `thiserror` and `tracing`.

## Forbidden dependencies

No Tokio, reqwest, socket2, async runtime, database clients, NATS or media
clients. Those belong to `cheetah-onvif-driver-tokio`.
