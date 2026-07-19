# cheetah-onvif-driver-tokio

Tokio network driver for ONVIF.

## Scope

- WS-Discovery Probe over UDP multicast with datagram size limits and XAddr SSRF filtering.
- SOAP 1.2 HTTP client with connect/request deadlines, response body limits, concurrency caps,
  and per-request SSRF validation (including optional redirect re-check).
- Convenience helpers for Device Information, SystemDateAndTime, Media profiles/URIs.

## Allowed dependencies

- `cheetah-onvif-core`, `cheetah-onvif-module`, `cheetah-signal-types`
- `reqwest`, `tokio`, `tracing`, `url`, `uuid`, `thiserror`

## Forbidden

No SQLx, NATS, media codec processing, or domain aggregate mutation. Domain mapping stays in
the module; media RTP still belongs to the media plane.
