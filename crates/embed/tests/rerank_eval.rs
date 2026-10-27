//! Would a reranker help? The same question list as the hybrid eval is searched with words and meaning together, and
//! then the top 20 are reordered by a cross-encoder that reads the question and each chunk together. Recall@10 and mean
//! reciprocal rank are compared for each kind of question, with how long the reranking takes. The model is downloaded
//! on first use, so it only runs on request:
//! `cargo test -p looji-embed --test rerank_eval -- --ignored --nocapture`.

use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

use fastembed::{RerankInitOptions, RerankerModel, TextRerank};
use looji_embed::LocalEmbedder;
use looji_knowledge::{
    Hit, KnowledgeStore, SearchQuery, Syncer,
    vectors::{Embedder, embed_pending},
};

const PROJECT: &str = "sample";
/// How many hybrid results the reranker reorders.
const CANDIDATES: usize = 20;
/// The depth recall and rank are measured to.
const DEPTH: usize = 10;

fn knowledge_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../knowledge/tests/fixtures")
}

fn rank(hits: &[Hit], expected: &str) -> Option<usize> {
    hits.iter()
        .take(DEPTH)
        .position(|hit| {
            hit.path
                .as_deref()
                .is_some_and(|path| path.contains(expected))
        })
        .map(|index| index + 1)
}

#[derive(Default)]
struct Score {
    questions: usize,
    found: usize,
    reciprocal_rank: f64,
}

impl Score {
    fn add(&mut self, position: Option<usize>) {
        self.questions += 1;
        if let Some(position) = position {
            self.found += 1;
            self.reciprocal_rank += 1.0 / position as f64;
        }
    }
}

#[test]
#[ignore = "downloads and runs the embedding model and a reranker"]
fn does_reranking_the_hybrid_results_help() {
    let root = knowledge_dir().join("sample");
    let mut store = KnowledgeStore::open_in_memory().unwrap();
    Syncer::new()
        .unwrap()
        .sync_repo(&mut store, PROJECT, &root)
        .unwrap();
    let model = LocalEmbedder::new(std::env::temp_dir().join("looji-embed-test"));
    embed_pending(
        &store,
        PROJECT,
        &model,
        16,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    let mut reranker = TextRerank::try_new(
        RerankInitOptions::new(RerankerModel::JINARerankerV1TurboEn)
            .with_cache_dir(std::env::temp_dir().join("looji-rerank-test"))
            .with_show_download_progress(true),
    )
    .unwrap();

    let mut before: BTreeMap<String, Score> = BTreeMap::new();
    let mut after: BTreeMap<String, Score> = BTreeMap::new();
    let mut spent = Duration::ZERO;
    let mut reranked = 0u32;
    let list = fs::read_to_string(knowledge_dir().join("questions.txt")).unwrap();
    for line in list
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
    {
        let parts: Vec<&str> = line.split('|').map(str::trim).collect();
        if parts[0] == "issues" {
            continue;
        }
        let query = SearchQuery {
            text: parts[1].to_string(),
            limit: CANDIDATES,
            ..Default::default()
        };
        let vector = model.embed(&[parts[1].to_string()]).unwrap().remove(0);
        let hybrid = store
            .search_hybrid(PROJECT, &query, Some((model.model_id(), &vector)))
            .unwrap();

        let documents: Vec<&str> = hybrid.iter().map(|hit| hit.text.as_str()).collect();
        let started = Instant::now();
        let order = reranker.rerank(parts[1], &documents, false, None).unwrap();
        spent += started.elapsed();
        reranked += 1;
        let reordered: Vec<Hit> = order
            .iter()
            .map(|result| hybrid[result.index].clone())
            .collect();

        before
            .entry(parts[0].to_string())
            .or_default()
            .add(rank(&hybrid, parts[2]));
        after
            .entry(parts[0].to_string())
            .or_default()
            .add(rank(&reordered, parts[2]));
    }
    for (kind, was) in &before {
        let now = &after[kind];
        let n = was.questions as f64;
        println!(
            "{kind:<11} hybrid recall {:.2} mrr {:.2}   reranked recall {:.2} mrr {:.2}   ({} questions)",
            was.found as f64 / n,
            was.reciprocal_rank / n,
            now.found as f64 / n,
            now.reciprocal_rank / n,
            was.questions
        );
    }
    println!(
        "reranking {CANDIDATES} chunks took {:.0} ms per question on average",
        spent.as_secs_f64() * 1000.0 / f64::from(reranked)
    );
}
