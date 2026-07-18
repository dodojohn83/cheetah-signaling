# cheetah-ctl

Operational CLI for Cheetah Signaling. It wraps the administrative HTTP
endpoints and local configuration validation so operators can diagnose and
recover a node without writing raw HTTP requests.

## Responsibility

- Validate a local TOML configuration file without starting the server.
- Query and mutate node-level state via the HTTP admin API:
  - database migration status / run migrations
  - request a graceful node drain
  - replay pending outbox events
  - trigger background reconciliation
  - fetch a sanitized diagnostics package for a device

## Allowed dependencies

- `cheetah-config`: local configuration loading and validation.
- `cheetah-signal-types`: `ConfigSource` trait for `LayeredConfigSource`.
- `clap`: command-line argument parsing.
- `reqwest`: HTTP client for admin API calls.
- `serde_json`: JSON output formatting.
- `tokio`: async runtime.

## Forbidden dependencies

- No direct database drivers (SQLx) or repository access; all persistence
  operations go through the HTTP API.
- No protocol drivers, runtime internals, or secret providers.
- No media engine or control-plane business logic.

## Features

No crate features. The binary always includes all subcommands.

## Public entry points

- `cargo run -p cheetah-ctl -- <subcommand>`
- Installed binary: `cheetah-ctl`

## Configuration

The CLI reads defaults and per-command flags:

- `--base-url` / `CHEETAH_BASE_URL`: HTTP API base URL (default `http://localhost:8080`).
- `--api-key` / `CHEETAH_API_KEY`: API key with `system_admin` scope.
- `--tenant` / `CHEETAH_TENANT_ID`: Tenant UUID. Required for `device-diagnostics`
  and sent as the `x-tenant-id` header. Node-level commands do not need it.

## Subcommands

```bash
cheetah-ctl validate-config /etc/cheetah-signaling/config.toml
cheetah-ctl db-status
cheetah-ctl db-migrate
cheetah-ctl node-drain
cheetah-ctl outbox-replay
cheetah-ctl reconcile
cheetah-ctl device-diagnostics <device-id> --tenant <tenant-uuid>
```
