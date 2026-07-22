#!/usr/bin/env bash
# Publish a versioned `cheetah.media.v1` contract tag and descriptor checksum.
# Usage: publish_proto_tag.sh <version>
set -euo pipefail

VERSION="${1:?contract version required (e.g. 1.0.0)}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
OUT_DIR="${ROOT_DIR}/target/contract-baseline"
DESCRIPTOR="${OUT_DIR}/descriptor.bin"
CHECKSUM="${DESCRIPTOR}.sha256"
TAG="proto/cheetah.media.v1/v${VERSION}"

cd "${ROOT_DIR}"

# Ensure descriptor is up-to-date.
./scripts/generate_contract_baseline.sh

# Compute checksum.
sha256sum "${DESCRIPTOR}" | awk '{print $1}' > "${CHECKSUM}"

echo "Tag:    ${TAG}"
echo "File:   ${DESCRIPTOR}"
echo "SHA256: $(cat "${CHECKSUM}")"

# Create an annotated tag pointing at the current commit.
git tag -a "${TAG}" -m "cheetah.media.v1 contract release v${VERSION}"

echo "Created annotated tag ${TAG}."
echo "Push with: git push origin ${TAG}"
echo "Attach ${DESCRIPTOR} and ${CHECKSUM} to the GitHub release."
