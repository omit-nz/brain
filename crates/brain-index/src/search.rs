//! Hybrid search: BM25 over the text, vectors when a model is loaded, and the link graph, fused
//! by weighted reciprocal rank; the cross-encoder reranks the top when asked. A shared-only search
//! reads every lane through `notes_shared`, so a private note never takes a place in the results.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Context;
use brain_note::{Disclosure, Kind, Place};
use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use ll_core::embed::EmbeddingProvider;
use ll_core::graph::{GraphEdges, personalized_pagerank};
use ll_core::rerank::rerank;
use ll_core::scoring::{
    FtsConfig, FusionWeights, add_weighted_rrf, dot_product, finalize_rrf, try_fts_bm25_query,
};
use rusqlite::OptionalExtension;

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
    pub place: Place,
    pub score: f64,
    pub snippet: String,
    pub body: String,
    /// The day, here, the note's file was last changed; `None` while it waits for its id.
    pub modified: Option<Date>,
}

/// A search's hits, and how they were judged.
#[derive(Debug, Clone)]
pub struct Found {
    pub hits: Vec<Hit>,
    /// Whether any note matched a word of the question, the loose OR fallback included.
    pub words: bool,
    /// Whether the cross-encoder judged the hits, so only those at or above `RERANK_FLOOR` remain.
    pub reranked: bool,
}

/// The cross-encoder score below which a hit does not answer the question. Fitted on Robin's
/// fifteen questions in Part 2's Live check (2026-11-26): vector similarity could not tell an
/// answer from a near topic (the groups overlapped), while a cut at 0 here kept 3 of 4 answerable
/// questions and dropped 8 of 9 the vault cannot answer.
pub const RERANK_FLOOR: f64 = 0.0;

/// What a search means, decided here once for every door: reranked, a gap when no hit is left
/// above the floor, whatever words matched, since the loose word fallback matches nearly any
/// question on a large vault; otherwise a gap when no word matched.
pub fn is_gap(found: &Found) -> bool {
    if found.reranked {
        found.hits.is_empty()
    } else {
        !found.words
    }
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

fn snippet(body: &str) -> String {
    let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(SNIPPET_CHARS) {
        Some((i, _)) => format!("{}...", &flat[..i]),
        None => flat,
    }
}

/// The day here of a file's time. No file has a time before 1970; a note waiting for its id is
/// stored with one, to be read again.
fn day(mtime_ns: i64) -> Option<Date> {
    let at = (mtime_ns >= 0).then(|| Timestamp::from_nanosecond(mtime_ns.into()).ok())??;
    Some(at.to_zoned(TimeZone::system()).date())
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
    ) -> anyhow::Result<Found> {
        if q.rerank {
            crate::embed::runtime().context("reranking needs ONNX Runtime")?;
        }
        let mut found = Found {
            hits: Vec::new(),
            words: false,
            reranked: false,
        };
        let text = q.text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            return Ok(found);
        }
        let config = match q.scope {
            Scope::Mine => &MINE,
            Scope::SharedOnly => &SHARED,
        };
        let items = config.items_table;
        let weights = FusionWeights::default();
        let mut fused = HashMap::new();

        let bm25 = try_fts_bm25_query(&self.conn, &text, POOL, config)?;
        found.words = !bm25.is_empty();
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
            "SELECT n.note_uuid, n.title, n.kind, n.disclosure, c.body, n.place, n.mtime_ns
             FROM {items} n JOIN notes_content c ON c.id = n.id WHERE n.path = ?1"
        ))?;
        for (path, score) in finalize_rrf(fused, POOL) {
            let row = st
                .query_row([&path], |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, i64>(6)?,
                    ))
                })
                .optional()?;
            // A path the scope's view does not hold is not a hit; any other failure is an error.
            let Some((uuid, title, kind, disclosure, body, place, mtime_ns)) = row else {
                continue;
            };
            let kind = Kind::parse(&kind);
            if q.kinds.is_some_and(|ks| !ks.contains(&kind)) {
                continue;
            }
            hits.push(Hit {
                note_uuid: uuid.and_then(|u| u.parse().ok()),
                path,
                title,
                kind,
                disclosure: Disclosure::parse(&disclosure),
                place: Place::parse(&place),
                score,
                snippet: snippet(&body),
                body,
                modified: day(mtime_ns),
            });
        }

        if q.rerank {
            let top = hits.len().min(RERANK_TOP);
            let docs: Vec<(String, String)> = hits[..top]
                .iter()
                .map(|h| (h.path.clone(), format!("{}\n{}", h.title, h.body)))
                .collect();
            let order = rerank(&text, &docs, top);
            // A cross-encoder that scored nothing (its model would not load) leaves the fused
            // order, judged by words as an unreranked search is.
            if top == 0 || !order.is_empty() {
                let mut head: Vec<Option<Hit>> = hits.drain(..top).map(Some).collect();
                hits = order
                    .iter()
                    .filter(|r| r.score >= RERANK_FLOOR)
                    .filter_map(|r| {
                        head[r.index].take().map(|mut h| {
                            h.score = r.score;
                            h
                        })
                    })
                    .collect();
                found.reranked = true;
            }
        }

        hits.truncate(q.limit);
        found.hits = hits;
        Ok(found)
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
