# `cheetah.media.v1` Contract Release Process

This document defines how the `cheetah.media.v1` protobuf contract is versioned,
tagged and consumed by downstream repositories (e.g. `cheetah-media-server-rs`).

## Versioning

- The contract version is a monotonically increasing non-negative integer.
- The current supported range is declared in
  `crates/foundation/cheetah-signal-contracts/src/version.rs`:
  - `MINIMUM_SUPPORTED_CONTRACT_VERSION`
  - `MAXIMUM_SUPPORTED_CONTRACT_VERSION`
  - `ROLLING_UPGRADE_WINDOW_SECONDS`
- Backward-compatible v1 extensions may be added without bumping the maximum
  version. Breaking changes require a new major contract version and a migration
  window.

## Releasing a contract tag

1. Make sure `scripts/generate_contract_baseline.sh` passes and the generated
   `target/contract-baseline/descriptor.bin` reflects the contract state you
   intend to publish.
2. Run `scripts/publish_proto_tag.sh <version>` (e.g. `1.0.0`). The script:
   - Creates an annotated Git tag `proto/cheetah.media.v1/v<version>`.
   - Computes the SHA-256 checksum of `target/contract-baseline/descriptor.bin`.
   - Writes the checksum to `target/contract-baseline/descriptor.bin.sha256`.
   - Prints the tag and checksum for the release notes.
3. Push the tag: `git push origin proto/cheetah.media.v1/v<version>`.
4. Attach `descriptor.bin` and `descriptor.bin.sha256` to the GitHub release
   associated with that tag. Downstream media repositories consume the contract
   by the tag and verify the checksum; they must not copy the proto tree and
   evolve it independently.

## Rolling upgrade window

During a contract bump, media nodes running the previous supported version
continue to be accepted for `ROLLING_UPGRADE_WINDOW_SECONDS` (default 24 hours).
After the window, nodes must re-register with a supported contract version.
