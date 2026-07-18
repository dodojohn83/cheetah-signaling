# cheetah-onvif-simulator

An ONVIF device simulator for testing Cheetah Signaling's ONVIF driver.
It exposes HTTP SOAP endpoints for Device, Media, PTZ and Events services,
validates WS-Security username tokens (plain password or SHA-1 digest),
returns a synthetic system date/time with configurable clock drift, and
answers WS-Discovery `Probe` requests over UDP.

## Responsibilities

- HTTP SOAP handlers for Device, Media, PTZ and Events requests.
- WS-Security `UsernameToken` authentication (plain `Password` or
  `PasswordDigest` computed from `Nonce` + `Created` + password).
- `GetSystemDateAndTime` and `GetCapabilities` are accessible without
  authentication, matching typical ONVIF device behavior.
- Synthetic device information, capabilities, profiles, stream URI, PTZ nodes
  and event pull-point subscription responses.
- Configurable clock drift on `GetSystemDateAndTime`.
- Configurable synthetic SOAP Fault injection rate for resilience tests.
- UDP WS-Discovery `Probe` response with `--xaddr-host` override.

## Allowed dependencies

- `axum` for the HTTP server.
- `quick-xml` for SOAP parsing and typed XML generation.
- `sha1`, `base64`, `secrecy` for WSSE digest validation.
- `time` for drift-aware UTC date/time formatting.
- `url` for WS-Discovery `XAddrs` construction.
- `tokio`, `clap`, `tracing`, `rand`.

## Forbidden dependencies

- No SQLx, NATS, gRPC, media engine, or cluster registry.
- No `SystemTime::now()` in reusable parsing/auth helpers (only in runtime
  handlers for clock drift and timestamps).

## Features

- `default` only.

## Public entry points

- `cargo run -p cheetah-onvif-simulator -- --bind 127.0.0.1:8080 --xaddr-host 192.168.1.10 --user admin --password admin`
