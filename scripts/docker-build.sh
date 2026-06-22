#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
NODE_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"

IMAGE="${SAFE_NODE_IMAGE:-hypesafe/safe-node:local}"

docker build \
  --tag "$IMAGE" \
  "$NODE_DIR"

echo "Built $IMAGE"
