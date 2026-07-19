# BAS-004: Architecture and Placeholder Audit

- Dependency layer violations: 3
- Forbidden dependency warnings: 2
- Production `todo!` / `unimplemented!` / `panic!` hits: 0
- Test-fake `todo!` / `unimplemented!` hits: 0
- Direct SQL outside storage crates: 0

## Dependency layer violations

- LAYER VIOLATION: cheetah-gb28181-driver-tokio (layer 5) depends on cheetah-gb28181-module (layer 4)
- LAYER VIOLATION: cheetah-media-scheduler (layer 3) depends on cheetah-media-client (layer 2)
- LAYER VIOLATION: cheetah-onvif-driver-tokio (layer 5) depends on cheetah-onvif-module (layer 4)

## Forbidden dependency warnings

- FORBIDDEN DEP: cheetah-signal-contracts (layer 6) -> tonic
- FORBIDDEN DEP: cheetah-signal-contracts (layer 6) -> tonic-prost

## Production placeholder hits


## Test-fake placeholder hits


## Direct SQL outside storage crates

