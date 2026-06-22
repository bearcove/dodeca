//! Dodeca text-embedding processor (Model2Vec static embedder).
//!
//! Loads the vendored potion-base-8M model (a distilled static embedder): an
//! int8 per-row-quantized token-embedding matrix plus its tokenizer, both
//! brotli-compressed (see `assets/potion-base-8m.LICENSE`). Embedding a text is
//! just: tokenize → mean-pool the token rows → L2-normalize. No transformer
//! forward pass, so it runs fast on CPU.

use std::io::Read;
use std::sync::LazyLock;

use cell_embed_proto::{EmbedResult, Embedder};
use tokenizers::Tokenizer;

/// Brotli-compressed `PEM1` model: magic "PEM1", u32 vocab, u32 dim,
/// f32[vocab] row scales, i8[vocab*dim] weights.
static MODEL_BR: &[u8] = include_bytes!("../assets/potion-base-8m.pem.br");
/// Brotli-compressed HF tokenizer JSON, aligned to the model's vocab.
static TOKENIZER_BR: &[u8] = include_bytes!("../assets/potion-base-8m.tokenizer.json.br");

struct Model {
    tokenizer: Tokenizer,
    /// Dequantized embedding matrix, row-major `vocab * dim`.
    weights: Vec<f32>,
    dim: usize,
}

static MODEL: LazyLock<Model> = LazyLock::new(|| Model::load().expect("load vendored embed model"));

fn brotli_decompress(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    brotli::Decompressor::new(bytes, 4096).read_to_end(&mut out)?;
    Ok(out)
}

impl Model {
    fn load() -> Result<Self, String> {
        let tok_json = brotli_decompress(TOKENIZER_BR).map_err(|e| e.to_string())?;
        let tokenizer = Tokenizer::from_bytes(&tok_json).map_err(|e| e.to_string())?;

        let raw = brotli_decompress(MODEL_BR).map_err(|e| e.to_string())?;
        if raw.len() < 12 || &raw[..4] != b"PEM1" {
            return Err("bad embed model header".into());
        }
        let vocab = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
        let dim = u32::from_le_bytes(raw[8..12].try_into().unwrap()) as usize;
        let scales_at = 12;
        let weights_at = scales_at + vocab * 4;
        if raw.len() != weights_at + vocab * dim {
            return Err("embed model size mismatch".into());
        }
        // Dequantize once: weight[i][j] = scale[i] * q[i][j].
        let mut weights = vec![0f32; vocab * dim];
        for row in 0..vocab {
            let scale = f32::from_le_bytes(
                raw[scales_at + row * 4..scales_at + row * 4 + 4]
                    .try_into()
                    .unwrap(),
            );
            let q = &raw[weights_at + row * dim..weights_at + (row + 1) * dim];
            let w = &mut weights[row * dim..(row + 1) * dim];
            for j in 0..dim {
                w[j] = (q[j] as i8 as f32) * scale;
            }
        }
        Ok(Model {
            tokenizer,
            weights,
            dim,
        })
    }

    /// Embed one text into a unit-length vector: mean of its token rows,
    /// L2-normalized. An empty / all-unknown tokenization yields a zero vector.
    fn embed_one(&self, text: &str) -> Result<Vec<f32>, String> {
        let encoding = self
            .tokenizer
            .encode(text, false)
            .map_err(|e| e.to_string())?;
        let ids = encoding.get_ids();
        let mut acc = vec![0f32; self.dim];
        if !ids.is_empty() {
            for &id in ids {
                let row = &self.weights[id as usize * self.dim..(id as usize + 1) * self.dim];
                for (a, &r) in acc.iter_mut().zip(row) {
                    *a += r;
                }
            }
            let inv = 1.0 / ids.len() as f32;
            for a in &mut acc {
                *a *= inv;
            }
            let norm = acc.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 0.0 {
                for a in &mut acc {
                    *a /= norm;
                }
            }
        }
        Ok(acc)
    }
}

/// Embedder implementation. Stateless handle over the lazily-loaded [`MODEL`].
#[derive(Clone)]
pub struct EmbedderImpl;

impl Embedder for EmbedderImpl {
    async fn embed(&self, texts: Vec<String>) -> EmbedResult {
        let model = &*MODEL;
        let mut vectors = Vec::with_capacity(texts.len());
        for text in &texts {
            match model.embed_one(text) {
                Ok(v) => vectors.push(v),
                Err(message) => return EmbedResult::Error { message },
            }
        }
        EmbedResult::Success {
            vectors,
            dim: model.dim as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cos(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }

    /// The Rust port must reproduce the Python validation: the query embeds
    /// closest to the "hosts and compute" passage, far from unrelated ones.
    #[tokio::test]
    async fn ranks_relevant_passage_first() {
        let texts = vec![
            "where does training and GPU compute actually run".to_string(),
            "Hosts and compute: where compute runs, never the laptop".to_string(),
            "Verified facts ledger: only what we reproduced ourselves".to_string(),
            "Sami languages spoken in the arctic by indigenous people".to_string(),
        ];
        let EmbedResult::Success { vectors, dim } = EmbedderImpl.embed(texts).await else {
            panic!("embed failed");
        };
        assert_eq!(dim, 256);
        assert!((vectors[0].iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-3);

        let q = &vectors[0];
        let hosts = cos(q, &vectors[1]);
        let ledger = cos(q, &vectors[2]);
        let sami = cos(q, &vectors[3]);
        assert!(hosts > 0.5, "expected strong match, got {hosts}");
        assert!(hosts > ledger + 0.3, "hosts {hosts} vs ledger {ledger}");
        assert!(hosts > sami + 0.5, "hosts {hosts} vs sami {sami}");
    }
}
