# cheetah-gb28181-core

Sans-I/O GB28181 SIP message model, URI handling, parser and encoder, plus the
`GbAccessMachine` input/output contract.

## Responsibility

- SIP message types, methods, status codes, headers and URI.
- Streaming parser for UDP datagrams and TCP byte streams.
- Stable CRLF encoder with correct `Content-Length`.
- Configurable size limits and safe error reporting without echoing credentials.
- Generic `GbAccessMachine` trait and `AccessInput`/`AccessOutput` types used by
  the module and driver layers.

## Allowed dependencies

- `bytes`, `thiserror`, `tracing`.
- `md5`, `sha2`, `hmac`, `hex`, `subtle`, `secrecy` (digest authentication).
- Dev-only test helpers.

## Forbidden dependencies

- No Tokio, async runtime, socket, database, HTTP client, or protocol-specific media handling.
- No domain/application imports from `cheetah-signal-application` or `cheetah-domain`.

## Features

No optional features.

## Public entry

`lib.rs` re-exports `sip` modules and the primary `SipParser`, `SipMessage`,
`Method`, `StatusLine`, `SipUri`, `HeaderName`, and `encode_message`. Digest
primitives are exposed under `sip::digest`. The access machine contract is
exposed under `access` as `GbAccessMachine`, `AccessInput` and `AccessOutput`.
