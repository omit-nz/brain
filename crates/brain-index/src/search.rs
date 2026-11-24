//! Hybrid search: BM25 over the text, vectors when a model is loaded, and the link graph, fused
//! by weighted reciprocal rank; the cross-encoder reranks the top when asked. A shared-only search
//! reads every lane through `notes_shared`, so a private note never takes a place in the results.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::bail;
use brain_note::{Disclosure, Kind};
use ll_core::embed::EmbeddingProvider;
use ll_core::graph::{GraphEdges, personalized_pagerank};
use ll_core::rerank::rerank;
use ll_core::scoring::{
    FtsConfig, FusionWeights, add_weighted_rrf, dot_product, finalize_rrf, try_fts_bm25_query,
};

use crate::Index;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Every note of the person's own.
    Mine,
    /// What a colleague could see.
    SharedOnly,
}

pub struct Query<'a> {
    pub text: &'a str,
    pub limit: usize,
    pub kinds: Option<&'a [Kind]>,
    pub scope: Scope,
    pub rerank: bool,
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub note_uuid: Option<uuid::Uuid>,
    pub path: String,
    pub title: String,
    pub kind: Kind,
    pub disclosure: Disclosure,
    pub score: f64,
    pub snippet: String,
}

const WEIGHTS: &str = "10.0, 5.0, 1.0";

const MINE: FtsConfig =
    FtsConfig::new("notes_fts", "notes_content", "notes", "id", "path", WEIGHTS);

const SHARED: FtsConfig = FtsConfig::new(
    "notes_fts",
    "notes_content",
    "notes_shared",
    "id",
    "path",
    WEIGHTS,
);

/// How many fused candidates the kinds filter and the limit choose from.
const POOL: usize = 200;
/// How many of the top the cross-encoder reorders.
const RERANK_TOP: usize = 30;
const SEEDS: usize = 5;
const DAMPING: f32 = 0.85;
const SNIPPET_CHARS: usize = 200;

fn kind_of(s: &str) -> Kind {
    match s {
        "rule" => Kind::Rule,
        "history" => Kind::History,
        _ => Kind::Knowledge,
    }
}

fn disclosure_of(s: &str) -> Disclosure {
    match s {
        "shared" => Disclosure::Shared,
        "listed" => Disclosure::Listed,
        _ => Disclosure::Private,
    }
}

fn snippet(body: &str) -> String {
    let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(SNIPPET_CHARS) {
        Some((i, _)) => format!("{}...", &flat[..i]),
        None => flat,
    }
}

fn stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

impl Index {
    pub fn search(
        &self,
        q: &Query<'_>,
        provider: Option<&dyn EmbeddingProvider>,
    ) -> anyhow::Result<Vec<Hit>> {
        if q.rerank && !std::env::var_os("ORT_DYLIB_PATH").is_some_and(|p| Path::new(&p).is_file())
        {
            bail!("reranking needs ONNX Runtime, and ORT_DYLIB_PATH does not name a file");
        }
        let text = q.text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            return Ok(Vec::new());
        }
        let config = match q.scope {
            Scope::Mine => &MINE,
            Scope::SharedOnly => &SHARED,
        };
        let items = config.items_table;
        let weights = FusionWeights::default();
        let mut fused = HashMap::new();

        let bm25 = try_fts_bm25_query(&self.conn, &text, POOL, config)?;
        add_weighted_rrf(
            &mut fused,
            weights.bm25,
            bm25.iter().map(|(_, p, _)| p.as_str()),
        );

