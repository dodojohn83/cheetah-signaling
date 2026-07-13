# Cheetah Signaling Protobuf Contracts

Versioned cross-process and persistence wire schema.

- `cheetah/media/v1/` — media plane contracts
- `cheetah/signaling/v1/` — control plane contracts
- `cheetah/plugin/v1/` — plugin SDK contracts

Managed by `buf` v2. Use `buf lint` and `buf breaking --against '.git#branch=main'` before merging.
