//! The embedding model. A small quantised MiniLM runs locally through ONNX Runtime, so no project text leaves the
//! computer. The model files (about 25 MB) are downloaded the first time they are needed and kept in a cache folder.
//! This lives in its own crate so that everything else builds without ONNX Runtime.

use std::{path::PathBuf, sync::Mutex};

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use looji_knowledge::{Error, Result, vectors::Embedder};

/// Texts embedded per model call.
pub const BATCH: usize = 16;

/// The model's name as stored with each vector. Change it when the model changes, so old vectors are not mixed in.
const MODEL_ID: &str = "all-MiniLM-L6-v2-q";

/// The local model. It starts on first use, since starting it may download the files.
pub struct LocalEmbedder {
    cache: PathBuf,
    model: Mutex<Option<TextEmbedding>>,
}

impl LocalEmbedder {
    pub fn new(cache: PathBuf) -> Self {
        Self {
            cache,
            model: Mutex::new(None),
        }
    }

    fn start(&self) -> Result<TextEmbedding> {
        TextEmbedding::try_new(
            TextInitOptions::new(EmbeddingModel::AllMiniLML6V2Q)
                .with_cache_dir(self.cache.clone())
                .with_show_download_progress(false),
        )
        .map_err(|err| Error::Embed(format!("the model could not start: {err}")))
    }
}

/// Scales a vector to length one, so a dot product is a cosine.
fn unit(mut vector: Vec<f32>) -> Vec<f32> {
    let length = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if length > 0.0 {
        vector.iter_mut().for_each(|value| *value /= length);
    }
    vector
}

impl Embedder for LocalEmbedder {
    fn model_id(&self) -> &str {
        MODEL_ID
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut guard = self
            .model
            .lock()
            .map_err(|_| Error::Embed("the model is busy".to_string()))?;
        if guard.is_none() {
            *guard = Some(self.start()?);
        }
        let model = guard.as_mut().expect("the model was just started");
        let vectors = model
            .embed(texts, Some(BATCH))
            .map_err(|err| Error::Embed(err.to_string()))?;
        Ok(vectors.into_iter().map(unit).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_are_scaled_to_length_one_and_zero_stays_zero() {
        let scaled = unit(vec![3.0, 4.0]);
        assert!((scaled[0] - 0.6).abs() < 1e-6 && (scaled[1] - 0.8).abs() < 1e-6);
        assert_eq!(unit(vec![0.0, 0.0]), vec![0.0, 0.0]);
    }

    /// Downloads the model, so it only runs on request: `cargo test -p looji-embed -- --ignored`.
    #[test]
    #[ignore = "downloads the model"]
    fn the_real_model_puts_a_paraphrase_closer_than_an_unrelated_text() {
        let cache = std::env::temp_dir().join("looji-embed-test");
        let model = LocalEmbedder::new(cache);
        let texts = [
            "check the password and sign the user in".to_string(),
            "how does a person log in".to_string(),
            "render the invoice as a PDF".to_string(),
        ];
        let v = model.embed(&texts).unwrap();
        let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        assert!(dot(&v[0], &v[1]) > dot(&v[0], &v[2]) + 0.1);
    }
}