        if let Some(provider) = provider {
            let qv = provider.embed_query(&text)?;
            let mut st = self.conn.prepare(&format!(
                "SELECT n.path, e.data FROM embeddings e JOIN {items} n ON n.id = e.id
                 WHERE e.model = ?1"
            ))?;
            let mut scored: Vec<(String, f32)> = st
                .query_map([&provider.config().model_id], |r| {
                    let data: Vec<u8> = r.get(1)?;
                    let v: Vec<f32> = data
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|b| f32::from_le_bytes(*b))
                        .collect();
                    Ok((r.get(0)?, dot_product(&qv, &v)))
                })?
                .collect::<Result<_, _>>()?;
            scored.sort_by(|a, b| b.1.total_cmp(&a.1));
            scored.truncate(POOL);
            add_weighted_rrf(
                &mut fused,
                weights.vec,
                scored.iter().map(|(p, _)| p.as_str()),
            );
        }

        let seeds: Vec<String> = finalize_rrf(fused.clone(), SEEDS)
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        if !seeds.is_empty() {
            let graph = self.graph(items)?;
            let ranked = personalized_pagerank(&graph, &seeds, DAMPING, ll_core::PAGERANK_ITERS);
            add_weighted_rrf(
                &mut fused,
                weights.ppr,
                ranked.iter().map(|(p, _)| p.as_str()),
            );
        }

        let mut hits = Vec::new();
        let mut st = self.conn.prepare(&format!(
            "SELECT n.note_uuid, n.title, n.kind, n.disclosure, c.body
             FROM {items} n JOIN notes_content c ON c.id = n.id WHERE n.path = ?1"
        ))?;
        for (path, score) in finalize_rrf(fused, POOL) {
            let row = st.query_row([&path], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            });
            let Ok((uuid, title, kind, disclosure, body)) = row else {
                continue;
            };
            let kind = kind_of(&kind);
            if q.kinds.is_some_and(|ks| !ks.contains(&kind)) {
                continue;
            }
            hits.push((
                Hit {
                    note_uuid: uuid.and_then(|u| u.parse().ok()),
                    path,
                    title,
                    kind,
                    disclosure: disclosure_of(&disclosure),
                    score,
                    snippet: snippet(&body),
                },
                body,
            ));
        }

        if q.rerank && !hits.is_empty() {
            let top = hits.len().min(RERANK_TOP);
            let docs: Vec<(String, String)> = hits[..top]
                .iter()
                .map(|(h, body)| (h.path.clone(), format!("{}\n{body}", h.title)))
                .collect();
            let order = rerank(&text, &docs, top);
            let mut head: Vec<Option<(Hit, String)>> = hits.drain(..top).map(Some).collect();
            let mut reranked: Vec<(Hit, String)> = order
                .iter()
                .filter_map(|r| {
                    head[r.index].take().map(|(mut h, b)| {
                        h.score = r.score;
                        (h, b)
                    })
                })
                .collect();
            // A document the cross-encoder could not score keeps its fused place after the rest.
            reranked.extend(head.into_iter().flatten());
            reranked.append(&mut hits);
            hits = reranked;
        }

        hits.truncate(q.limit);
        Ok(hits.into_iter().map(|(h, _)| h).collect())
    }

    /// Links between notes in `items`, both ways, with targets resolved by lowercased file stem
    /// as wikilinks are. A note outside `items` is not in the graph, so it cannot be reached.
    fn graph(&self, items: &str) -> anyhow::Result<GraphEdges> {
        let mut st = self.conn.prepare(&format!("SELECT path FROM {items}"))?;
        let paths: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        let in_scope: HashSet<&str> = paths.iter().map(String::as_str).collect();
        let by_stem: HashMap<String, &str> = paths.iter().map(|p| (stem(p), p.as_str())).collect();
        let mut st = self.conn.prepare(&format!(
            "SELECT n.path, l.target FROM links l JOIN {items} n ON n.id = l.source_id"
        ))?;
        let mut graph = GraphEdges::new();
        for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (source, target) = row?;
            let Some(&target) = by_stem.get(&target) else {
                continue;
            };
            if !in_scope.contains(target) || target == source {
                continue;
            }
            graph
                .entry(source.clone())
                .or_default()
                .push(target.to_string());
            graph.entry(target.to_string()).or_default().push(source);
        }
        Ok(graph)
    }
}
