# PROD-API-001: 清除公开 NotImplemented 占位

## Summary

Closed the routable HTTP 501 placeholders listed in AUD-GAP-004 / PROD-API-001:

| Endpoint | Previous | Now |
| --- | --- | --- |
| `GET /api/v1/tenants` | `NotImplemented` | Paginated list via `TenantRepository` |
| `POST /api/v1/tenants` | `NotImplemented` | Create (idempotent on id+name) |
| `GET /api/v1/nodes` | `NotImplemented` | Alive cluster nodes via `NodeRepository::list_alive` |
| `GET /api/v1/media/sessions/{id}` | `NotImplemented` | Load via `MediaSessionRepository::get` |
| `POST /api/v1/operations` | `NotImplemented` | `OperationService::submit_operation` (202) |
| `GET /api/v1/media-nodes` | Incomplete pagination | Registry-backed cursor list with media-port fallback |

Also fixed HTTP integration tests that returned **502** under ambient `http_proxy` when targeting `127.0.0.1` (proxy bypass via `reqwest::ClientBuilder::no_proxy()`).

## Tenant stack

- Domain: `cheetah_domain::Tenant`
- Port: `cheetah_storage_api::TenantRepository` on `Storage`
- Adapters: SQLite + PostgreSQL
- HTTP: create + list with cursor pagination and `name_prefix`

## Verification

```bash
export PROTOC=$HOME/.local/bin/protoc
export PROTOC_INCLUDE=$HOME/.local/include
cargo fmt --all
cargo clippy -p cheetah-http-api -p cheetah-domain -p cheetah-storage-api \
  -p cheetah-storage-sqlite -p cheetah-storage-postgres --all-targets -- -D warnings
cargo test -p cheetah-http-api --tests
cargo check -p cheetah-signaling
```

Results:

- clippy (affected crates): PASS
- `cheetah-http-api` prod_api_test / device_test / health_test: PASS
- remaining handler `NotImplemented` only when webhook service is not wired (optional assembly path)

## Remaining PROD-API work (out of scope for this change)

- PROD-API-002 full error matrix and OpenAPI completeness
- PROD-API-003 SSE/Webhook edge cases
- Full workspace nextest under official CI image

Refs: PROD-API-001, AUD-GAP-004
