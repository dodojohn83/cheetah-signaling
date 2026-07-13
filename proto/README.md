# Cheetah Signaling Protobuf Contracts

Versioned cross-process and persistence wire schema.

- `cheetah/common/v1/` — shared types, IDs, envelope, errors and pagination
- `cheetah/device/v1/` — device domain snapshot and device events
- `cheetah/control/v1/` — control commands, results and `NodeCommand` service
- `cheetah/plugin/v1/` — plugin SDK runtime frame and `PluginRuntime` service
- `cheetah/media/v1/` — media DTO, media commands/events and media services
- `cheetah/cluster/v1/` — node and media-node registry, cluster events

Managed by `buf` v2. Use `buf lint` and `buf breaking --against '.git#branch=main'` before merging.
