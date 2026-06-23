#!/usr/bin/env bash
set -Eeuo pipefail

REPO="${SAFE_NODE_REPO:-hypesafe-io/safe-node}"
TAG="${SAFE_NODE_TAG:-}"
DEST_DIR="${SAFE_NODE_INSTALL_DIR:-${SAFE_NODE_DEST_DIR:-$PWD}}"
API_BASE="${GITHUB_API_URL:-https://api.github.com}"
GITHUB_BASE="${GITHUB_SERVER_URL:-https://github.com}"
RAW_BASE="${SAFE_NODE_RAW_BASE:-https://raw.githubusercontent.com}"
TARGET="x86_64-unknown-linux-gnu"
BIN_NAME="${SAFE_NODE_BIN_NAME:-safe-node}"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

curl_get() {
  local url="$1"
  local out="${2:-}"
  local args=(--fail --location --show-error --silent --retry 3)

  if [[ -n "${GITHUB_TOKEN:-}" ]]; then
    args+=(--header "Authorization: Bearer ${GITHUB_TOKEN}")
  fi

  if [[ -n "$out" ]]; then
    curl "${args[@]}" --output "$out" "$url"
  else
    curl "${args[@]}" "$url"
  fi
}

latest_tag() {
  local response

  response="$(curl_get "${API_BASE}/repos/${REPO}/releases/latest")"
  printf '%s\n' "$response" \
    | sed -n 's/^[[:space:]]*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' \
    | head -n 1
}

raw_file_url() {
  local ref="$1"
  local path="$2"

  if [[ -n "${SAFE_NODE_RAW_BASE:-}" ]]; then
    printf '%s/%s/%s/%s\n' "${RAW_BASE%/}" "$REPO" "$ref" "$path"
  else
    printf '%s/%s/raw/%s/%s\n' "${GITHUB_BASE%/}" "$REPO" "$ref" "$path"
  fi
}

