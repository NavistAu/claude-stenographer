#!/usr/bin/env bash
# reindex.sh — forward ingest. After a session ends, incrementally update the
# dense index so the just-finished session becomes searchable.
#
# Detached + non-blocking: returns immediately; the actual `rrecall index` runs
# in the background. Single-flight is handled INSIDE the binary (an advisory
# flock(2) on the lockfile fd), not here — macOS ships no `flock(1)`, so the old
# `flock -n 9` shell guard silently failed and the index never built. The build
# is incremental (unchanged sessions are reused; only new/changed ones embed).
set -euo pipefail

BIN="${CLAUDE_PLUGIN_DATA}/bin/rrecall"
[ -x "$BIN" ] || exit 0

# Detached background build; output discarded. If another build is already
# running, the binary's own lock makes this a no-op.
"$BIN" index --all-projects >/dev/null 2>&1 &

exit 0
