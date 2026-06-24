#!/usr/bin/env bash
set -Eeuo pipefail

IMAGE="${SAFE_NODE_IMAGE:-hypesafe/safe-node:local}"
CONTAINER_NAME="${SAFE_NODE_CONTAINER_NAME:-safe-node}"
RUN_DIR="${SAFE_NODE_RUN_DIR:-$PWD}"
mkdir -p "$RUN_DIR"
RUN_DIR="$(cd -- "$RUN_DIR" && pwd)"

CONFIG_FILE="${SAFE_NODE_CONFIG_FILE:-/app/config/config.json}"
ENV_FILE="${SAFE_NODE_ENV_FILE:-$RUN_DIR/.env}"
RPC_HTTP_PORT="${SAFE_NODE_RPC_HTTP_PORT:-9909}"

env_args=()
if [[ -f "$ENV_FILE" ]]; then
  env_args=(--env-file "$ENV_FILE")
fi

docker run --detach \
  --name "$CONTAINER_NAME" \
  --restart unless-stopped \
  --user "${SAFE_NODE_UID:-$(id -u)}:${SAFE_NODE_GID:-$(id -g)}" \
  --env HOME=/tmp \
  "${env_args[@]}" \
  --publish "127.0.0.1:${RPC_HTTP_PORT}:9909" \
  --volume "$RUN_DIR:/app/config:ro" \
  --volume "$RUN_DIR:/app/data" \
  "$IMAGE" \
  run --config "$CONFIG_FILE"

echo "Started $CONTAINER_NAME from $IMAGE"
echo "Logs: docker logs -f $CONTAINER_NAME"
