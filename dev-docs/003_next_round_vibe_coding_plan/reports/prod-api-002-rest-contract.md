# PROD-API-002 (partial) + GB auth production default

## Changes

### REST contract

1. **`JsonBody` extractor** (`cheetah-http-api::json_body`): invalid JSON returns
   HTTP 400 `INVALID_JSON` Problem Details instead of Axum plain-text 422.
2. **Request correlation**: middleware assigns/echoes `x-request-id` and injects
   `request_id` into RFC 9457 problem bodies when missing.
3. **Async create `Location`**:
   - `POST /api/v1/operations` → `Location: /api/v1/operations/{id}`
   - `POST /api/v1/media/sessions` → `Location: /api/v1/media/sessions/{id}`
4. Write paths for tenants/devices/operations/media use `JsonBody`.

### Production bug: GB28181 auth

- Assembly previously forced `AuthPolicy::ChallengeOptional` (accepts
  unauthenticated REGISTER). That violates GB-002 / production policy.
- Default is now `AuthPolicy::Required`.
- Opt-in only via `gb28181.challenge_optional = true` (documented as
  development-only in `config.example.toml`).

## Verification

```bash
export PROTOC=$HOME/.local/bin/protoc
export PROTOC_INCLUDE=$HOME/.local/include
cargo test -p cheetah-http-api --tests
cargo clippy -p cheetah-http-api -p cheetah-signaling -p cheetah-signal-types \
  --all-targets -- -D warnings
```

- `cheetah-http-api` tests: PASS (including Location + request_id + invalid JSON)
- clippy: see command output for this change set

## If-Match / ETag (added)

- `IfMatchRevision` extractor: required `If-Match` with numeric/`"n"`/`W/"n"` ETag.
- Device and webhook PATCH compare against current revision; mismatch →
  `FAILED_PRECONDITION` (HTTP 412).
- GET/update responses set `ETag: "{revision}"`.
- `Idempotency-Key` is now **required** (no silent UUID synthesis).

## Remaining PROD-API-002

- Full error-matrix tests (409/429/timeout/unsupported/unavailable)
- OpenAPI complete schemas for all DTOs
- Channel catalog updates If-Match (if exposed as PATCH)

Refs: PROD-API-002, GB-002, AUD-GAP-002
