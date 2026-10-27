//! Meaning-based search. Each chunk can carry a vector from an embedding model, and a question is matched to the
//! chunks whose vectors point the same way. That finds a chunk that says the same thing in other words, which the
//! word index can't. The model itself lives in the `looji-embed` crate behind [`Embedder`], so this crate builds
//! without it.
//!
//! Vectors are stored as blobs and compared by brute force. A project has tens of thousands of chunks, not millions,
//! and a pass over them takes a few milliseconds, so there is no index to keep in step with the chunks. Vector hits
//! and word hits are merged by rank (reciprocal rank fusion), so an exact identifier still wins when the question
//! names one, and a paraphrase still finds its chunk when it doesn't.

use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, Ordering},
};

use rusqlite::params;

use crate::{
    Error, Result,
    chunk::SourceKind,
    store::{Hit, KnowledgeStore, SearchQuery},
};

/// Turns text into vectors. Vectors are unit length, so the dot product of two is their cosine.
pub trait Embedder: Send + Sync {
    /// Names the model, so vectors from another model are never compared with these.
    fn model_id(&self) -> &str;
    /// One vector for each text, in order.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}

/// The reciprocal-rank-fusion constant. Larger values flatten the difference between ranks.
const RRF_K: f64 = 60.0;
/// A vector hit must be at least this similar to the question to count.
const MIN_SIMILARITY: f32 = 0.25;
/// Most characters of a chunk that go to the model.
const MAX_EMBED_CHARS: usize = 1_500;

/// What the model reads for a chunk: where it is and what it is called, then its text.
pub fn embedding_text(path: Option<&str>, label: Option<&str>, text: &str) -> String {
    let mut out = String::new();
    if let Some(path) = path {
        out.push_str(path);
        out.push(' ');
    }
    if let Some(label) = label {
        out.push_str(label);
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(text);
    if out.len() > MAX_EMBED_CHARS {
        let mut end = MAX_EMBED_CHARS;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
    }
    out
}

fn encode(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn decode(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| f32::from_le_bytes(*word))
        .collect()
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

impl KnowledgeStore {
    /// Chunks that have no vector from `model` yet, with the text to embed for each, oldest first.
    pub fn chunks_without_vectors(
        &self,
        project: &str,
        model: &str,
        limit: usize,
    ) -> Result<Vec<(i64, String)>> {
        let mut statement = self.connection.prepare(
            "SELECT c.id, c.path, c.label, c.text FROM chunks c
             LEFT JOIN chunk_vectors v ON v.chunk_id = c.id AND v.model = ?2
             WHERE c.project = ?1 AND v.chunk_id IS NULL
             ORDER BY c.id LIMIT ?3",
        )?;
        let rows = statement.query_map(params![project, model, limit as i64], |row| {
            let path: Option<String> = row.get(1)?;
            let label: Option<String> = row.get(2)?;
            let text: String = row.get(3)?;
            Ok((
                row.get::<_, i64>(0)?,
                embedding_text(path.as_deref(), label.as_deref(), &text),
            ))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Saves vectors for chunks. A chunk that already has one gets the new one.
    pub fn put_vectors(
        &self,
        project: &str,
        model: &str,
        vectors: &[(i64, Vec<f32>)],
    ) -> Result<()> {
        let mut statement = self.connection.prepare(
            "INSERT OR REPLACE INTO chunk_vectors (chunk_id, project, model, vector)
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (id, vector) in vectors {
            statement.execute(params![id, project, model, encode(vector)])?;
        }
        Ok(())
    }

    /// How many chunks of the project have a vector from `model`, and how many chunks there are.
    pub fn vector_progress(&self, project: &str, model: &str) -> Result<(usize, usize)> {
        let done: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM chunk_vectors v JOIN chunks c ON c.id = v.chunk_id
             WHERE c.project = ?1 AND v.model = ?2",
            params![project, model],
            |row| row.get(0),
        )?;
        let total: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM chunks WHERE project = ?1",
            params![project],
            |row| row.get(0),
        )?;
        Ok((done as usize, total as usize))
    }

    /// The chunks whose vectors are closest to `query`, best first, of the given kinds (all, when empty).
    pub fn vector_search(
        &self,
        project: &str,
        model: &str,
        query: &[f32],
        kinds: &[SourceKind],
        limit: usize,
    ) -> Result<Vec<Hit>> {
        let mut statement = self.connection.prepare(
            "SELECT c.id, c.kind, c.source, c.path, c.label, c.start_line, c.end_line, c.text, v.vector
             FROM chunk_vectors v JOIN chunks c ON c.id = v.chunk_id
             WHERE c.project = ?1 AND v.model = ?2",
        )?;
        let mut scored: Vec<Hit> = Vec::new();
        let mut rows = statement.query(params![project, model])?;
        while let Some(row) = rows.next()? {
            let kind =
                SourceKind::from_stored(&row.get::<_, String>(1)?).unwrap_or(SourceKind::Code);
            if !kinds.is_empty() && !kinds.contains(&kind) {
                continue;
            }
            let vector = decode(&row.get::<_, Vec<u8>>(8)?);
            if vector.len() != query.len() {
                continue;
            }
            let similarity = dot(query, &vector);
            if similarity < MIN_SIMILARITY {
                continue;
            }
            scored.push(Hit {
                id: row.get(0)?,
                kind,
                source: row.get(2)?,
                path: row.get(3)?,
                label: row.get(4)?,
                start_line: row.get(5)?,
                end_line: row.get(6)?,
                text: row.get(7)?,
                score: f64::from(similarity),
            });
        }
        scored.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.id.cmp(&b.id)));
        scored.truncate(limit.max(1));
        Ok(scored)
    }

    /// Word search and meaning search together. With no query vector this is the plain word search. With one, both
    /// rankings are merged by reciprocal rank fusion, so a chunk found by both rises above one found by either alone.
    pub fn search_hybrid(
        &self,
        project: &str,
        query: &SearchQuery,
        meaning: Option<(&str, &[f32])>,
    ) -> Result<Vec<Hit>> {
        let Some((model, vector)) = meaning else {
            return self.search(project, query);
        };
        let wide = SearchQuery {
            limit: query.limit.max(1) * 3,
            ..query.clone()
        };
        let words = self.search(project, &wide)?;
        let meanings = self.vector_search(project, model, vector, &query.kinds, wide.limit)?;
        if meanings.is_empty() {
            let mut words = words;
            words.truncate(query.limit.max(1));
            return Ok(words);
        }
        let mut fused: HashMap<i64, (f64, Hit)> = HashMap::new();
        for ranking in [words, meanings] {
            for (rank, hit) in ranking.into_iter().enumerate() {
                let share = 1.0 / (RRF_K + rank as f64 + 1.0);
                fused
                    .entry(hit.id)
                    .and_modify(|(score, _)| *score += share)
                    .or_insert((share, hit));
            }
        }
        let mut hits: Vec<Hit> = fused
            .into_values()
            .map(|(score, mut hit)| {
                hit.score = score;
                hit
            })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.id.cmp(&b.id)));
        hits.truncate(query.limit.max(1));
        Ok(hits)
    }
}

/// Gives vectors to the chunks that lack them, `batch` at a time, until none are left or `stop` is set. Returns how
/// many were embedded, and calls `progress` with the running total after each batch. Safe to call again after an interruption: it picks up where it left off.
pub fn embed_pending(
    store: &KnowledgeStore,
    project: &str,
    embedder: &dyn Embedder,
    batch: usize,
    stop: &AtomicBool,
    progress: &mut dyn FnMut(usize),
) -> Result<usize> {
    let mut total = 0;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(total);
        }
        let pending = store.chunks_without_vectors(project, embedder.model_id(), batch.max(1))?;
        if pending.is_empty() {
            return Ok(total);
        }
        let texts: Vec<String> = pending.iter().map(|(_, text)| text.clone()).collect();
        let vectors = embedder.embed(&texts)?;
        if vectors.len() != pending.len() {
            return Err(Error::Embed(format!(
                "the model returned {} vectors for {} texts",
                vectors.len(),
                pending.len()
            )));
        }
        let saved: Vec<(i64, Vec<f32>)> = pending.iter().map(|(id, _)| *id).zip(vectors).collect();
        store.put_vectors(project, embedder.model_id(), &saved)?;
        total += saved.len();
        progress(total);
    }
}

