# cheetah-protocol-gb28181-core

Sans-I/O GB28181 SIP message model, URI handling, parser and encoder.

## Responsibility

- SIP message types, methods, status codes, headers and URI.
- Streaming parser for UDP datagrams and TCP byte streams.
- Stable CRLF encoder with correct `Content-Length`.
- Configurable size limits and safe error reporting without echoing credentials.

## Allowed dependencies

- `bytes`, `thiserror`, `tracing`.
- Dev-only test helpers.

## Forbidden dependencies

- No Tokio, async runtime, socket, database, HTTP client, or protocol-specific media handling.
- No domain/application imports from `cheetah-signal-application` or `cheetah-domain`.

## Public entry

`lib.rs` re-exports `sip` modules and the primary `SipParser`, `SipMessage`, `Method`, `StatusLine`, `SipUri`, `HeaderName`, and `encode_message`.
