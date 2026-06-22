#!/usr/bin/env bash
set -Eeuo pipefail

REPO="${SAFE_NODE_REPO:-hypesafe-io/safe-node}"
TAG="${SAFE_NODE_TAG:-}"
DEST_DIR="${SAFE_NODE_INSTALL_DIR:-${SAFE_NODE_DEST_DIR:-$PWD}}"
API_BASE="${GITHUB_API_URL:-https://api.github.com}"
GITHUB_BASE="${GITHUB_SERVER_URL:-https://github.com}"
TARGET="x86_64-unknown-linux-gnu"

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

require_cmd curl
require_cmd sed
require_cmd tar

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
ARCHIVE="${ASSET_NAME}.tar.gz"
CHECKSUM="${ARCHIVE}.sha256"
ARCHIVE_URL="${GITHUB_BASE}/${REPO}/releases/download/${TAG}/${ARCHIVE}"
CHECKSUM_URL="${GITHUB_BASE}/${REPO}/releases/download/${TAG}/${CHECKSUM}"

mkdir -p "$DEST_DIR"
DEST_DIR="$(cd -- "$DEST_DIR" && pwd)"
INSTALL_DIR="${DEST_DIR}/${ASSET_NAME}"

if [[ -e "$INSTALL_DIR" ]]; then
  echo "Destination already exists: $INSTALL_DIR" >&2
  echo "Remove it or set SAFE_NODE_INSTALL_DIR to another directory." >&2
  exit 1
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "Installing safe-node ${TAG} from ${REPO}"
curl_get "$ARCHIVE_URL" "${TMP_DIR}/${ARCHIVE}"
curl_get "$CHECKSUM_URL" "${TMP_DIR}/${CHECKSUM}"

if command -v sha256sum >/dev/null 2>&1; then
  (
    cd "$TMP_DIR"
    sha256sum -c "$CHECKSUM"
  )
else
  echo "sha256sum is not available; skipping checksum verification." >&2
fi

tar -xzf "${TMP_DIR}/${ARCHIVE}" -C "$DEST_DIR"

cat <<EOF
Installed safe-node ${VERSION} to:
  ${INSTALL_DIR}

Next steps:
  cd "${INSTALL_DIR}"
  ./safe-node --version
  cp config/example.node.json config/node.json
  ./safe-node keystore generate --out config/signer.json

Edit config/node.json with your leader, multisig, signer, and policy settings.
Then run:
  SAFE_NODE_KEYSTORE_PASSWORD=... ./safe-node run --config config/node.json
  ./safe-node tui
EOF
