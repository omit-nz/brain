//! Note embeddings. A batch that fails is retried a note at a time, so one note the model cannot
//! take is marked failed on its own; when every note fails and the embedder also refuses a
//! known-good text, the embedder is blamed and nothing is marked, so the queue resumes once it
//! works again.

use std::path::PathBuf;

use anyhow::{Context, bail};
use ll_core::embed::{EmbeddingProvider, ModelConfig};
use ll_core::models::bge_small::{BgeSmallProvider, ensure_model};
use rusqlite::params;

use crate::Index;

/// The ONNX Runtime library `ORT_DYLIB_PATH` names, once it is known to load. The ort crate waits
/// forever on a file it cannot load, so a truncated, foreign or non-library file is refused here,
/// by name. Never downloads a runtime.
pub fn runtime() -> anyhow::Result<PathBuf> {
    let ort = PathBuf::from(
        std::env::var_os("ORT_DYLIB_PATH")
            .context("ORT_DYLIB_PATH is not set, so there is no ONNX Runtime")?,
    );
    if !ort.is_file() {
        bail!(
            "ORT_DYLIB_PATH names {}, which is not a file",
            ort.display()
        );
    }
    // SAFETY: loading runs the library's initialisers, which the ort crate runs anyway when it
    // loads the same file; this only learns whether it loads at all.
    unsafe { libloading::Library::new(&ort) }.with_context(|| {
        format!(
            "ORT_DYLIB_PATH names {}, which cannot be loaded as ONNX Runtime",
            ort.display()
        )
    })?;
    Ok(ort)
}

/// The bge-small provider over the ONNX Runtime brain ships. A debug build given
/// `BRAIN_TEST_EMBEDDER` gets the hash embedder instead, so a test can drive the commands that
/// embed without a model download; release builds never read it.
pub fn load() -> anyhow::Result<Box<dyn EmbeddingProvider>> {
    if cfg!(debug_assertions) && std::env::var_os("BRAIN_TEST_EMBEDDER").is_some() {
        return Ok(Box::new(HashEmbedder::new(8)));
    }
    runtime()?;
    let (model, tokenizer) = ensure_model()?;
    Ok(Box::new(BgeSmallProvider::from_files(&model, &tokenizer)?))
}

/// A text every working embedder takes, used to tell a failing embedder from failing notes.
const KNOWN_GOOD: &str = "brain";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Embedded {
    pub done: usize,
    pub failed: usize,
    pub pending: usize,
    /// Why embedding stopped, when the embedder failed on every note it was given.
    pub stopped: Option<String>,
}

/// A deterministic embedder from word hashes, for tests that need vectors without a model.
pub struct HashEmbedder {
    pub dim: usize,
    config: ModelConfig,
}

impl HashEmbedder {
    pub fn new(dim: usize) -> HashEmbedder {
        HashEmbedder {
            dim,
            config: ModelConfig::new("hash".into(), dim, usize::MAX),
        }
    }
}

impl EmbeddingProvider for HashEmbedder {
    fn embed_batch(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0f32; self.dim];
                for word in t
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|w| !w.is_empty())
                {
                    // FNV-1a, so the vectors are the same on every platform and Rust version.
                    let h = word
                        .to_lowercase()
                        .bytes()
                        .fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
                            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
                        });
                    v[(h % self.dim as u64) as usize] += 1.0;
                }
                let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                if norm > 0.0 {
                    v.iter_mut().for_each(|x| *x /= norm);
                }
                v
            })
            .collect())
    }

    fn config(&self) -> &ModelConfig {
        &self.config
    }
}

/// The note's embedded text: title, tags, then the whole body. The model's tokenizer truncates by
/// tokens; nothing here cuts bytes.
fn input(title: &str, tags: &str, body: &str) -> String {
    format!("{title}\n{tags}\n{body}")
}

