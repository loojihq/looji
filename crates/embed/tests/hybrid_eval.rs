//! Does matching by meaning help? The knowledge crate's sample repository and question list are searched with words
//! alone and with words and meaning together, and recall@10 and mean reciprocal rank are compared for each kind of
//! question. It runs the real model, which is downloaded on first use, so it only runs on request:
//! `cargo test -p looji-embed --test hybrid_eval -- --ignored --nocapture`.

use std::{collections::BTreeMap, fs, path::Path, sync::atomic::AtomicBool};

use looji_embed::LocalEmbedder;
use looji_knowledge::{
    Hit, KnowledgeStore, SearchQuery, Syncer,
    vectors::{Embedder, embed_pending},
};

const PROJECT: &str = "sample";

fn knowledge_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../knowledge/tests/fixtures")
}

fn rank(hits: &[Hit], expected: &str) -> Option<usize> {
    hits.iter()
        .position(|hit| {
            hit.path
                .as_deref()
                .is_some_and(|path| path.contains(expected))
        })
        .map(|index| index + 1)
}

#[derive(Default, Debug)]
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

    fn recall(&self) -> f64 {
        self.found as f64 / self.questions as f64
    }

    fn mrr(&self) -> f64 {
        self.reciprocal_rank / self.questions as f64
    }
}

#[test]
#[ignore = "downloads and runs the embedding model"]
fn meaning_helps_paraphrases_and_does_not_hurt_identifiers() {
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

    let mut words: BTreeMap<String, Score> = BTreeMap::new();
    let mut both: BTreeMap<String, Score> = BTreeMap::new();
    let list = fs::read_to_string(knowledge_dir().join("questions.txt")).unwrap();
    for line in list
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
    {
        let parts: Vec<&str> = line.split('|').map(str::trim).collect();
        // Issues are not in this index, so their questions are measured in the knowledge crate.
        if parts[0] == "issues" {
            continue;
        }
        let query = SearchQuery {
            text: parts[1].to_string(),
            limit: 10,
            ..Default::default()
        };
        let vector = model.embed(&[parts[1].to_string()]).unwrap().remove(0);
        let plain = store.search(PROJECT, &query).unwrap();
        let hybrid = store
            .search_hybrid(PROJECT, &query, Some((model.model_id(), &vector)))
            .unwrap();
        words
            .entry(parts[0].to_string())
            .or_default()
            .add(rank(&plain, parts[2]));
        both.entry(parts[0].to_string())
            .or_default()
            .add(rank(&hybrid, parts[2]));
    }
    for (kind, plain) in &words {
        let hybrid = &both[kind];
        println!(
            "{kind:<11} words recall {:.2} mrr {:.2}   with meaning recall {:.2} mrr {:.2}",
            plain.recall(),
            plain.mrr(),
            hybrid.recall(),
            hybrid.mrr()
        );
        assert!(
            hybrid.recall() >= plain.recall(),
            "{kind}: meaning lost recall"
        );
    }
    assert!(
        both["identifier"].mrr() >= words["identifier"].mrr() - 0.05,
        "identifier questions got worse"
    );
    assert!(
        both["paraphrase"].mrr() > words["paraphrase"].mrr(),
        "paraphrases did not improve"
    );
}
