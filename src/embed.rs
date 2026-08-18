use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use std::path::PathBuf;

/// Embedding dimension of all-MiniLM-L6-v2.
pub const DIM: usize = 384;

/// Where the fastembed model (all-MiniLM-L6-v2 ONNX, ~90 MB) is cached.
///
/// fastembed's default is a RELATIVE `./.fastembed_cache`, so it gets dropped
/// into whatever directory rrecall happens to run from — which, when invoked
/// by the stenographer plugin, is the *current project repo*, littering
/// unrelated trees. Pin it to the XDG data dir (~/.local/share/rrecall/fastembed)
/// via the xdg crate: one stable location, and the data dir (not cache) so a
/// `~/.cache` purge doesn't force the slow re-download.
fn embed_cache_dir() -> PathBuf {
    xdg::BaseDirectories::with_prefix("rrecall")
        .get_data_home()
        .unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("rrecall")
        })
        .join("fastembed")
}

pub struct Embedder {
    model: TextEmbedding,
}

impl Embedder {
    /// Initialise the model (downloads it on first use). Expensive — construct once.
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let cache_dir = embed_cache_dir();
        std::fs::create_dir_all(&cache_dir).ok();
        let model = TextEmbedding::try_new(
            TextInitOptions::new(EmbeddingModel::AllMiniLML6V2).with_cache_dir(cache_dir),
        )?;
        Ok(Self { model })
    }

    pub fn embed_batch(
        &mut self,
        texts: &[&str],
    ) -> Result<Vec<Vec<f32>>, Box<dyn std::error::Error>> {
        Ok(self.model.embed(texts, None)?)
    }

    pub fn embed_one(&mut self, text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
        Ok(self
            .embed_batch(&[text])?
            .into_iter()
            .next()
            .unwrap_or_default())
    }
}

/// Cosine similarity of two equal-length vectors. Returns 0.0 for a zero vector.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

#[cfg(test)]
mod tests {
    use super::cosine;
    #[test]
    fn test_cosine_identical_and_orthogonal() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }
}
