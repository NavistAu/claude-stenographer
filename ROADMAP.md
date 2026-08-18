# Roadmap

Deferred work, in rough priority order. Phase 1 (lexical) and Phase 2 (dense
retrieval) shipped in v0.2.0 — see
`docs/superpowers/specs/2026-05-30-phase2-dense-effectiveness-findings.md`.

## Search-time reconcile insurance

The `SessionEnd` hook (`plugin/hooks/reindex.sh`) keeps the dense index fresh by
incrementally re-indexing after each session. That is the primary freshness
mechanism, but it has a gap: if the hook fails to fire or errors (disabled hooks,
a crash, a session that ends abnormally), the index silently goes stale and a
`dense`/`hybrid` search misses recent sessions with no signal to the user.

**Add a self-healing reconcile to the binary**: on a `dense`/`hybrid` search,
after returning results, spawn a detached, single-flighted incremental
`rrecall index` (same flock as the hook) so the index converges to complete even
if the hook never ran. It must:

- be **non-blocking** — the search returns immediately; the reconcile runs detached;
- be **single-flighted** — reuse the `${TMPDIR}/rrecall-reindex.lock` flock so a
  search during an in-progress reindex is a no-op, not a stampede;
- be **throttled** — skip if a reconcile started within the last few minutes
  (a lockfile-mtime check), so a burst of searches triggers at most one;
- **never recurse** — the spawned `index` subcommand must not itself reconcile.

This was intentionally deferred at v0.2.0 ship because the `SessionEnd` hook
already covers the common case; the reconcile is belt-and-suspenders.

## Build / index ergonomics

- **Manifest bootstrap is a one-time full rebuild.** The incremental index reuses
  sessions by file-signature manifest, but an index built before the manifest
  feature (or deleted) re-embeds everything once to seed the manifest (~2 h on the
  full corpus). Document this; consider a progress ETA.
- **Removed-file detection.** A session is reused if all its *current* files match
  the manifest; a *deleted* file in an otherwise-unchanged session isn't detected,
  so stale entries can linger. Add a file-set comparison (count/hash of the set).
- **Embedding throughput.** Cross-session batching (`EMBED_BATCH=256`) helps, but
  the initial build is still slow. Profile; consider larger batches / parallel
  embedding if `ort` supports it.

## Retrieval quality

- **Fusion weight is tuned on 4 probes.** `--dense-weight` defaults to 1.5 from a
  tiny probe set. Build a larger labelled probe set (target session + disjoint
  query pairs) and tune properly; consider per-query adaptive weighting.
- **Dense-only ceiling.** When lexical is silent, `hybrid` can't rank a target
  above its dense rank and adds both-list sessions on top, so `dense` alone is
  marginally better on pure vocabulary-mismatch. A re-ranker (cross-encoder) or a
  dense-confidence floor could close this.
- **ANN store.** The flat brute-force cosine store is fine at ~100 k chunks
  (query ~0.4 s). If the corpus grows enough to make brute-force slow, move to
  `sqlite-vec` / HNSW (the store interface is already isolated in `src/index.rs`).

## Smaller follow-ups

- Rich context for dense-only hits (currently a lean placeholder row — they have
  no lexical match to anchor a context window; use the matched chunk offset).
- Per-term IDF could be corpus-wide rather than over the candidate set.