fn embed(provider: &dyn EmbeddingProvider, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
    let vecs = provider.embed_documents(texts)?;
    if vecs.len() != texts.len() {
        bail!("{} vectors for {} texts", vecs.len(), texts.len());
    }
    Ok(vecs)
}

impl Index {
    pub fn embed_pending(
        &mut self,
        provider: &dyn EmbeddingProvider,
        batch: usize,
    ) -> anyhow::Result<Embedded> {
        self.embed_rounds(provider, batch, usize::MAX)
    }

    /// One batch of `embed_pending`, so a long first embed can stop between batches.
    pub fn embed_batch(
        &mut self,
        provider: &dyn EmbeddingProvider,
        batch: usize,
    ) -> anyhow::Result<Embedded> {
        self.embed_rounds(provider, batch, 1)
    }

    fn embed_rounds(
        &mut self,
        provider: &dyn EmbeddingProvider,
        batch: usize,
        rounds: usize,
    ) -> anyhow::Result<Embedded> {
        let model = provider.config().model_id.clone();
        let mut out = Embedded::default();
        for _ in 0..rounds {
            let pending: Vec<(i64, String, String)> = {
                let mut st = self.conn.prepare(
                    "SELECT n.id, n.path, c.title, c.tags, c.body
                     FROM notes n JOIN notes_content c USING (id)
                     WHERE n.embedded = 0 ORDER BY n.id LIMIT ?1",
                )?;
                st.query_map([batch as i64], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        input(
                            &r.get::<_, String>(2)?,
                            &r.get::<_, String>(3)?,
                            &r.get::<_, String>(4)?,
                        ),
                    ))
                })?
                .collect::<Result<_, _>>()?
            };
            if pending.is_empty() {
                break;
            }
            let texts: Vec<String> = pending.iter().map(|(_, _, t)| t.clone()).collect();
            let results: Vec<Result<Vec<f32>, String>> = match embed(provider, &texts) {
                Ok(vecs) => vecs.into_iter().map(Ok).collect(),
                Err(batch_err) => {
                    let one: Vec<_> = texts
                        .iter()
                        .map(|t| {
                            embed(provider, std::slice::from_ref(t))
                                .map(|mut v| v.remove(0))
                                .map_err(|e| format!("{e:#}"))
                        })
                        .collect();
                    // Every note failing alone says nothing about the embedder by itself (a batch
                    // can hold one note); one it takes a known-good text from is working.
                    if one.iter().all(Result::is_err)
                        && embed(provider, &[KNOWN_GOOD.to_string()]).is_err()
                    {
                        out.stopped = Some(format!("{batch_err:#}"));
                        break;
                    }
                    one
                }
            };
            let tx = self.conn.transaction()?;
            for ((id, path, _), result) in pending.iter().zip(results) {
                match result {
                    Ok(v) => {
                        let data: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
                        tx.execute(
                            "INSERT INTO embeddings(id, model, data) VALUES (?1, ?2, ?3)
                             ON CONFLICT(id) DO UPDATE SET model = excluded.model, data = excluded.data",
                            params![id, model, data],
                        )?;
                        tx.execute("UPDATE notes SET embedded = 1 WHERE id = ?1", [id])?;
                        tx.execute("DELETE FROM skipped WHERE path = ?1", [path])?;
                        out.done += 1;
                    }
                    Err(why) => {
                        tx.execute("UPDATE notes SET embedded = -1 WHERE id = ?1", [id])?;
                        tx.execute(
                            "INSERT INTO skipped(path, why) VALUES (?1, ?2)
                             ON CONFLICT(path) DO UPDATE SET why = excluded.why",
                            params![path, format!("embedding failed: {why}")],
                        )?;
                        out.failed += 1;
                    }
                }
            }
            tx.commit()?;
        }
        out.pending =
            self.conn
                .query_row("SELECT count(*) FROM notes WHERE embedded = 0", [], |r| {
                    r.get::<_, i64>(0)
                })? as usize;
        Ok(out)
    }
}
