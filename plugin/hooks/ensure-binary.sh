#!/bin/bash
set -euo pipefail

# ensure-binary.sh — SessionStart hook for stenographer
# Downloads rrecall binary from GitHub Releases if missing or version mismatch.

PLUGIN_ROOT="${CLAUDE_PLUGIN_ROOT}"
DATA_DIR="${CLAUDE_PLUGIN_DATA}"
BINARY="rrecall"
REPO="navistau/claude-stenographer"
INSTALLER_URL="https://raw.githubusercontent.com/navistau/claude-marketplace/main/install-gh-release.sh"

# Read plugin version
VERSION=$(python3 -c "import json; print(json.load(open('${PLUGIN_ROOT}/.claude-plugin/plugin.json'))['version'])")

# Check if binary exists and version matches
VERSION_FILE="${DATA_DIR}/.binary-version"
if [[ -f "${DATA_DIR}/bin/${BINARY}" ]] && [[ -f "$VERSION_FILE" ]] && [[ "$(cat "$VERSION_FILE")" == "$VERSION" ]]; then
  exit 0
fi

echo "Installing ${BINARY} v${VERSION}..."
mkdir -p "${DATA_DIR}/bin"

curl -fsSL "$INSTALLER_URL" | bash -s -- \
  --repo "$REPO" \
  --binary "$BINARY" \
  --version "$VERSION" \
  --dest "${DATA_DIR}/bin"

echo "$VERSION" > "$VERSION_FILE"