/// The question as a vector, for a hybrid search. `None` when the project has no vectors from this model yet, which
/// also means the model has never run here, so a search never waits on a download. `None` too if the model fails.
pub fn embed_query(
    store: &KnowledgeStore,
    project: &str,
    embedder: &dyn Embedder,
    text: &str,
) -> Option<Vec<f32>> {
    let (done, _) = store.vector_progress(project, embedder.model_id()).ok()?;
    if done == 0 {
        return None;
    }
    embedder.embed(&[text.to_string()]).ok()?.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{chunk::Chunk, store::SourceMeta};

    /// A toy model: each concept is a direction, and words that mean the same share one.
    struct Concepts;

    const CONCEPTS: [&[&str]; 4] = [
        &["login", "signin", "authenticate", "password"],
        &["invoice", "bill", "payment", "charge"],
        &["slow", "latency", "sluggish", "performance"],
        &["export", "download", "pdf"],
    ];

    impl Embedder for Concepts {
        fn model_id(&self) -> &str {
            "concepts-v1"
        }

        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|text| {
                    let lower = text.to_lowercase();
                    let mut vector = vec![0.0f32; CONCEPTS.len()];
                    for (index, words) in CONCEPTS.iter().enumerate() {
                        vector[index] =
                            words.iter().filter(|word| lower.contains(*word)).count() as f32;
                    }
                    let length = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
                    if length > 0.0 {
                        vector.iter_mut().for_each(|v| *v /= length);
                    }
                    vector
                })
                .collect())
        }
    }

    fn chunk(path: &str, label: &str, text: &str) -> Chunk {
        Chunk {
            kind: SourceKind::Code,
            source: path.to_string(),
            path: Some(path.to_string()),
            label: Some(label.to_string()),
            start_line: 1,
            end_line: 3,
            text: text.to_string(),
        }
    }

    fn store() -> KnowledgeStore {
        let mut store = KnowledgeStore::open_in_memory().unwrap();
        for chunk in [
            chunk(
                "auth.rs",
                "fn authenticate",
                "check the password and signin the user",
            ),
            chunk(
                "billing.rs",
                "fn charge",
                "create the invoice and take payment",
            ),
            chunk("cache.rs", "fn warm", "reduce latency on the slow path"),
        ] {
            let source = chunk.source.clone();
            store
                .replace_source(
                    "p",
                    SourceKind::Code,
                    &source,
                    &SourceMeta::new("1", 1, None),
                    &[chunk],
                )
                .unwrap();
        }
        store
    }

    fn query(text: &str) -> SearchQuery {
        SearchQuery {
            text: text.to_string(),
            kinds: Vec::new(),
            limit: 3,
            boost_paths: Vec::new(),
            boost_sources: Vec::new(),
        }
    }

    #[test]
    fn embedding_gives_every_chunk_a_vector_and_a_second_pass_does_nothing() {
        let store = store();
        let stop = AtomicBool::new(false);
        assert_eq!(
            embed_pending(&store, "p", &Concepts, 2, &stop, &mut |_| {}).unwrap(),
            3
        );
        assert_eq!(store.vector_progress("p", "concepts-v1").unwrap(), (3, 3));
        assert_eq!(
            embed_pending(&store, "p", &Concepts, 2, &stop, &mut |_| {}).unwrap(),
            0
        );
    }

    #[test]
    fn a_stop_leaves_the_rest_for_later() {
        let store = store();
        let stop = AtomicBool::new(true);
        assert_eq!(
            embed_pending(&store, "p", &Concepts, 2, &stop, &mut |_| {}).unwrap(),
            0
        );
        assert_eq!(store.vector_progress("p", "concepts-v1").unwrap(), (0, 3));
    }

    #[test]
    fn a_paraphrase_finds_the_chunk_the_words_alone_miss() {
        let store = store();
        embed_pending(
            &store,
            "p",
            &Concepts,
            8,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        // None of these words appear in auth.rs, so the word index finds nothing.
        let question = "authenticate";
        let words = store.search("p", &query("how do users sign in")).unwrap();
        assert!(
            words
                .iter()
                .all(|hit| hit.path.as_deref() != Some("auth.rs")),
            "{words:?}"
        );

        let vector = Concepts
            .embed(&["how do users signin".to_string()])
            .unwrap()
            .remove(0);
        let hits = store
            .search_hybrid(
                "p",
                &query("how do users sign in"),
                Some(("concepts-v1", &vector)),
            )
            .unwrap();
        assert_eq!(
            hits[0].path.as_deref(),
            Some("auth.rs"),
            "{question}: {hits:?}"
        );
    }

    #[test]
    fn an_exact_identifier_still_wins_and_a_chunk_found_by_both_rises() {
        let store = store();
        embed_pending(
            &store,
            "p",
            &Concepts,
            8,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        let vector = Concepts.embed(&["payment".to_string()]).unwrap().remove(0);
        let hits = store
            .search_hybrid(
                "p",
                &query("charge payment"),
                Some(("concepts-v1", &vector)),
            )
            .unwrap();
        assert_eq!(hits[0].path.as_deref(), Some("billing.rs"));
    }

    #[test]
    fn without_a_query_vector_the_search_is_the_word_search() {
        let store = store();
        let hybrid = store.search_hybrid("p", &query("invoice"), None).unwrap();
        let plain = store.search("p", &query("invoice")).unwrap();
        assert_eq!(hybrid.len(), plain.len());
        assert_eq!(hybrid[0].id, plain[0].id);
    }

    #[test]
    fn vectors_from_another_model_are_not_used_and_replaced_chunks_lose_theirs() {
        let mut store = store();
        embed_pending(
            &store,
            "p",
            &Concepts,
            8,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        let vector = Concepts.embed(&["signin".to_string()]).unwrap().remove(0);
        assert!(
            store
                .vector_search("p", "other-model", &vector, &[], 3)
                .unwrap()
                .is_empty()
        );

        store
            .replace_source(
                "p",
                SourceKind::Code,
                "auth.rs",
                &SourceMeta::new("2", 1, None),
                &[chunk("auth.rs", "fn login", "new body")],
            )
            .unwrap();
        assert_eq!(store.vector_progress("p", "concepts-v1").unwrap(), (2, 3));
    }
}