write_upgrade_script() {
  local path="$1"

  {
    printf '%s\n' '#!/usr/bin/env bash'
    printf '%s\n' 'set -Eeuo pipefail'
    printf '\n'
    printf 'DEFAULT_REPO=%q\n' "$REPO"
    printf 'DEFAULT_API_BASE=%q\n' "$API_BASE"
    printf 'DEFAULT_GITHUB_BASE=%q\n' "$GITHUB_BASE"
    printf 'DEFAULT_RAW_BASE=%q\n' "$RAW_BASE"
    printf 'DEFAULT_BIN_NAME=%q\n' "$BIN_NAME"
    cat <<'EOF'

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

stop_running_safe_node() {
  local target="${SCRIPT_DIR}/${DEFAULT_BIN_NAME}"
  local pids=()
  local exe
  local pid
  local resolved

  if [[ ! -d /proc ]]; then
    return
  fi

  for exe in /proc/[0-9]*/exe; do
    [[ -e "$exe" ]] || continue
    resolved="$(readlink "$exe" 2>/dev/null || true)"
    if [[ "$resolved" == "$target" || "$resolved" == "${target} (deleted)" ]]; then
      pid="${exe#/proc/}"
      pid="${pid%/exe}"
      pids+=("$pid")
    fi
  done

  if ((${#pids[@]} == 0)); then
    return
  fi

  echo "Stopping running safe-node process(es): ${pids[*]}"
  kill -TERM "${pids[@]}" 2>/dev/null || true

  for _ in {1..30}; do
    local alive=()
    for pid in "${pids[@]}"; do
      if kill -0 "$pid" 2>/dev/null; then
        alive+=("$pid")
      fi
    done

    if ((${#alive[@]} == 0)); then
      return
    fi

    sleep 1
  done

  echo "safe-node is still running; stop it manually and rerun upgrade.sh" >&2
  exit 1
}

require_cmd curl
require_cmd sed
require_cmd readlink

stop_running_safe_node

repo="${SAFE_NODE_REPO:-$DEFAULT_REPO}"
api_base="${GITHUB_API_URL:-$DEFAULT_API_BASE}"
github_base="${GITHUB_SERVER_URL:-$DEFAULT_GITHUB_BASE}"
raw_base="${SAFE_NODE_RAW_BASE:-$DEFAULT_RAW_BASE}"

if [[ -n "${SAFE_NODE_INSTALL_SCRIPT_URL:-}" ]]; then
  install_script_url="$SAFE_NODE_INSTALL_SCRIPT_URL"
elif [[ -n "${SAFE_NODE_RAW_BASE:-}" ]]; then
  install_script_url="${raw_base%/}/${repo}/main/scripts/install.sh"
else
  install_script_url="${github_base%/}/${repo}/raw/main/scripts/install.sh"
fi

echo "Upgrading safe-node in ${SCRIPT_DIR}"
curl_args=(--fail --location --show-error --silent --retry 3)
if [[ -n "${GITHUB_TOKEN:-}" ]]; then
  curl_args+=(--header "Authorization: Bearer ${GITHUB_TOKEN}")
fi

curl "${curl_args[@]}" "$install_script_url" \
  | SAFE_NODE_REPO="$repo" \
    GITHUB_API_URL="$api_base" \
    GITHUB_SERVER_URL="$github_base" \
    SAFE_NODE_RAW_BASE="$raw_base" \
    SAFE_NODE_BIN_NAME="$DEFAULT_BIN_NAME" \
    SAFE_NODE_INSTALL_DIR="$SCRIPT_DIR" \
    bash
EOF
  } > "$path"

  chmod +x "$path"
}

require_cmd curl
require_cmd sed

if [[ -z "$TAG" ]]; then
  TAG="$(latest_tag)"
fi

if [[ -z "$TAG" ]]; then
  echo "Could not determine latest release tag for ${REPO}" >&2
  exit 1
fi

if [[ ! "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Release tag must use vMAJOR.MINOR.PATCH, got: $TAG" >&2
  exit 1
fi

VERSION="${TAG#v}"
ASSET_NAME="safe-node-${VERSION}-${TARGET}"
CHECKSUM="${ASSET_NAME}.sha256"
ASSET_URL="${GITHUB_BASE}/${REPO}/releases/download/${TAG}/${ASSET_NAME}"
CHECKSUM_URL="${GITHUB_BASE}/${REPO}/releases/download/${TAG}/${CHECKSUM}"
EXAMPLE_CONFIG_URL="$(raw_file_url "$TAG" "config/example.node.json")"

mkdir -p "$DEST_DIR"
DEST_DIR="$(cd -- "$DEST_DIR" && pwd)"
BIN_PATH="${DEST_DIR}/${BIN_NAME}"
CONFIG_DIR="${DEST_DIR}/config"
EXAMPLE_CONFIG_PATH="${CONFIG_DIR}/example.node.json"
LATEST_PATH="${DEST_DIR}/latest"
UPGRADE_PATH="${DEST_DIR}/upgrade.sh"

if [[ -e "$BIN_PATH" && ! -f "$BIN_PATH" ]]; then
  echo "Destination binary path exists and is not a file: $BIN_PATH" >&2
  exit 1
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "Installing safe-node ${TAG} from ${REPO}"
curl_get "$ASSET_URL" "${TMP_DIR}/${ASSET_NAME}"
curl_get "$CHECKSUM_URL" "${TMP_DIR}/${CHECKSUM}"
curl_get "$EXAMPLE_CONFIG_URL" "${TMP_DIR}/example.node.json"

if command -v sha256sum >/dev/null 2>&1; then
  (
    cd "$TMP_DIR"
    sha256sum -c "$CHECKSUM"
  )
else
  echo "sha256sum is not available; skipping checksum verification." >&2
fi

chmod +x "${TMP_DIR}/${ASSET_NAME}"
mkdir -p "$CONFIG_DIR"
mv "${TMP_DIR}/${ASSET_NAME}" "$BIN_PATH"
mv "${TMP_DIR}/example.node.json" "$EXAMPLE_CONFIG_PATH"
printf '%s\n' "$TAG" > "$LATEST_PATH"
write_upgrade_script "$UPGRADE_PATH"

cat <<EOF
Installed safe-node ${VERSION} to:
  ${BIN_PATH}

Synced files:
  ${EXAMPLE_CONFIG_PATH}
  ${LATEST_PATH}
  ${UPGRADE_PATH}

Next steps:
  cd "${DEST_DIR}"
  ./safe-node --version
  cp config/example.node.json config/node.json
  ./safe-node keystore generate --out config/signer.json

Edit config/node.json with your leader, multisig, signer, and policy settings.
The config loader accepts JSON5 comments and trailing commas.
Then run:
  SAFE_NODE_KEYSTORE_PASSWORD=... ./safe-node run --config config/node.json
  ./safe-node tui

Upgrade later with:
  ./upgrade.sh
EOF
