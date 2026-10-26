//! Turning code and questions into the words the search index matches on.
//!
//! Code is full of identifiers that are one word to the compiler and several to a reader: `retrieveHunks`,
//! `retrieve_hunks`, `RetrieveHunks`. The index text keeps each identifier whole and also adds its parts, and a
//! query is expanded the same way, so a question that says "retrieve hunks" finds `retrieve_hunks` and the other way
//! round. Paths split the same way, at `/`, `.`, `-` and `_`.

/// Words too common in questions to be worth matching. Identifiers like `the` are rare in code, and keeping them in
/// a query only pulls in noise.
const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "can", "do", "does", "for", "from", "how",
    "i", "if", "in", "is", "it", "of", "on", "or", "our", "that", "the", "this", "to", "we",
    "what", "when", "where", "which", "who", "why", "with", "you",
];

/// The pieces of one identifier: split at underscores, at the start of a capitalised word, and between letters and
/// digits. `HTTPServerV2` gives `http`, `server`, `v`, `2`. Everything is lowercase.
pub fn identifier_parts(identifier: &str) -> Vec<String> {
    let chars: Vec<char> = identifier.chars().collect();
    let mut parts = Vec::new();
    let mut current = String::new();
    for (index, &c) in chars.iter().enumerate() {
        if c == '_' || c == '-' {
            flush(&mut current, &mut parts);
            continue;
        }
        if let Some(&previous) = index.checked_sub(1).and_then(|before| chars.get(before)) {
            let next_is_lower = chars.get(index + 1).is_some_and(|next| next.is_lowercase());
            let boundary = !current.is_empty()
                && ((previous.is_lowercase() && c.is_uppercase())
                    || (previous.is_uppercase() && c.is_uppercase() && next_is_lower)
                    || (previous.is_alphabetic() && c.is_ascii_digit())
                    || (previous.is_ascii_digit() && c.is_alphabetic()));
            if boundary {
                flush(&mut current, &mut parts);
            }
        }
        current.extend(c.to_lowercase());
    }
    flush(&mut current, &mut parts);
    parts
}

fn flush(current: &mut String, parts: &mut Vec<String>) {
    if !current.is_empty() {
        parts.push(std::mem::take(current));
    }
}

/// The text the index stores for a piece of content: every word-like token lowercased and whole, then its parts when
/// it has more than one. Punctuation is dropped, so the index sees words and not syntax.
pub fn index_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 2);
    for raw in text.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
        if raw.is_empty() {
            continue;
        }
        let whole = raw.to_lowercase();
        let parts = identifier_parts(raw);
        push_word(&mut out, &whole);
        if parts.len() > 1 || (parts.len() == 1 && parts[0] != whole) {
            for part in &parts {
                if *part != whole {
                    push_word(&mut out, part);
                }
            }
        }
    }
    out
}

fn push_word(out: &mut String, word: &str) {
    if !out.is_empty() {
        out.push(' ');
    }
    out.push_str(word);
}

/// The text a path is indexed under: its parts, split at separators and identifier boundaries.
pub fn path_text(path: &str) -> String {
    index_text(&path.replace(['/', '\\', '.', '-'], " "))
}

/// The words a question searches for, in order, once each. Stopwords and one-letter words are dropped. Identifiers
/// in the question are kept whole as well as split, so an exact name still matches exactly.
pub fn query_terms(question: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for word in index_text(question).split(' ') {
        if word.chars().count() < 2
            || STOPWORDS.contains(&word)
            || terms.iter().any(|seen| seen == word)
        {
            continue;
        }
        terms.push(word.to_string());
    }
    terms
}

/// An FTS5 query for a question: each term quoted, joined with OR, so a chunk that matches more of them ranks
/// higher under BM25. `None` when the question has no usable words.
pub fn match_expression(question: &str) -> Option<String> {
    let terms = query_terms(question);
    if terms.is_empty() {
        return None;
    }
    Some(
        terms
            .iter()
            .map(|term| format!("\"{}\"", term.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_split_at_case_underscores_and_digits() {
        assert_eq!(identifier_parts("retrieveHunks"), ["retrieve", "hunks"]);
        assert_eq!(identifier_parts("retrieve_hunks"), ["retrieve", "hunks"]);
        assert_eq!(identifier_parts("RetrieveHunks"), ["retrieve", "hunks"]);
        assert_eq!(
            identifier_parts("HTTPServerV2"),
            ["http", "server", "v", "2"]
        );
        assert_eq!(identifier_parts("sha256"), ["sha", "256"]);
    }

    #[test]
    fn the_index_keeps_an_identifier_whole_and_adds_its_parts() {
        let text = index_text("fn retrieveHunks(root: &Path)");
        let words: Vec<&str> = text.split(' ').collect();
        assert!(words.contains(&"retrievehunks"));
        assert!(words.contains(&"retrieve"));
        assert!(words.contains(&"hunks"));
        assert!(words.contains(&"root"));
    }

    #[test]
    fn a_plain_question_matches_a_snake_case_name_and_the_other_way_round() {
        let indexed = index_text("pub fn retrieve_hunks()");
        for term in query_terms("how do we retrieve hunks?") {
            assert!(indexed.split(' ').any(|word| word == term), "{term}");
        }
        let camel = query_terms("where is retrieveHunks used");
        assert!(camel.contains(&"retrievehunks".to_string()));
        assert!(camel.contains(&"retrieve".to_string()));
        assert!(indexed.split(' ').any(|word| word == "retrieve"));
    }

    #[test]
    fn stopwords_and_single_letters_are_dropped_and_terms_are_unique() {
        assert_eq!(query_terms("What is the login of a login"), ["login"]);
        assert_eq!(match_expression("the a of"), None);
    }

    #[test]
    fn paths_split_at_separators_and_extensions() {
        assert_eq!(
            path_text("src/auth/login_form.rs"),
            "src auth login_form login form rs"
        );
    }

    #[test]
    fn the_match_expression_quotes_every_term() {
        assert_eq!(
            match_expression("login failure").unwrap(),
            "\"login\" OR \"failure\""
        );
    }
}
