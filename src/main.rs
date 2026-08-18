mod chunk;
mod embed;
mod escalate;
mod fusion;
mod index;
mod output;
mod parser;
mod query;
mod reindex;
mod scan;
mod scope;
mod search;

use clap::Parser;
use std::collections::HashMap;
use std::path::PathBuf;
use walkdir::WalkDir;

use escalate::{escalate, Scope, TierOutcome};
use query::CompiledQuery;

#[derive(Parser)]
#[command(
    name = "rrecall",
    version,
    about = "Search Claude Code conversation history"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    Search {
        query: String,
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long, default_value_t = false)]
        all_projects: bool,
        #[arg(long, default_value_t = 50)]
        recent_limit: usize,
        #[arg(long, default_value_t = 1)]
        target: usize,
        #[arg(long, default_value_t = 20)]
        max_results: usize,
        #[arg(long, default_value_t = 3)]
        context: usize,
        #[arg(long, env = "RRECALL_PROJECTS_DIR")]
        claude_dir: Option<PathBuf>,
        #[arg(long, default_value = "hybrid")]
        mode: String,
        #[arg(long)]
        index_dir: Option<PathBuf>,
        /// Hybrid fusion: weight on the dense signal relative to lexical (1.0).
        #[arg(long, default_value_t = 1.5)]
        dense_weight: f64,
        /// Output format: `text` (compact, human-readable — the default) or `json`.
        #[arg(long, default_value = "text")]
        format: String,
    },
    /// Build the dense embedding index over the corpus.
    Index {
        #[arg(long, default_value_t = false)]
        all_projects: bool,
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long, env = "RRECALL_PROJECTS_DIR")]
        claude_dir: Option<PathBuf>,
        #[arg(long)]
        index_dir: Option<PathBuf>,
    },
}

fn default_projects_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".claude")
        .join("projects")
}

fn default_index_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rrecall")
        .join("index")
}

/// Map session_id -> (files, project) across the given project dirs.
fn session_files(
    project_dirs: &[PathBuf],
) -> std::collections::HashMap<String, (Vec<PathBuf>, String)> {
    let mut groups: std::collections::HashMap<String, (Vec<PathBuf>, String)> =
        std::collections::HashMap::new();
    for dir in project_dirs {
        let proj = dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        for entry in WalkDir::new(dir).into_iter().flatten() {
            let p = entry.path();
            if p.extension().is_some_and(|e| e == "jsonl") {
                let key = parser::parent_session_id(p).unwrap_or_else(|| {
                    p.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                });
                let e = groups
                    .entry(key)
                    .or_insert_with(|| (Vec::new(), proj.clone()));
                e.0.push(p.to_path_buf());
            }
        }
    }
    groups
}

/// Chunks are embedded in batches of this size, accumulated ACROSS sessions, so
/// the model is utilized well instead of one tiny batch per session.
const EMBED_BATCH: usize = 256;

/// Checkpoint the partial index to disk this often during a build, so a killed
/// build resumes from the checkpoint instead of losing everything (the final
/// save alone meant a long catch-up that never ran to completion made no
/// progress at all).
const CHECKPOINT_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

/// Embed the buffered batch and append it to the index, clearing the buffers.
/// The embedder is initialized lazily so a fully-incremental run (everything
/// reused) never loads the model.
fn flush_batch(
    idx: &mut index::Index,
    embedder: &mut Option<embed::Embedder>,
    texts: &mut Vec<String>,
    meta: &mut Vec<index::EntryMeta>,
) -> Result<(), Box<dyn std::error::Error>> {
    if texts.is_empty() {
        return Ok(());
    }
    if embedder.is_none() {
        *embedder = Some(embed::Embedder::new()?);
    }
    let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
    let vecs = embedder.as_mut().unwrap().embed_batch(&refs)?;
    for v in &vecs {
        idx.vectors.extend_from_slice(v);
    }
    idx.meta.entries.append(meta);
    texts.clear();
    Ok(())
}

/// (sessions reused, sessions updated, chunks reused within updated sessions,
/// chunks embedded, total chunks).
type BuildIndexStats = (usize, usize, usize, usize, usize);

