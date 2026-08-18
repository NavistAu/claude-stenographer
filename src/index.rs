use crate::embed::{cosine, DIM};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone)]
pub struct EntryMeta {
    pub session_id: String,
    pub project: String,
    pub offset: usize,
    pub timestamp: String,
    /// FNV-1a of the chunk text, for chunk-level vector reuse across builds.
    /// 0 = unknown (entry written by an older binary): never matches, so the
    /// chunk re-embeds once and gains a hash.
    #[serde(default)]
    pub hash: u64,
}

/// Stable FNV-1a over chunk text. Deterministic across runs and versions
/// (unlike std's DefaultHasher). Remapped so 0 stays reserved for "no hash".
pub fn chunk_hash(text: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    if h == 0 {
        1
    } else {
        h
    }
}

#[derive(Serialize, Deserialize, Default)]
pub struct Manifest {
    pub files: HashMap<String, (u64, u64)>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Meta {
    pub entries: Vec<EntryMeta>,
    pub manifest: Manifest,
}

pub struct Index {
    pub vectors: Vec<f32>,
    pub meta: Meta,
}

impl Index {
    pub fn vectors_path(dir: &Path) -> PathBuf {
        dir.join("vectors.bin")
    }
    pub fn meta_path(dir: &Path) -> PathBuf {
        dir.join("meta.json")
    }

    pub fn load(dir: &Path) -> Option<Index> {
        let raw = std::fs::read(Self::vectors_path(dir)).ok()?;
        let meta: Meta = serde_json::from_slice(&std::fs::read(Self::meta_path(dir)).ok()?).ok()?;
        let vectors = raw
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect::<Vec<f32>>();
        // A reader can catch the files between a writer's two renames; a
        // misaligned pair would index out of bounds in search. Treat it as
        // no index (lexical fallback / full rebuild) instead.
        if vectors.len() != meta.entries.len() * DIM {
            return None;
        }
        Some(Index { vectors, meta })
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        self.save_with_leftovers(&HashMap::new(), &HashMap::new(), dir)
    }

    /// Save the index plus `leftovers` — sessions from the previous build the
    /// current build has not reached yet (as grouped by [`by_session`]) — and
    /// their old manifest signatures. A mid-build checkpoint saved this way
    /// stays complete: killed and restarted, the build still has every
    /// unprocessed session's vectors and hashes to reuse. Files go via
    /// tmp + rename so a concurrent reader never sees a torn file.
    pub fn save_with_leftovers(
        &self,
        leftovers: &HashMap<String, (Vec<EntryMeta>, Vec<f32>)>,
        old_manifest: &HashMap<String, (u64, u64)>,
        dir: &Path,
    ) -> std::io::Result<()> {
        use std::io::Write;
        std::fs::create_dir_all(dir)?;
        let tmp_vectors = dir.join("vectors.bin.tmp");
        let tmp_meta = dir.join("meta.json.tmp");

        let mut entries = self.meta.entries.clone();
        {
            let mut w = std::io::BufWriter::new(std::fs::File::create(&tmp_vectors)?);
            for v in &self.vectors {
                w.write_all(&v.to_le_bytes())?;
            }
            // Single pass so entry order and vector order stay in lockstep.
            for (e, vecs) in leftovers.values() {
                entries.extend(e.iter().cloned());
                for v in vecs {
                    w.write_all(&v.to_le_bytes())?;
                }
            }
            w.flush()?;
        }

        // Old signatures first, processed files overwrite: unprocessed
        // sessions keep exactly the staleness check the next build needs.
        let mut files = old_manifest.clone();
        files.extend(
            self.meta
                .manifest
                .files
                .iter()
                .map(|(k, v)| (k.clone(), *v)),
        );

        let meta = Meta {
            entries,
            manifest: Manifest { files },
        };
        std::fs::write(&tmp_meta, serde_json::to_vec(&meta).unwrap())?;
        std::fs::rename(&tmp_vectors, Self::vectors_path(dir))?;
        std::fs::rename(&tmp_meta, Self::meta_path(dir))?;
        Ok(())
    }

    /// Best cosine score per session for a query vector, ranked desc.
    /// Returns (session_id, best_score, offset_of_best_chunk).
    pub fn search(&self, query: &[f32]) -> Vec<(String, f32, usize)> {
        let n = self.meta.entries.len();
        let mut best: std::collections::HashMap<&str, (f32, usize)> =
            std::collections::HashMap::new();
        for i in 0..n {
            let v = &self.vectors[i * DIM..(i + 1) * DIM];
            let s = cosine(query, v);
            let e = best
                .entry(self.meta.entries[i].session_id.as_str())
                .or_insert((f32::MIN, 0));
            if s > e.0 {
                *e = (s, self.meta.entries[i].offset);
            }
        }
        let mut out: Vec<(String, f32, usize)> = best
            .into_iter()
            .map(|(k, (s, o))| (k.to_string(), s, o))
            .collect();
        out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        out
    }

