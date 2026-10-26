//! How well search finds the right thing. A small sample repository and a list of questions, each with the file or
//! issue that should come back. The measures are recall@10 (the right answer is in the first ten) and mean
//! reciprocal rank (how near the top it is), for each kind of question, so a change that helps one kind and hurts
//! another is caught here. Run with `--nocapture` to see the numbers.

use std::{collections::BTreeMap, fs, path::Path};

use looji_knowledge::{Hit, IssueDoc, KnowledgeStore, SearchQuery, SourceKind, Syncer};

const PROJECT: &str = "sample";

struct Question {
    kind: String,
    text: String,
    expected: String,
}

fn questions() -> Vec<Question> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/questions.txt");
    fs::read_to_string(path)
        .expect("the question list is checked in")
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| {
            let parts: Vec<&str> = line.split('|').map(str::trim).collect();
            assert_eq!(parts.len(), 3, "{line}");
            Question {
                kind: parts[0].to_string(),
                text: parts[1].to_string(),
                expected: parts[2].to_string(),
            }
        })
        .collect()
}

fn issue(
    id: &str,
    title: &str,
    description: &str,
    status: &str,
    assignee: Option<&str>,
) -> IssueDoc {
    IssueDoc {
        id: id.to_string(),
        title: title.to_string(),
        description: description.to_string(),
        status: status.to_string(),
        priority: None,
        assignee: assignee.map(str::to_string),
        tags: Vec::new(),
    }
}

fn issues() -> Vec<IssueDoc> {
    vec![
        issue(
            "i-login-timeout",
            "Fix login timeout",
            "Customers are signed out after a few minutes because the session times out too early.",
            "In progress",
            Some("Maya"),
        ),
        issue(
            "i-partial-refunds",
            "Support partial refunds",
            "Refunds only work for the whole payment. Allow giving back part of the amount to the card.",
            "Todo",
            None,
        ),
        issue(
            "i-postcode",
            "Postcode check rejects valid addresses",
            "The shipping address validation refuses postcodes that have a space in them.",
            "Todo",
            Some("Sam"),
        ),
        issue(
            "i-finance-export",
            "Finance wants the sales spreadsheet export every month",
            "Run the sales export on the first of the month and mail it to finance.",
            "Backlog",
            None,
        ),
        issue(
            "i-invoice-email",
            "Invoice emails arrive without the PDF",
            "Some customers get the invoice email but the attachment is missing.",
            "In review",
            Some("Priya"),
        ),
        issue(
            "i-colours",
            "Update the storefront colours",
            "The brand colours changed. Update the buttons and headers.",
            "Todo",
            None,
        ),
        issue(
            "i-search",
            "Improve product search ranking",
            "Popular products should come first in the results.",
            "Backlog",
            None,
        ),
        issue(
            "i-onboarding",
            "Welcome email for new customers",
            "Send a short welcome message after the first order.",
            "Done",
            Some("Maya"),
        ),
    ]
}

fn indexed() -> KnowledgeStore {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample");
    let mut store = KnowledgeStore::open_in_memory().unwrap();
    let mut syncer = Syncer::new().unwrap();
    syncer.sync_repo(&mut store, PROJECT, &root).unwrap();
    syncer.sync_issues(&mut store, PROJECT, &issues()).unwrap();
    store
}

/// The 1-based position of the expected answer in the hits, if it is there.
fn rank(hits: &[Hit], expected: &str) -> Option<usize> {
    hits.iter()
        .position(|hit| match expected.strip_prefix("issue:") {
            Some(id) => hit.kind == SourceKind::Issue && hit.source == id,
            None => hit
                .path
                .as_deref()
                .is_some_and(|path| path.contains(expected)),
        })
        .map(|index| index + 1)
}

#[derive(Default)]
struct Score {
    questions: usize,
    found: usize,
    reciprocal_rank: f64,
}

fn run() -> BTreeMap<String, Score> {
    let store = indexed();
    let mut scores: BTreeMap<String, Score> = BTreeMap::new();
    for question in questions() {
        let hits = store
            .search(
                PROJECT,
                &SearchQuery {
                    text: question.text.clone(),
                    limit: 10,
                    ..Default::default()
                },
            )
            .unwrap();
        let score = scores.entry(question.kind.clone()).or_default();
        score.questions += 1;
        match rank(&hits, &question.expected) {
            Some(position) => {
                score.found += 1;
                score.reciprocal_rank += 1.0 / position as f64;
            }
            None => println!(
                "MISS [{}] {}  (wanted {})",
                question.kind, question.text, question.expected
            ),
        }
    }
    scores
}

#[test]
fn search_finds_the_right_answer_for_each_kind_of_question() {
    // The lowest recall@10 and mean reciprocal rank each kind of question may have. They sit just under what lexical
    // search scores today, and rise when a change makes search better. Known misses are questions that share no word
    // with their answer, such as "how is the system structured" for a document titled Architecture. Those are what
    // embeddings are for, and the floors for paraphrases and docs are meant to go up when they arrive.
    let floors: [(&str, f64, f64); 4] = [
        ("identifier", 1.0, 0.9),
        ("paraphrase", 0.8, 0.5),
        ("docs", 0.6, 0.6),
        ("issues", 0.8, 0.5),
    ];
    let scores = run();
    for (kind, recall_floor, mrr_floor) in floors {
        let score = &scores[kind];
        let recall = score.found as f64 / score.questions as f64;
        let mrr = score.reciprocal_rank / score.questions as f64;
        println!(
            "{kind:<11} recall@10 {recall:.2}  mrr {mrr:.2}  ({} questions)",
            score.questions
        );
        assert!(
            recall >= recall_floor,
            "{kind}: recall@10 {recall:.2} is under {recall_floor}"
        );
        assert!(
            mrr >= mrr_floor,
            "{kind}: mrr {mrr:.2} is under {mrr_floor}"
        );
    }
}

#[test]
fn a_question_with_no_connection_to_the_project_finds_nothing() {
    let store = indexed();
    let hits = store
        .search(
            PROJECT,
            &SearchQuery {
                text: "zebra migration patterns in the savanna".into(),
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(hits.is_empty(), "{hits:?}");
}

#[test]
fn searching_the_sample_repository_is_fast() {
    let store = indexed();
    let started = std::time::Instant::now();
    for _ in 0..50 {
        store
            .search(
                PROJECT,
                &SearchQuery {
                    text: "how do customers get billed on their credit card".into(),
                    limit: 10,
                    ..Default::default()
                },
            )
            .unwrap();
    }
    let each = started.elapsed() / 50;
    assert!(
        each < std::time::Duration::from_millis(50),
        "a search took {each:?}"
    );
}