/// Build (or incrementally update) the dense index. Sessions whose files are
/// unchanged since the last build are reused verbatim. Within a changed
/// session, chunks whose text hash-matches a previously embedded chunk reuse
/// their old vector — an append to a live session only embeds the new tail
/// windows, not the whole transcript.
/// Returns (sessions reused, sessions updated, chunks reused within updated
/// sessions, chunks embedded, total chunks).
fn build_index(
    project_dirs: &[PathBuf],
    index_dir: &std::path::Path,
) -> Result<BuildIndexStats, Box<dyn std::error::Error>> {
    let old = index::Index::load(index_dir);
    let old_manifest = old
        .as_ref()
        .map(|o| o.meta.manifest.files.clone())
        .unwrap_or_default();
    let mut old_by_session = old.map(|o| o.by_session()).unwrap_or_default();

    let groups = session_files(project_dirs);
    let mut idx = index::Index {
        vectors: Vec::new(),
        meta: index::Meta::default(),
    };
    let mut embedder: Option<embed::Embedder> = None;
    let mut buf_texts: Vec<String> = Vec::new();
    let mut buf_meta: Vec<index::EntryMeta> = Vec::new();
    let (mut reused, mut embedded, mut done) = (0usize, 0usize, 0usize);
    let (mut chunks_reused, mut chunks_embedded) = (0usize, 0usize);
    let mut last_checkpoint = std::time::Instant::now();

    for (sid, (files, proj)) in &groups {
        // Reusable iff indexed before AND every file's signature is unchanged.
        let mut unchanged = old_by_session.contains_key(sid);
        for f in files {
            let key = f.to_string_lossy().to_string();
            let sig = index::file_sig(f);
            if old_manifest.get(&key) != Some(&sig) {
                unchanged = false;
            }
            idx.meta.manifest.files.insert(key, sig);
        }

        if unchanged {
            if let Some((entries, vecs)) = old_by_session.remove(sid) {
                idx.meta.entries.extend(entries);
                idx.vectors.extend(vecs);
                reused += 1;
            }
        } else {
            let mut messages = Vec::new();
            for f in files {
                let (m, _) = parser::parse_session(f);
                messages.extend(m);
            }
            let chunks = chunk::chunk_session(&messages);
            // Chunk-level reuse: map this session's previously embedded chunks
            // by text hash. An unchanged window keeps its old vector; only
            // new/changed windows go to the embed buffer. (Appending a direct
            // meta+vector pair while the buffer holds pending ones is safe:
            // every append adds entries and vectors in lockstep, so index i
            // always corresponds to vectors[i*DIM..].)
            let mut old_vecs: HashMap<u64, Vec<f32>> = HashMap::new();
            if let Some((entries, vecs)) = old_by_session.remove(sid) {
                for (i, e) in entries.iter().enumerate() {
                    if e.hash != 0 {
                        old_vecs
                            .entry(e.hash)
                            .or_insert_with(|| vecs[i * embed::DIM..(i + 1) * embed::DIM].to_vec());
                    }
                }
            }
            if !chunks.is_empty() {
                for c in &chunks {
                    let hash = index::chunk_hash(&c.text);
                    let meta = index::EntryMeta {
                        session_id: sid.clone(),
                        project: proj.clone(),
                        offset: c.offset,
                        timestamp: c.timestamp.clone(),
                        hash,
                    };
                    if let Some(v) = old_vecs.get(&hash) {
                        idx.vectors.extend_from_slice(v);
                        idx.meta.entries.push(meta);
                        chunks_reused += 1;
                    } else {
                        buf_texts.push(c.text.clone());
                        buf_meta.push(meta);
                        chunks_embedded += 1;
                    }
                }
                embedded += 1;
                if buf_texts.len() >= EMBED_BATCH {
                    flush_batch(&mut idx, &mut embedder, &mut buf_texts, &mut buf_meta)?;
                }
            }
        }
        done += 1;
        if done % 100 == 0 {
            eprintln!(
                "  {done}/{} sessions ({reused} reused, {embedded} (re)embedded, {} chunks)...",
                groups.len(),
                idx.meta.entries.len()
            );
        }
        if last_checkpoint.elapsed() >= CHECKPOINT_EVERY {
            flush_batch(&mut idx, &mut embedder, &mut buf_texts, &mut buf_meta)?;
            // Best effort: a failed checkpoint costs resumability, not the build.
            if let Err(e) = idx.save_with_leftovers(&old_by_session, &old_manifest, index_dir) {
                eprintln!("warning: checkpoint failed: {e}");
            }
            last_checkpoint = std::time::Instant::now();
        }
    }
    flush_batch(&mut idx, &mut embedder, &mut buf_texts, &mut buf_meta)?;
    idx.save(index_dir)?;
    Ok((
        reused,
        embedded,
        chunks_reused,
        chunks_embedded,
        idx.meta.entries.len(),
    ))
}

