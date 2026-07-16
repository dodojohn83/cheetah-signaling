# cheetah-onvif-core

Sans-I/O ONVIF core crate for the Cheetah signaling server.

## Scope

This crate contains protocol-agnostic ONVIF wire types, serializers and parsers:

- WS-Discovery `Probe`/`ProbeMatches`/`Hello`/`Bye`/`Resolve` message builders and parsers.
- SOAP 1.2 envelope builder and `Fault` parser.
- WS-Security `UsernameToken` with `PasswordDigest` and `PasswordText` support.
- XAddr SSRF filtering (scheme, IP segment, loopback/link-local/private checks).

It intentionally contains no UDP sockets, HTTP clients, clocks or random sources.
Those belong to `cheetah-onvif-driver-tokio` (or a future driver adapter), which
injects `AppId`, nonce, timestamps and executes outbound requests.

## Allowed dependencies

- `quick-xml` for XML serialization and parsing.
- `url` for XAddr validation.
- `base64`, `hmac`, `sha2`, `md5` for WS-Security digests.
- `secrecy` for password handling.
- `time` for timestamp formatting (from injected Unix seconds).
- `thiserror` for error types.

## Forbidden dependencies

No Tokio, reqwest, socket2, async runtime, database clients, NATS or media clients.
