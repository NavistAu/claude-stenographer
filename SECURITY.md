# Security Policy

## Reporting a vulnerability

Report privately via GitHub Security Advisories:
`https://github.com/navistau/claude-stenographer/security/advisories/new`

Do not open a public issue for a suspected vulnerability.

## Supported versions

Only the latest release is supported. Please upgrade before reporting, if possible.

## In scope

- The `rrecall` binary (search, index, and CLI argument handling).
- The plugin hooks, in particular `plugin/hooks/ensure-binary.sh`'s binary-download path (checksum/version verification, where it fetches from, how it writes into `${CLAUDE_PLUGIN_DATA}`).
- The search agent's tool restriction, `plugin/hooks/restrict-to-rrecall.sh` (the `PreToolUse(Bash)` hook that's supposed to confine the agent to `rrecall` and a small pipe allowlist).

## Out of scope

- Claude Code itself — report those to Anthropic, not here.
- Vulnerabilities in dependencies — report upstream to the dependency's maintainers. A heads-up here is still welcome so this project can track and update, but the fix belongs upstream.