fn ts_in_range(ts: &str, after: &Option<String>, before: &Option<String>) -> bool {
    let date = ts.get(0..10).unwrap_or("");
    if let Some(a) = after {
        if date < a.as_str() {
            return false;
        }
    }
    if let Some(b) = before {
        if date > b.as_str() {
            return false;
        }
    }
    true
}

/// Max context windows returned per session, so `max_results` spans many
/// distinct sessions instead of being saturated by one chatty session.
const PER_SESSION_WINDOW_CAP: usize = 2;

/// Hybrid fusion: lexical weight (the dense weight is the `--dense-weight` flag,
/// default 1.5 — dense bridges vocabulary mismatch; lexical handles precision).
const HYBRID_W_LEXICAL: f64 = 1.0;

struct Searcher {
    working_dir: PathBuf,
    projects_dir: PathBuf,
    all_projects_flag: bool,
    recent_limit: usize,
    max_results: usize,
    context: usize,
    total_malformed: std::cell::Cell<usize>,
}

impl Searcher {
    fn run_tier(&self, scope: &Scope, q: &CompiledQuery) -> TierOutcome<output::SessionResult> {
        let all_projects = self.all_projects_flag || *scope == Scope::AllProjects;
        let mut project_dirs =
            scope::resolve_project_dirs(&self.working_dir, &self.projects_dir, all_projects);
        if !all_projects && *scope == Scope::AncestorProjects {
            project_dirs.extend(scope::resolve_ancestor_dirs(
                &self.working_dir,
                &self.projects_dir,
            ));
        }

        let candidate_files: Vec<(PathBuf, String)> =
            match scan::rg_candidates(&q.positive_phrases, &project_dirs) {
                Some(files) => {
                    let unique: std::collections::BTreeSet<PathBuf> =
                        files.into_iter().filter(|p| scan::is_jsonl(p)).collect();
                    unique
                        .into_iter()
                        .map(|p| {
                            let proj = project_for(&p, &project_dirs);
                            (p, proj)
                        })
                        .collect()
                }
                None => {
                    let mut v = Vec::new();
                    for dir in &project_dirs {
                        let dir_name = dir
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();
                        for entry in WalkDir::new(dir).into_iter().flatten() {
                            if scan::is_jsonl(entry.path()) {
                                v.push((entry.path().to_path_buf(), dir_name.clone()));
                            }
                        }
                    }
                    v
                }
            };

        let mut groups: HashMap<String, Vec<(PathBuf, String)>> = HashMap::new();
        let mut mtimes: HashMap<String, std::time::SystemTime> = HashMap::new();
        for (path, proj) in &candidate_files {
            if let Some(pf) = &q.filters.project {
                if !proj.contains(pf.as_str()) {
                    continue;
                }
            }
            let key = parser::parent_session_id(path).unwrap_or_else(|| {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            });
            let mtime = std::fs::metadata(path)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            groups
                .entry(key.clone())
                .or_default()
                .push((path.clone(), proj.clone()));
            let e = mtimes.entry(key).or_insert(mtime);
            if mtime > *e {
                *e = mtime;
            }
        }

        let mut keys: Vec<String> = groups.keys().cloned().collect();
        keys.sort_by(|a, b| {
            let ma = mtimes
                .get(a)
                .copied()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let mb = mtimes
                .get(b)
                .copied()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            mb.cmp(&ma)
        });
        if *scope == Scope::CurrentProjectRecent {
            keys.truncate(self.recent_limit);
        }

        // Coverage count for the tier actually reached (post-truncation), surfaced to the agent.
        let sessions_searched = keys.len();

        // Evaluate every candidate session, then RANK BY RELEVANCE using
        // inverse-document-frequency-weighted term matching: each matched query
        // term contributes idf = ln((N+1)/(df+1)) + 1, where df = how many
        // candidate sessions also matched it. Rare/distinctive terms dominate;
        // common terms add little — removing the session-length bias of a plain
        // "distinct terms matched" count (a long session no longer ranks high
        // just for incidentally containing many common words). Recency breaks
        // ties; windows per session are still capped for diversity.
        let n_terms = q.positive_phrases.len();
        struct Scored {
            matched_terms: Vec<usize>,
            mtime: std::time::SystemTime,
            windows: Vec<output::SessionResult>,
        }
        let mut scored: Vec<Scored> = Vec::new();
        let mut df = vec![0usize; n_terms];
        for key in &keys {
            let files = match groups.get(key) {
                Some(f) => f,
                None => continue,
            };
            let mtime = mtimes
                .get(key)
                .copied()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let mut windows = Vec::new();
            let mut session_added = 0usize;
            let mut term_seen = vec![false; n_terms];
            for (path, proj) in files {
                let (messages, malformed) = parser::parse_session(path);
                self.total_malformed
                    .set(self.total_malformed.get() + malformed);
                let eval = search::evaluate(&messages, q);
                if eval.matches.is_empty() {
                    continue;
                }
                let first_ts = messages
                    .get(eval.matches[0].message_index)
                    .map(|m| m.timestamp.clone())
                    .unwrap_or_default();
                if !ts_in_range(&first_ts, &q.filters.after, &q.filters.before) {
                    continue;
                }
                for &t in &eval.matched_terms {
                    if t < n_terms {
                        term_seen[t] = true;
                    }
                }
                // Cap windows per SESSION (across all its files, incl. subagent logs).
                let per_session = PER_SESSION_WINDOW_CAP.saturating_sub(session_added);
                if per_session == 0 {
                    continue;
                }
                let mut sr = output::build_session_result(
                    &messages,
                    &eval.matches,
                    self.context,
                    proj,
                    per_session,
                );
                session_added += sr.len();
                windows.append(&mut sr);
            }
            if !windows.is_empty() {
                let matched_terms: Vec<usize> = (0..n_terms).filter(|&i| term_seen[i]).collect();
                for &t in &matched_terms {
                    df[t] += 1;
                }
                scored.push(Scored {
                    matched_terms,
                    mtime,
                    windows,
                });
            }
        }
        let n_docs = scored.len() as f64;
        let idf: Vec<f64> = df
            .iter()
            .map(|&d| ((n_docs + 1.0) / (d as f64 + 1.0)).ln() + 1.0)
            .collect();
        let term_score = |terms: &[usize]| -> f64 {
            terms
                .iter()
                .filter(|&&t| t < n_terms)
                .map(|&t| idf[t])
                .sum()
        };
        scored.sort_by(|a, b| {
            term_score(&b.matched_terms)
                .partial_cmp(&term_score(&a.matched_terms))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.mtime.cmp(&a.mtime))
        });
        let results: Vec<output::SessionResult> = scored
            .into_iter()
            .flat_map(|s| s.windows)
            .take(self.max_results)
            .collect();
        TierOutcome {
            results,
            sessions_searched,
        }
    }
}

