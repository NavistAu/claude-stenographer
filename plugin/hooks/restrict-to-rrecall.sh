#!/usr/bin/env bash
# restrict-to-rrecall.sh — PreToolUse(Bash) hook for the stenographer agent.
#
# Mechanically confines the agent to the rrecall search tool. The prompt tells
# it "only rrecall", but text alone doesn't reliably hold: in testing the agent
# hit a search miss, ran `git log` as a fallback, and confabulated a reason
# ("predates the corpus") rather than refining its query. This hook makes that
# impossible — every Bash command must invoke rrecall, and chaining to an
# alternate data source (git/curl/ssh/find/...) is blocked.
#
# Protocol: read the PreToolUse JSON on stdin; exit 0 to allow, exit 2 to block
# (stderr is shown to the agent). Fails OPEN on any parsing problem.
set -euo pipefail

input="$(cat)"
cmd="$(printf '%s' "$input" | jq -r '.tool_input.command // empty' 2>/dev/null || true)"

# No command to inspect → don't get in the way.
[ -z "$cmd" ] && exit 0

# Core rule: the stenographer's ONLY data source is rrecall. Require it.
if ! printf '%s' "$cmd" | grep -q 'rrecall'; then
  echo "BLOCKED: the stenographer may only use the rrecall search tool — not git, grep, cat, or any other source. Refine your rrecall query instead (pin a rare AND-term, change vocabulary, or add an after:/before: date range)." >&2
  exit 2
fi

# Even with rrecall present, forbid chaining to an alternate source/script.
# (Piping rrecall output to jq/head/tail/sort/uniq/wc/grep is fine.)
if printf '%s' "$cmd" | grep -Eq '(^|[;&|])[[:space:]]*(git|curl|wget|ssh|scp|python3?|node|fd|find)([[:space:]]|$)'; then
  echo "BLOCKED: drop the git/find/network/script command — only rrecall (optionally piped to jq/head/tail/sort/uniq/wc) is permitted. Refine your rrecall query instead." >&2
  exit 2
fi

exit 0
