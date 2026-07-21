# PROD-API-002：HTTP 错误矩阵

## Summary

Added integration tests covering stable RFC 9457 Problem Details for common
failure statuses used by the northbound API.

| Status | Code | Coverage |
| --- | --- | --- |
| 400 | `INVALID_ARGUMENT` | Missing `Idempotency-Key` |
| 401 | `UNAUTHENTICATED` | No credentials (+ `request_id` echo) |
| 403 | `PERMISSION_DENIED` | JWT tenant header mismatch; missing `operator` scope |
| 404 | `NOT_FOUND` | Missing device; unknown route fallback |
| 412 | `FAILED_PRECONDITION` | Stale `If-Match` (see prod_api_test) |
| 429 | `RATE_LIMITED` | Burst=1 rate limit on `/api/v1/devices` |

## Implementation notes

- Extended `TestServer` with `TestServerOptions` (rate limit, JWT PEM, disable static key).
- Added RSA fixtures under `crates/api/cheetah-http-api/tests/fixtures/` (test-only keys).
- New test binary: `tests/error_matrix_test.rs`.

## Commands

```bash
export PROTOC=$HOME/.local/bin/protoc
export PROTOC_INCLUDE=$HOME/.local/include
cargo test -p cheetah-http-api --test error_matrix_test
cargo test -p cheetah-http-api --tests
```

## Results

- `error_matrix_test`: 7 passed
- Existing API tests remain green with the shared harness change

## Remaining

- 409 conflict (storage concurrent revision race via parallel writers)
- 504 timeout / 503 unavailable / 501 unsupported capability paths
- Sensitive-field redaction assertions on error bodies

Refs: PROD-API-002