fn project_for(path: &std::path::Path, project_dirs: &[PathBuf]) -> String {
    for d in project_dirs {
        if path.starts_with(d) {
            return d
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
        }
    }
    path.parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Search {
            query,
            project_dir,
            all_projects,
            recent_limit,
            target,
            max_results,
            context,
            claude_dir,
            mode,
            index_dir,
            dense_weight,
            format,
        } => {
            let as_json = format.eq_ignore_ascii_case("json");
            let render = |out: &output::SearchOutput| {
                if as_json {
                    out.to_json()
                } else {
                    out.to_text()
                }
            };
            let projects_dir = claude_dir.unwrap_or_else(default_projects_dir);
            if !projects_dir.exists() {
                let out =
                    output::SearchOutput::error(&query, "Claude projects directory not found");
                println!("{}", render(&out));
                std::process::exit(1);
            }
            let working_dir = if project_dir == std::path::Path::new(".") {
                std::env::current_dir().unwrap_or(project_dir)
            } else {
                project_dir.canonicalize().unwrap_or(project_dir)
            };

            let compiled = query::compile(&query);
            let searcher = Searcher {
                working_dir,
                projects_dir,
                all_projects_flag: all_projects,
                recent_limit,
                max_results,
                context,
                total_malformed: std::cell::Cell::new(0),
            };

            let escalated = if all_projects {
                // Explicit --all-projects: skip the ladder, search everything once,
                // and report the scope honestly.
                let outcome = searcher.run_tier(&Scope::AllProjects, &compiled);
                escalate::Escalated {
                    results: outcome.results,
                    scope_reached: Scope::AllProjects,
                    sessions_searched: outcome.sessions_searched,
                }
            } else {
                escalate(target, |scope| searcher.run_tier(scope, &compiled))
            };

            if searcher.total_malformed.get() > 0 {
                eprintln!(
                    "warning: skipped {} malformed lines",
                    searcher.total_malformed.get()
                );
            }

            // Capture before partial move of escalated.
            let sessions_searched = escalated.sessions_searched;
            let scope_label = escalated.scope_reached.label().to_string();
            let scoped_to_project = !all_projects && escalated.scope_reached != Scope::AllProjects;
            let include_ancestors = escalated.scope_reached == Scope::AncestorProjects;

            let mode_l = mode.to_lowercase();
            let lexical_results = escalated.results;
            let mut lexical_ids: Vec<String> = Vec::new();
            {
                let mut seen = std::collections::HashSet::new();
                for r in &lexical_results {
                    if seen.insert(r.session_id.clone()) {
                        lexical_ids.push(r.session_id.clone());
                    }
                }
            }

            let idx_dir = index_dir.unwrap_or_else(default_index_dir);
            let results: Vec<output::SessionResult> = if mode_l == "lexical" {
                lexical_results
            } else {
                match index::Index::load(&idx_dir) {
                    None => {
                        eprintln!("warning: no index at {idx_dir:?}; falling back to lexical mode");
                        lexical_results
                    }
                    Some(idx) => match embed::Embedder::new() {
                        Err(e) => {
                            eprintln!("warning: embedder init failed ({e}); lexical fallback");
                            lexical_results
                        }
                        Ok(mut embedder) => {
                            let qvec = embedder.embed_one(&query).unwrap_or_default();
                            // id -> (project, timestamp) from index meta, for scope
                            // filtering and lean dense-only results.
                            let mut meta_map: std::collections::HashMap<&str, (String, String)> =
                                std::collections::HashMap::new();
                            for e in &idx.meta.entries {
                                meta_map
                                    .entry(e.session_id.as_str())
                                    .or_insert((e.project.clone(), e.timestamp.clone()));
                            }
                            // idx.search ranks the ENTIRE corpus — it has no scope
                            // notion — while scope_reached reports the lexical
                            // ladder's tier. Unfiltered, dense-only hits from foreign
                            // projects leak into a current_project result set. Keep
                            // only sessions in the reported scope (same rule as
                            // resolve_project_dirs: the project itself or one of its
                            // subdirectory projects; at the ancestor tier, also exact
                            // path-ancestor projects). Tier 1's recency cap is
                            // deliberately not applied to dense: it searches the
                            // project's whole history at every project tier.
                            let dense_ranked: Vec<(String, f32, usize)> = if scoped_to_project {
                                let enc = scope::encode_path(&searcher.working_dir);
                                let ancestors: std::collections::HashSet<String> =
                                    if include_ancestors {
                                        scope::ancestor_names(&searcher.working_dir)
                                            .into_iter()
                                            .collect()
                                    } else {
                                        std::collections::HashSet::new()
                                    };
                                idx.search(&qvec)
                                    .into_iter()
                                    .filter(|(id, _, _)| {
                                        meta_map.get(id.as_str()).is_some_and(|(proj, _)| {
                                            scope::project_in_scope(proj, &enc)
                                                || ancestors.contains(proj)
                                        })
                                    })
                                    .collect()
                            } else {
                                idx.search(&qvec)
                            };
                            let dense_ids: Vec<String> =
                                dense_ranked.iter().map(|(id, _, _)| id.clone()).collect();
                            let final_ids = if mode_l == "dense" {
                                dense_ids
                            } else {
                                // Score-aware fusion: lexical contributes reciprocal-rank
                                // (no comparable scalar score), dense contributes its real
                                // cosine — so a strong dense-only rescue is not buried.
                                let lex_scored: Vec<(String, f64)> = lexical_ids
                                    .iter()
                                    .enumerate()
                                    .map(|(i, id)| (id.clone(), 1.0 / (1.0 + i as f64)))
                                    .collect();
                                let dense_scored: Vec<(String, f64)> = dense_ranked
                                    .iter()
                                    .map(|(id, s, _)| (id.clone(), *s as f64))
                                    .collect();
                                fusion::weighted_fuse(&[
                                    (lex_scored, HYBRID_W_LEXICAL),
                                    (dense_scored, dense_weight),
                                ])
                            };
                            // id -> lexical SessionResults (rich) for reuse
                            let mut lex_map: std::collections::HashMap<
                                String,
                                Vec<output::SessionResult>,
                            > = std::collections::HashMap::new();
                            for r in lexical_results {
                                lex_map.entry(r.session_id.clone()).or_default().push(r);
                            }
                            let mut out_results: Vec<output::SessionResult> = Vec::new();
                            for id in &final_ids {
                                if out_results.len() >= max_results {
                                    break;
                                }
                                if let Some(mut rich) = lex_map.remove(id) {
                                    out_results.append(&mut rich);
                                } else if let Some((proj, ts)) = meta_map.get(id.as_str()) {
                                    // lean dense-only hit (semantic match, no lexical term)
                                    out_results.push(output::SessionResult {
                                        session_id: id.clone(),
                                        project_dir: proj.clone(),
                                        first_match_timestamp: ts.clone(),
                                        match_context: vec![output::ContextMessage {
                                            role: "system".into(),
                                            text: "[semantic match — no lexical term; run --mode lexical or open the session for context]".into(),
                                            timestamp: ts.clone(),
                                            matched: true,
                                        }],
                                    });
                                }
                            }
                            out_results
                        }
                    },
                }
            };

            let out = output::SearchOutput {
                query,
                sessions_searched,
                scope_reached: scope_label,
                hit_count: results.len(),
                mode: mode_l.clone(),
                results,
                error: None,
            };
            println!("{}", render(&out));

            // Search-time reconcile: in dense/hybrid mode, kick a detached
            // incremental index build so the dense index converges to complete
            // even if the SessionEnd hook never fired. No-op in lexical mode.
            if mode_l != "lexical" {
                reindex::spawn_reconcile(&idx_dir);
            }
        }
        Commands::Index {
            all_projects,
            project_dir,
            claude_dir,
            index_dir,
        } => {
            // Index builds are background maintenance (hook- or search-spawned):
            // run at background priority so embedding never starves the machine.
            reindex::background_priority();
            let index_dir = index_dir.unwrap_or_else(default_index_dir);
            // Single-flight: bail if another build (hook- or search-spawned) holds
            // the lock, rather than stampeding two concurrent embeds over one index.
            let _build_lock = match reindex::try_build_lock(&index_dir) {
                Some(l) => l,
                None => {
                    eprintln!("index: another build is in progress; skipping");
                    return;
                }
            };
            let projects_dir = claude_dir.unwrap_or_else(default_projects_dir);
            let working_dir = if project_dir == std::path::Path::new(".") {
                std::env::current_dir().unwrap_or(project_dir)
            } else {
                project_dir.canonicalize().unwrap_or(project_dir)
            };
            let project_dirs =
                scope::resolve_project_dirs(&working_dir, &projects_dir, all_projects);
            match build_index(&project_dirs, &index_dir) {
                Ok((reused, updated, cr, ce, chunks)) =>
                    eprintln!("index: {reused} sessions reused, {updated} updated ({cr} chunks reused, {ce} embedded), {chunks} chunks total -> {index_dir:?}"),
                Err(e) => { eprintln!("index build failed: {e}"); std::process::exit(1); }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ts_in_range;

    #[test]
    fn test_ts_in_range_within() {
        assert!(ts_in_range(
            "2026-05-18T10:00:00Z",
            &Some("2026-05-17".into()),
            &Some("2026-05-19".into())
        ));
    }
    #[test]
    fn test_ts_in_range_out_of_window() {
        assert!(!ts_in_range(
            "2026-05-20T10:00:00Z",
            &None,
            &Some("2026-05-19".into())
        ));
        assert!(!ts_in_range(
            "2026-05-10T10:00:00Z",
            &Some("2026-05-17".into()),
            &None
        ));
    }
    #[test]
    fn test_ts_in_range_no_filters() {
        assert!(ts_in_range("2026-05-18T10:00:00Z", &None, &None));
    }
    #[test]
    fn test_ts_in_range_empty_ts_excluded_when_after_set() {
        // An undated message can't be confirmed in-range, so an `after` filter
        // excludes it (documented behaviour).
        assert!(!ts_in_range("", &Some("2026-05-01".into()), &None));
        // With no `after`, an empty ts is not excluded by `before`.
        assert!(ts_in_range("", &None, &Some("2026-05-19".into())));
    }
}
