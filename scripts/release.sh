#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT_DIR"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

confirm() {
  local prompt="$1"
  local answer

  read -r -p "$prompt [y/N] " answer
  [[ "$answer" == "y" || "$answer" == "Y" || "$answer" == "yes" || "$answer" == "YES" ]]
}

manifest_version() {
  sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1
}

update_manifest_version() {
  local version="$1"

  NEW_VERSION="$version" perl -0pi -e '
    s/(\[package\]\s+name = "safe-node"\s+version = ")[^"]+(")/$1$ENV{NEW_VERSION}$2/s
  ' Cargo.toml
}

require_cmd cargo
require_cmd git
require_cmd perl
require_cmd sed

if [[ "$(git branch --show-current)" != "main" ]]; then
  echo "Release must be created from the main branch." >&2
  exit 1
fi

if [[ -n "$(git status --porcelain --untracked-files=all)" ]]; then
  echo "Git working tree must be clean before releasing." >&2
  git status --short
  exit 1
fi

current_version="$(manifest_version)"
if [[ ! "$current_version" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
  echo "Cargo.toml version must use MAJOR.MINOR.PATCH, got: $current_version" >&2
  exit 1
fi

major="${BASH_REMATCH[1]}"
minor="${BASH_REMATCH[2]}"
patch="${BASH_REMATCH[3]}"

cat <<EOF
Current version: ${current_version}

Version bump:
  major - incompatible release (${major}.${minor}.${patch} -> $((major + 1)).0.0)
  minor - compatible feature release (${major}.${minor}.${patch} -> ${major}.$((minor + 1)).0)
  patch - compatible bugfix release (${major}.${minor}.${patch} -> ${major}.${minor}.$((patch + 1)))
EOF

read -r -p "Select bump [major/minor/patch] (patch): " bump
bump="${bump:-patch}"

case "$bump" in
  major)
    new_version="$((major + 1)).0.0"
    ;;
  minor)
    new_version="${major}.$((minor + 1)).0"
    ;;
  patch)
    new_version="${major}.${minor}.$((patch + 1))"
    ;;
  *)
    echo "Unknown bump type: $bump" >&2
    exit 1
    ;;
esac

tag="v${new_version}"

if git rev-parse --verify --quiet "refs/tags/${tag}" >/dev/null; then
  echo "Tag already exists: ${tag}" >&2
  exit 1
fi

cat <<EOF

Release summary:
  current version: ${current_version}
  new version:     ${new_version}
  tag:             ${tag}

This will update Cargo.toml and Cargo.lock, run tests, commit, and create an annotated tag.
EOF

if ! confirm "Continue"; then
  echo "Release cancelled."
  exit 0
fi

update_manifest_version "$new_version"
cargo metadata --format-version 1 >/dev/null
cargo test --locked

git add Cargo.toml Cargo.lock
git commit -m "chore: release ${tag}"
git tag -a "$tag" -m "safe-node ${tag}"

cat <<EOF

Created release commit and tag locally:
  commit: $(git rev-parse --short HEAD)
  tag:    ${tag}
EOF

if confirm "Push commit and tag to origin"; then
  git push origin main
  git push origin "$tag"
  echo "Pushed main and ${tag}. GitHub Actions will create the release."
else
  cat <<EOF
Push skipped. Run these commands when ready:
  git push origin main
  git push origin ${tag}
EOF
fi
