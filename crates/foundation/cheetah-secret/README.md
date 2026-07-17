# cheetah-secret

Secret provider implementations for Cheetah Signaling.

## Public entry points

- `cheetah_secret::InMemorySecretStore`
- `cheetah_secret::EnvSecretStore`
- `cheetah_secret::FileSecretStore`
- `cheetah_secret::CompositeSecretStore`

## Features

None.

## Allowed dependencies

- `cheetah-signal-types`
- `secrecy`, `uuid`

## Prohibited dependencies

No real external secret manager SDKs, cloud IAM clients, or network I/O. This
crate only provides local env/file/memory sources and a composable layering
helper.

## Design

All implementations use `secrecy::SecretString` to prevent accidental `Debug`
leaks. `EnvSecretStore` normalizes keys to upper case and replaces
non-alphanumeric characters with `_`. `FileSecretStore` stores one secret per
plain-text file under a configured directory and rejects keys that contain path
separators. `CompositeSecretStore` layers sources: `get` returns the first match
while mutating operations succeed with the first writable store that accepts.
