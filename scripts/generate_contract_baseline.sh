#!/usr/bin/env bash
# Generate the `cheetah.media.v1` contract descriptor and breaking baseline.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
PROTO_DIR="${ROOT_DIR}/proto"
OUT_DIR="${ROOT_DIR}/target/contract-baseline"

mkdir -p "${OUT_DIR}"

# Build the current proto tree into a FileDescriptorSet.
buf build "${PROTO_DIR}" --as-file-descriptor-set -o "${OUT_DIR}/descriptor.bin"

# Verify the current proto does not break against the main branch baseline.
# The baseline is the last released contract state stored in git.
buf breaking "${PROTO_DIR}" --against ".git#branch=origin/main"

echo "Contract baseline generated at ${OUT_DIR}/descriptor.bin"
