#!/usr/bin/env bash
set -Eeuo pipefail

IMAGE="${SAFE_NODE_IMAGE:-hypesafe/safe-node:local}"
RUN_DIR="${SAFE_NODE_RUN_DIR:-$PWD}"
mkdir -p "$RUN_DIR"
RUN_DIR="$(cd -- "$RUN_DIR" && pwd)"

CONFIG_FILE="${SAFE_NODE_CONFIG_FILE:-/app/config/config.json}"
ENV_FILE="${SAFE_NODE_ENV_FILE:-$RUN_DIR/.env}"

env_args=()
if [[ -f "$ENV_FILE" ]]; then
  env_args=(--env-file "$ENV_FILE")
fi

cmd=(once --config "$CONFIG_FILE")
if [[ "${SAFE_NODE_DRY_RUN:-}" == "1" || "${SAFE_NODE_DRY_RUN:-}" == "true" ]]; then
  cmd+=(--dry-run)
fi
cmd+=("$@")

docker run --rm \
  --name "${SAFE_NODE_EXECUTOR_NAME:-safe-node-executor}" \
  --user "${SAFE_NODE_UID:-$(id -u)}:${SAFE_NODE_GID:-$(id -g)}" \
  --env HOME=/tmp \
  "${env_args[@]}" \
  --volume "$RUN_DIR:/app/config:ro" \
  --volume "$RUN_DIR:/app/data" \
  "$IMAGE" \
  "${cmd[@]}"
