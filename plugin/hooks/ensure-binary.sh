#!/bin/bash
set -euo pipefail

# ensure-binary.sh — SessionStart hook for stenographer
# Installs/updates the rrecall binary using its per-release, checksum-verified
# installer (rrecall-installer.sh, published alongside each GitHub release).
# Floats to the highest available patch of plugin.json's major.minor, checked
# at most once/day. Never fails session start: any problem here just means
# rrecall is unavailable this session, retried next time.

PLUGIN_ROOT="${CLAUDE_PLUGIN_ROOT}"
DATA_DIR="${CLAUDE_PLUGIN_DATA}"
REPO="navistau/claude-stenographer"
BIN="${DATA_DIR}/bin/rrecall"
VERSION_FILE="${DATA_DIR}/.binary-version"
LAST_CHECK="${DATA_DIR}/.last-check"

warn() {
  echo "ensure-binary: $1 — stenographer binary unavailable this session (will retry next session)." >&2
}

mkdir -p "${DATA_DIR}/bin"

PLUGIN_VERSION=$(python3 -c "import json; print(json.load(open('${PLUGIN_ROOT}/.claude-plugin/plugin.json'))['version'])" 2>/dev/null) \
  || { warn "could not read version from plugin.json"; exit 0; }
MAJOR_MINOR="${PLUGIN_VERSION%.*}"
MAJOR_MINOR_RE=$(printf '%s' "$MAJOR_MINOR" | sed 's/\./\\./g')

RESOLVED="$PLUGIN_VERSION"
if [ -x "$BIN" ] && [ -f "$VERSION_FILE" ] && [ -f "$LAST_CHECK" ] \
  && [ -n "$(find "$LAST_CHECK" -mtime -1 2>/dev/null)" ]; then
  RESOLVED="$(cat "$VERSION_FILE")"
else
  LATEST=$(GIT_TERMINAL_PROMPT=0 timeout 10 git ls-remote --tags \
      "https://github.com/${REPO}.git" "v${MAJOR_MINOR}.*" 2>/dev/null \
    | grep -v '\^{}' \
    | sed -n "s#.*refs/tags/\\(v${MAJOR_MINOR_RE}\\.[0-9][0-9]*\\)\$#\\1#p" \
    | sort -V | tail -1) || LATEST=""
  [ -n "$LATEST" ] && RESOLVED="${LATEST#v}"
  touch "$LAST_CHECK" 2>/dev/null || true
fi

if [ -x "$BIN" ] && [ -f "$VERSION_FILE" ] && [ "$(cat "$VERSION_FILE")" = "$RESOLVED" ]; then
  exit 0
fi

TMP=$(mktemp) || { warn "could not create a temp file"; exit 0; }
trap 'rm -f "$TMP"' EXIT

INSTALLER_URL="https://github.com/${REPO}/releases/download/v${RESOLVED}/rrecall-installer.sh"
if ! curl -fsSL --proto '=https' --tlsv1.2 --connect-timeout 10 -m 30 "$INSTALLER_URL" -o "$TMP"; then
  warn "download of rrecall-installer.sh v${RESOLVED} failed"
  exit 0
fi

if command -v gh >/dev/null 2>&1; then
  ATTEST_OUT=$(gh attestation verify "$TMP" --repo "$REPO" 2>&1) && ATTEST_RC=0 || ATTEST_RC=$?
  if [ "$ATTEST_RC" -ne 0 ] && printf '%s' "$ATTEST_OUT" | grep -qi "does not match\|digest mismatch"; then
    warn "attestation verification reported rrecall-installer.sh v${RESOLVED} does not match its signed digest"
    exit 0
  fi
  # Any other outcome (no attestations published, rate-limited, gh not
  # authenticated, ...) is inconclusive, not a tamper signal — proceed.
fi

if ! RRECALL_INSTALL_DIR="${DATA_DIR}" RRECALL_NO_MODIFY_PATH=1 RRECALL_PRINT_QUIET=1 \
    bash "$TMP"; then
  warn "rrecall-installer.sh v${RESOLVED} failed"
  exit 0
fi

echo "$RESOLVED" > "$VERSION_FILE"
