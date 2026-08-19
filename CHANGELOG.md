# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Any change to what gets indexed, or to how search results are scoped (the escalation
ladder, project-boundary resolution, dense/lexical fusion), is called out here
regardless of size — those are the changes most likely to silently affect what a
query can and can't find.

## [0.4.0] - 2026-08-19

First public release.

Supported platforms: darwin arm64, linux x64/arm64. No Intel-mac or Windows
binaries.

### Added

- `rrecall search`: lexical search over Claude Code transcripts using Lucene/
  Elasticsearch query syntax (`AND`/`OR`/`NOT`, grouping, exact phrases), with
  field filters `role:`, `project:`, `after:`, `before:`.
- `rrecall search --mode dense`: local-embedding semantic search
  (`all-MiniLM-L6-v2`, no network at query time), for queries that share no
  words with the target session.
- `rrecall search --mode hybrid` (default): score-aware fusion of lexical and
  dense results.
- Escalation ladder: search widens scope through `current_project_recent` ->
  `current_project_all` -> `ancestor_projects` -> `all_projects`, stopping once
  `--target` results are found; `--all-projects` bypasses it.
- `rrecall index`: incremental dense-index builder, reusing unchanged sessions
  and chunks by content signature; checkpoints during long builds so a killed
  build resumes instead of losing progress.
- `SessionEnd` hook that incrementally reindexes after every session, plus a
  search-time reconcile as a backstop if the hook doesn't fire.
- `stenographer` subagent: takes a natural-language question, searches via
  `rrecall`, and synthesizes a cited answer, so raw transcript excerpts never
  reach the caller's context. A `PreToolUse(Bash)` hook confines the agent to
  `rrecall` itself.
- `text` (default, human-readable) and `json` (full structured) output formats.
- `ensure-binary.sh`: installs `rrecall` from each release's own checksum-verified
  installer script, floating to the highest available patch of `plugin.json`'s
  major.minor (`git ls-remote --tags`, checked at most once/day) rather than
  pinning the exact version, with an optional `gh attestation verify` tier.
