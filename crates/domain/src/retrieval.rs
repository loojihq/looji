//! Picks the code a task is about, by keyword overlap. Pure: the caller reads the source lines and hands in the
//! candidates. The rules here are the terms a task contains, how a symbol scores against them, and how many
//! hunks fit in a budget.

/// The token budget for one set of hunks, and the most hunks returned, whatever the budget.
pub const DEFAULT_TOKEN_BUDGET: usize = 2_000;
pub const MAX_HUNKS: usize = 8;

/// Words that appear in almost every task or every file, so matching them says nothing.
const STOPWORDS: &[&str] = &[
    "about", "add", "all", "and", "any", "are", "but", "can", "change", "could", "for", "from",
    "has", "have", "into", "its", "make", "new", "not", "should", "the", "their", "then", "this",
    "update", "use", "was", "when", "will", "with", "without", "would", "your", "fix", "need",
    "needs", "also", "some", "what", "how", "why",
];

/// A piece of code to show: a range of lines in one file, with how well it matched and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// Relative to the repository root.
    pub path: String,
    /// 1-based, inclusive.
    pub start_line: u32,
    pub end_line: u32,
    pub score: u32,
    pub text: String,
}

/// The lowercase words in a piece of text that could say what code is about. Words are split at punctuation,
/// spaces, and camelCase boundaries, so `verifyPassword` and `verify_password` both give `verify` and `password`.
/// Short words and stopwords are dropped, and each word appears once.
pub fn task_terms(text: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric()) {
        let mut current = String::new();
        let mut previous_lower = false;
        for c in raw.chars() {
            if c.is_uppercase() && previous_lower && !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            previous_lower = c.is_lowercase();
            current.extend(c.to_lowercase());
        }
        if !current.is_empty() {
            words.push(current);
        }
    }

    let mut terms: Vec<String> = Vec::new();
    for word in words {
        if word.chars().count() >= 3
            && !STOPWORDS.contains(&word.as_str())
            && !terms.contains(&word)
        {
            terms.push(word);
        }
    }
    terms
}

/// How well a symbol matches the task. Each distinct term found in the symbol's name is worth three points, and
/// each found in its file's path is worth one. A term in the name is what the task is most likely about.
pub fn score_symbol(terms: &[String], name: &str, path: &str) -> u32 {
    score_terms(terms, &task_terms(name), &task_terms(path))
}

/// The same score as [`score_symbol`], for words that have already been split. A caller scoring many symbols splits each
/// file path once and passes the result in, instead of splitting it again for every symbol.
pub fn score_terms(terms: &[String], name_terms: &[String], path_terms: &[String]) -> u32 {
    terms
        .iter()
        .map(|term| {
            let in_name = if name_terms.contains(term) { 3 } else { 0 };
            let in_path = if path_terms.contains(term) { 1 } else { 0 };
            in_name + in_path
        })
        .sum()
}

/// Rough token count: about four characters to a token for source code.
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// Chooses the hunks to return. The best-scoring candidates go first, and a candidate that overlaps one already
/// chosen is dropped. Candidates with no match are never chosen, so a task with no matching code gets nothing.
/// Stops at the budget or at `max_hunks`, whichever comes first.
pub fn pack_hunks(mut candidates: Vec<Hunk>, budget_tokens: usize, max_hunks: usize) -> Vec<Hunk> {
    candidates.retain(|hunk| hunk.score > 0);
    candidates.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.path.cmp(&b.path))
            .then(a.start_line.cmp(&b.start_line))
    });

    let mut chosen: Vec<Hunk> = Vec::new();
    let mut used = 0;
    for candidate in candidates {
        if chosen.len() >= max_hunks {
            break;
        }
        let overlaps = chosen.iter().any(|kept| {
            kept.path == candidate.path
                && candidate.start_line <= kept.end_line
                && kept.start_line <= candidate.end_line
        });
        if overlaps {
            continue;
        }
        let cost = estimate_tokens(&candidate.text);
        if used + cost > budget_tokens {
            continue;
        }
        used += cost;
        chosen.push(candidate);
    }
    chosen
}

/// Whether a path is a test file, by the names test runners use. Checks the directory names and the file name.
pub fn is_test_path(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    let (file, folders) = parts
        .split_last()
        .map_or((path, &[][..]), |(file, rest)| (*file, rest));
    let lower = file.to_ascii_lowercase();
    let test_folder = folders.iter().any(|folder| {
        matches!(
            folder.to_ascii_lowercase().as_str(),
            "test" | "tests" | "__tests__" | "spec" | "specs"
        )
    });
    let test_file = lower.starts_with("test_")
        || lower.contains(".test.")
        || lower.contains(".spec.")
        || lower.ends_with("_test.go")
        || lower.ends_with("_test.rs")
        || lower.ends_with("_test.py");
    test_folder || test_file
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::retrieval::{Hunk, is_test_path, pack_hunks, score_symbol, task_terms};

    fn hunk(path: &str, start: u32, end: u32, score: u32, text: &str) -> Hunk {
        Hunk {
            path: path.to_string(),
            start_line: start,
            end_line: end,
            score,
            text: text.to_string(),
        }
    }

    #[test]
    fn identifiers_split_at_case_and_underscores_and_stopwords_drop() {
        assert_eq!(
            task_terms("Fix verifyPassword in the auth_login module"),
            ["verify", "password", "auth", "login", "module"]
        );
    }

    #[test]
    fn short_words_and_repeats_are_dropped() {
        assert_eq!(task_terms("a go to go Go"), Vec::<String>::new());
    }

    #[test]
    fn a_name_match_outranks_a_path_match() {
        let terms = task_terms("password check");
        let in_name = score_symbol(&terms, "check_password", "src/other.rs");
        let in_path = score_symbol(&terms, "helper", "src/password/check.rs");
        assert!(in_name > in_path);
        assert_eq!(score_symbol(&terms, "render", "src/ui.rs"), 0);
    }

    #[test]
    fn packing_keeps_the_best_and_drops_overlaps_and_the_over_budget() {
        let chosen = pack_hunks(
            vec![
                hunk("a.rs", 1, 10, 2, &"x".repeat(40)),
                hunk("a.rs", 5, 8, 9, &"y".repeat(12)),
                hunk("b.rs", 1, 3, 1, &"z".repeat(4000)),
                hunk("c.rs", 1, 3, 0, "never"),
            ],
            100,
            8,
        );
        let paths: Vec<(&str, u32)> = chosen
            .iter()
            .map(|h| (h.path.as_str(), h.start_line))
            .collect();
        assert_eq!(
            paths,
            [("a.rs", 5)],
            "overlap and over-budget are dropped, zero scores never chosen"
        );
    }

    #[test]
    fn packing_respects_the_hunk_count() {
        let many: Vec<Hunk> = (0..20)
            .map(|i| hunk(&format!("f{i}.rs"), 1, 1, 1, "x"))
            .collect();
        assert_eq!(pack_hunks(many, 10_000, 5).len(), 5);
    }

    #[test]
    fn test_files_are_recognised_by_folder_and_name() {
        assert!(is_test_path("tests/invoice.rs"));
        assert!(is_test_path("src/__tests__/app.js"));
        assert!(is_test_path("billing/test_invoice.py"));
        assert!(is_test_path("web/button.test.tsx"));
        assert!(is_test_path("api/handler_test.go"));
        assert!(!is_test_path("src/testing_utils.rs"));
        assert!(!is_test_path("src/attest.rs"));
        assert!(!is_test_path("src/main.rs"));
    }
}