    /// Group entries + their vectors by session_id, for incremental reuse of
    /// unchanged sessions on a rebuild.
    pub fn by_session(&self) -> HashMap<String, (Vec<EntryMeta>, Vec<f32>)> {
        let mut map: HashMap<String, (Vec<EntryMeta>, Vec<f32>)> = HashMap::new();
        for (i, e) in self.meta.entries.iter().enumerate() {
            let slot = map.entry(e.session_id.clone()).or_default();
            slot.0.push(e.clone());
            slot.1
                .extend_from_slice(&self.vectors[i * DIM..(i + 1) * DIM]);
        }
        map
    }
}

/// File signature for the manifest: (mtime_secs, len). A missing/unreadable
/// file yields (0, 0), which will never match a real file's signature.
pub fn file_sig(path: &Path) -> (u64, u64) {
    match std::fs::metadata(path) {
        Ok(m) => {
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            (mtime, m.len())
        }
        Err(_) => (0, 0),
    }
}

/// Embed and append a session's chunks. Test-only helper — the binary's
/// `build_index` batches across sessions instead of per-session.
#[cfg(test)]
fn append_session<F>(
    idx: &mut Index,
    chunks: &[crate::chunk::Chunk],
    session_id: &str,
    project: &str,
    mut embed: F,
) where
    F: FnMut(&[&str]) -> Vec<Vec<f32>>,
{
    if chunks.is_empty() {
        return;
    }
    let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
    let vecs = embed(&texts);
    for (c, v) in chunks.iter().zip(vecs) {
        idx.vectors.extend_from_slice(&v);
        idx.meta.entries.push(EntryMeta {
            session_id: session_id.into(),
            project: project.into(),
            offset: c.offset,
            timestamp: c.timestamp.clone(),
            hash: chunk_hash(&c.text),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Chunk;
    fn unit(seed: f32) -> Vec<f32> {
        let mut v = vec![0.0; DIM];
        v[0] = seed;
        v
    }
    #[test]
    fn test_append_and_search_ranks_nearest_session() {
        let mut idx = Index {
            vectors: vec![],
            meta: Meta::default(),
        };
        let ch = vec![Chunk {
            text: "a".into(),
            offset: 0,
            timestamp: "t".into(),
        }];
        append_session(&mut idx, &ch, "sess-near", "p", |_| vec![unit(1.0)]);
        append_session(&mut idx, &ch, "sess-far", "p", |_| {
            vec![{
                let mut v = vec![0.0; DIM];
                v[1] = 1.0;
                v
            }]
        });
        let ranked = idx.search(&unit(1.0));
        assert_eq!(ranked[0].0, "sess-near");
        assert!(ranked[0].1 > ranked[1].1);
    }
    #[test]
    fn test_chunk_hash_stable_and_nonzero() {
        assert_eq!(chunk_hash("user: hello"), chunk_hash("user: hello"));
        assert_ne!(chunk_hash("user: hello"), chunk_hash("user: hello!"));
        assert_ne!(chunk_hash(""), 0);
    }
    #[test]
    fn test_entry_meta_without_hash_deserializes_to_zero() {
        // meta.json written by a pre-hash binary must still load; hash
        // defaults to 0 = "never matches", forcing a one-time re-embed.
        let e: EntryMeta =
            serde_json::from_str(r#"{"session_id":"s","project":"p","offset":0,"timestamp":"t"}"#)
                .unwrap();
        assert_eq!(e.hash, 0);
    }
    #[test]
    fn test_checkpoint_keeps_leftover_sessions_and_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let mut idx = Index {
            vectors: vec![],
            meta: Meta::default(),
        };
        let ch = vec![Chunk {
            text: "a".into(),
            offset: 0,
            timestamp: "t".into(),
        }];
        append_session(&mut idx, &ch, "processed", "p", |_| vec![unit(1.0)]);
        idx.meta
            .manifest
            .files
            .insert("processed.jsonl".into(), (2, 2));

        let mut leftovers = HashMap::new();
        leftovers.insert(
            "unreached".to_string(),
            (
                vec![EntryMeta {
                    session_id: "unreached".into(),
                    project: "p".into(),
                    offset: 0,
                    timestamp: "t".into(),
                    hash: 7,
                }],
                unit(0.5),
            ),
        );
        let mut old_manifest = HashMap::new();
        old_manifest.insert("unreached.jsonl".to_string(), (1u64, 1u64));
        // Stale old signature for a processed file: the fresh one must win.
        old_manifest.insert("processed.jsonl".to_string(), (9u64, 9u64));

        idx.save_with_leftovers(&leftovers, &old_manifest, tmp.path())
            .unwrap();
        let loaded = Index::load(tmp.path()).unwrap();
        assert_eq!(loaded.meta.entries.len(), 2);
        assert_eq!(loaded.vectors.len(), 2 * DIM);
        let by = loaded.by_session();
        assert_eq!(by["unreached"].0[0].hash, 7);
        assert_eq!(loaded.meta.manifest.files["unreached.jsonl"], (1, 1));
        assert_eq!(loaded.meta.manifest.files["processed.jsonl"], (2, 2));
    }
    #[test]
    fn test_load_rejects_misaligned_pair() {
        let tmp = tempfile::tempdir().unwrap();
        let mut idx = Index {
            vectors: vec![],
            meta: Meta::default(),
        };
        let ch = vec![Chunk {
            text: "a".into(),
            offset: 0,
            timestamp: "t".into(),
        }];
        append_session(&mut idx, &ch, "s", "p", |_| vec![unit(1.0)]);
        idx.save(tmp.path()).unwrap();
        // Truncate vectors.bin: entry count no longer matches vector count.
        std::fs::write(Index::vectors_path(tmp.path()), [0u8; 8]).unwrap();
        assert!(Index::load(tmp.path()).is_none());
    }
    #[test]
    fn test_save_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut idx = Index {
            vectors: vec![],
            meta: Meta::default(),
        };
        let ch = vec![Chunk {
            text: "a".into(),
            offset: 2,
            timestamp: "t".into(),
        }];
        append_session(&mut idx, &ch, "s", "proj", |_| vec![unit(0.5)]);
        idx.save(tmp.path()).unwrap();
        let loaded = Index::load(tmp.path()).unwrap();
        assert_eq!(loaded.meta.entries.len(), 1);
        assert_eq!(loaded.meta.entries[0].offset, 2);
        assert_eq!(loaded.vectors.len(), DIM);
    }
}
