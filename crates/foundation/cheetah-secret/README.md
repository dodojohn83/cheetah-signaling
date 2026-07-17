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
- `libc` on Unix targets only, for portable `O_NOFOLLOW`

## Prohibited dependencies

No real external secret manager SDKs, cloud IAM clients, or network I/O. This
crate only provides local env/file/memory sources and a composable layering
helper.

## Design

All implementations use `secrecy::SecretString` to prevent accidental `Debug`
leaks. `EnvSecretStore` normalizes keys to upper case and replaces
non-alphanumeric characters with `_`. `FileSecretStore` stores one secret per
plain-text file under a configured directory, rejects keys that contain path
separators or traversal components, and reads/writes the exact file bytes. Files
created through the store are created without following symlinks and with
owner-only (`0o600`) permissions on Unix. `CompositeSecretStore` layers sources:
`get` returns the first match, `put`/`rotate` succeed with the first writable
store that accepts, and `delete` removes the key from every layer so that no
readable copy remains.
