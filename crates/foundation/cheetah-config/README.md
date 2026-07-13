# cheetah-config

Layered configuration loader for Cheetah Signaling.

Merges defaults, TOML files, environment variables and secrets into a
validated `SignalConfig`. Implements the `ConfigSource` port defined in
`cheetah-signal-types`.

## Dependencies

- `cheetah-signal-types` for `SignalConfig` and `ConfigSource`.
- `config` for layered source resolution.
- `toml` for default configuration serialization.
