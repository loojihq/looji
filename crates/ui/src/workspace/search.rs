//! Board search, without any window: which tasks match a query, where their text matches, and how a
//! drop lands when only some cards are showing.
//!
//! A query is split on whitespace into terms, and every term has to match somewhere on a card: its
//! title, its description, or one of its tags. Terms are plain text. Nothing is parsed as a pattern,
//! so `*`, `(`, `\`, `%`, `.*`, and the rest are just characters to find.
//!
//! Highlights are byte ranges in the original text. Lowercasing can change a string's length (the
//! capital dotted I, for one), so a match is found in a lowercased copy and mapped back to the
//! original characters. The ranges always fall on character boundaries.

use std::ops::Range;

use looji_domain::{Task, TaskId};

/// How many characters of the description are shown before the first match in a snippet.
const SNIPPET_BEFORE: usize = 20;
/// How many characters of the description are shown from the first match onward.
const SNIPPET_AFTER: usize = 60;

/// The query split into lowercase terms. Empty when the query has no words in it, which means no
/// filtering.
pub fn terms(query: &str) -> Vec<String> {
    looji_domain::search_terms(query)
}

/// Lowercased text, with each byte of it mapped back to the original text's characters.
struct Folded {
    lower: String,
    /// For each byte of `lower`, where the original character it came from starts.
    starts: Vec<usize>,
    /// For each byte of `lower`, where the original character it came from ends.
    ends: Vec<usize>,
}

fn fold(text: &str) -> Folded {
    let mut folded = Folded {
        lower: String::with_capacity(text.len()),
        starts: Vec::with_capacity(text.len()),
        ends: Vec::with_capacity(text.len()),
    };
    for (start, original) in text.char_indices() {
        let end = start + original.len_utf8();
        for lowered in original.to_lowercase() {
            let width = lowered.len_utf8();
            folded.lower.push(lowered);
            folded.starts.extend(std::iter::repeat_n(start, width));
            folded.ends.extend(std::iter::repeat_n(end, width));
        }
    }
    folded
}

fn contains(text: &str, term: &str) -> bool {
    fold(text).lower.contains(term)
}

/// Every place `term` appears in `text`, as byte ranges of `text`, in order.
fn find(text: &str, term: &str) -> Vec<Range<usize>> {
    let folded = fold(text);
    folded
        .lower
        .match_indices(term)
        .map(|(found, matched)| {
            let last = found + matched.len() - 1;
            folded.starts[found]..folded.ends[last]
        })
        .collect()
}

/// Sorts ranges and joins the ones that touch or overlap.
fn merge(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

/// Whether a task matches: each term must appear in its title, its description, or a tag.
pub fn matches(task: &Task, terms: &[String]) -> bool {
    terms
        .iter()
        .all(|term| looji_domain::term_matches(task, term))
}

/// A short piece of the description around its first match, with the matches in it marked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    pub text: String,
    pub ranges: Vec<Range<usize>>,
}

/// Where a card's text matches the query.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hits {
    /// Matches in the title, as byte ranges of the title.
    pub title: Vec<Range<usize>>,
    /// One flag per tag, in the task's tag order: whether that tag matches a term.
    pub tags: Vec<bool>,
    /// The description around its first match. Cards don't show descriptions, so this is how a
    /// match there is made visible.
    pub snippet: Option<Snippet>,
}

/// Where the task's text matches the terms. The terms must already be checked with [`matches`].
pub fn hits(task: &Task, terms: &[String]) -> Hits {
    let title = merge(
        terms
            .iter()
            .flat_map(|term| find(task.title(), term))
            .collect(),
    );
    let description: Vec<Range<usize>> = terms
        .iter()
        .flat_map(|term| find(task.description(), term))
        .collect();
    let tags = task
        .tags()
        .iter()
        .map(|tag| terms.iter().any(|term| contains(tag, term)))
        .collect();
    let snippet =
        (!description.is_empty()).then(|| excerpt(task.description(), &merge(description)));
    Hits {
        title,
        tags,
        snippet,
    }
}

/// The text around the first match, with a "…" where the text was cut. Newlines and tabs become
/// spaces, which keeps every byte offset the same.
fn excerpt(text: &str, ranges: &[Range<usize>]) -> Snippet {
    let first = ranges[0].start;
    let from = text[..first]
        .char_indices()
        .rev()
        .nth(SNIPPET_BEFORE - 1)
        .map_or(0, |(index, _)| index);
    let to = text[first..]
        .char_indices()
        .nth(SNIPPET_AFTER)
        .map_or(text.len(), |(index, _)| first + index);
    let lead = if from > 0 { "…" } else { "" };
    let tail = if to < text.len() { "…" } else { "" };
    let body = text[from..to].replace(['\n', '\r', '\t'], " ");
    let offset = lead.len() as isize - from as isize;
    let ranges = ranges
        .iter()
        .filter(|range| range.end > from && range.start < to)
        .map(|range| {
            let start = range.start.max(from) as isize + offset;
            let end = range.end.min(to) as isize + offset;
            start as usize..end as usize
        })
        .collect();
    Snippet {
        text: format!("{lead}{body}{tail}"),
        ranges,
    }
}

/// Where a dropped card goes in its column, as an index among all the column's cards.
///
/// `full` and `visible` are the column's cards in order, both without the card being moved. `index`
/// is where the card was dropped among `visible`. It lands in front of the visible card at that
/// index, or after the last visible card when it's dropped at the end. Hidden cards keep their
/// place relative to the visible ones around them.
pub fn full_index(full: &[TaskId], visible: &[TaskId], index: usize) -> usize {
    let position = |id: &TaskId| full.iter().position(|other| other == id);
    match visible.get(index) {
        Some(anchor) => position(anchor).unwrap_or(full.len()),
        None => visible
            .last()
            .and_then(position)
            .map_or(full.len(), |last| last + 1),
    }
}

/// Holds a count of keystrokes, so only the latest one can apply the query after its delay. Each
/// keystroke takes a new number. A delayed apply checks that its number is still the latest, so
/// typing several characters applies the query once, for the last one.
#[derive(Debug, Default)]
pub struct Debounce {
    latest: u64,
}

impl Debounce {
    /// Takes the number for a new keystroke. Any earlier number is now stale.
    pub fn touch(&mut self) -> u64 {
        self.latest += 1;
        self.latest
    }

    pub fn is_latest(&self, number: u64) -> bool {
        self.latest == number
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use looji_domain::{Position, ProjectId, Status, Task, TaskId};

    use super::{Debounce, full_index, hits, matches, merge, terms};

    /// Every special character the spec lists, plus a few more that are regex or path syntax.
    const SPECIAL: &[&str] = &[
        "\"", "*", "(", ")", "-", ":", "\\", "%", "_", "[", "^", ".*", "🚀", "+", "?", "{", "|",
        "$", "'", "&", "#",
    ];

    fn task(title: &str, description: &str, tags: &[&str]) -> Task {
        let mut task = Task::new(
            ProjectId::new(),
            title,
            Status::Todo,
            Position::new("a0").unwrap(),
        )
        .unwrap();
        task.set_description(description);
        task.set_tags(tags.iter().map(|tag| tag.to_string()).collect());
        task
    }

    #[test]
    fn a_query_is_split_into_lowercase_terms() {
        assert_eq!(terms("  Foo   BAR "), ["foo", "bar"]);
        assert!(
            terms("   \t ").is_empty(),
            "whitespace-only means no filter"
        );
        assert!(terms("").is_empty());
    }

    #[test]
    fn matching_ignores_case() {
        let card = task("Write Copy", "", &[]);
        assert!(matches(&card, &terms("write COPY")));
    }

    #[test]
    fn every_term_must_match_somewhere_on_the_card() {
        let card = task("Write copy", "for the launch", &["web"]);
        assert!(
            matches(&card, &terms("copy launch")),
            "title and description"
        );
        assert!(matches(&card, &terms("copy web")), "title and tag");
        assert!(!matches(&card, &terms("copy invoice")));
    }

    #[test]
    fn a_description_match_counts() {
        let card = task("Plan", "Ship the Änderung before Friday", &[]);
        assert!(matches(&card, &terms("änderung")));
    }

    #[test]
    fn unicode_text_matches_by_its_lowercase_form() {
        let card = task("ÉCOLE du soir", "", &[]);
        assert!(matches(&card, &terms("école")));
        assert!(matches(&card, &terms("ÉCOLE")));
    }

    #[test]
    fn special_characters_are_literal_and_never_panic() {
        let card = task(
            "Cost (est.) 50% [draft] C:\\temp .* ^start 🚀 done_now",
            "",
            &[],
        );
        for special in SPECIAL {
            // Each one is searched for as text. Matching or not is decided by whether the card
            // contains it, never by pattern syntax.
            let found = matches(&card, &terms(special));
            let contained = card
                .title()
                .to_lowercase()
                .contains(&special.to_lowercase());
            assert_eq!(found, contained, "query {special:?}");
        }
    }

    #[test]
    fn a_dot_star_matches_only_the_literal_text() {
        let plain = task("Anything at all", "", &[]);
        let literal = task("Use .* to match", "", &[]);
        assert!(!matches(&plain, &terms(".*")));
        assert!(matches(&literal, &terms(".*")));
    }

    #[test]
    fn highlights_fall_on_the_matched_text_in_the_title() {
        let card = task("Naïve café launch", "", &[]);
        let found = hits(&card, &terms("café"));
        assert_eq!(found.title.len(), 1);
        let range = found.title[0].clone();
        assert_eq!(&card.title()[range], "café");
    }

    #[test]
    fn highlights_are_valid_after_characters_that_change_length_when_lowercased() {
        // "İ" lowercases to two characters, so the offsets move. Each range must still be on a
        // boundary of the original title, and cover the text that matched.
        let card = task("İstanbul İletişim", "", &[]);
        let found = hits(&card, &terms("i"));
        assert!(!found.title.is_empty());
        for range in &found.title {
            assert!(card.title().is_char_boundary(range.start));
            assert!(card.title().is_char_boundary(range.end));
        }
        let matched: Vec<&str> = found
            .title
            .iter()
            .map(|r| &card.title()[r.clone()])
            .collect();
        assert_eq!(matched[0], "İ", "the dotted capital is one whole highlight");
    }

    #[test]
    fn a_match_in_several_terms_is_highlighted_once() {
        let card = task("copy copy", "", &[]);
        let found = hits(&card, &terms("copy cop"));
        assert_eq!(found.title, [0..4, 5..9]);
    }

    #[test]
    fn tags_that_match_are_flagged() {
        let card = task("Plan", "", &["alpha", "beta"]);
        assert_eq!(hits(&card, &terms("alp")).tags, [true, false]);
    }

    #[test]
    fn a_description_match_gives_a_snippet_with_the_match_marked() {
        let text = format!("{} the target word {}", "x".repeat(40), "y".repeat(100));
        let card = task("Plan", &text, &[]);
        let snippet = hits(&card, &terms("target"))
            .snippet
            .expect("description matched");
        assert!(snippet.text.starts_with('…'), "cut at the start");
        assert!(snippet.text.ends_with('…'), "cut at the end");
        let marked: Vec<&str> = snippet
            .ranges
            .iter()
            .map(|r| &snippet.text[r.clone()])
            .collect();
        assert_eq!(marked, ["target"]);
    }

    #[test]
    fn a_snippet_keeps_multibyte_text_whole() {
        let card = task("Plan", "日本語のテキスト café 検索対象 end", &[]);
        let snippet = hits(&card, &terms("café")).snippet.unwrap();
        let marked: Vec<&str> = snippet
            .ranges
            .iter()
            .map(|r| &snippet.text[r.clone()])
            .collect();
        assert_eq!(marked, ["café"]);
    }

    #[test]
    fn a_snippet_turns_line_breaks_into_spaces_without_moving_the_match() {
        let card = task("Plan", "first line\nthe needle\nlast", &[]);
        let snippet = hits(&card, &terms("needle")).snippet.unwrap();
        assert!(!snippet.text.contains('\n'));
        let marked: Vec<&str> = snippet
            .ranges
            .iter()
            .map(|r| &snippet.text[r.clone()])
            .collect();
        assert_eq!(marked, ["needle"]);
    }

    #[test]
    fn no_snippet_when_only_the_title_matched() {
        let card = task("Plan", "unrelated", &[]);
        assert!(hits(&card, &terms("plan")).snippet.is_none());
    }

    #[test]
    fn a_drop_lands_in_front_of_the_visible_card_at_its_index() {
        let (a, b, c, d) = (TaskId::new(), TaskId::new(), TaskId::new(), TaskId::new());
        // The full column is a, b, c, d; only b and d are visible.
        let full = [a, b, c, d];
        let visible = [b, d];
        assert_eq!(full_index(&full, &visible, 0), 1, "in front of b");
        assert_eq!(
            full_index(&full, &visible, 1),
            3,
            "in front of d, after hidden c"
        );
    }

    #[test]
    fn a_drop_after_the_last_visible_card_goes_after_it_not_at_the_end() {
        let (a, b, c) = (TaskId::new(), TaskId::new(), TaskId::new());
        let full = [a, b, c];
        let visible = [a];
        assert_eq!(
            full_index(&full, &visible, 1),
            1,
            "after a, before hidden b and c"
        );
    }

    #[test]
    fn a_drop_into_a_column_with_nothing_visible_goes_to_the_end() {
        let (a, b) = (TaskId::new(), TaskId::new());
        assert_eq!(full_index(&[a, b], &[], 0), 2);
    }

    #[test]
    fn debouncing_applies_only_the_last_of_several_quick_keystrokes() {
        let mut debounce = Debounce::default();
        // All five keystrokes arrive before any delay ends, so each one takes its number first.
        let numbers: Vec<u64> = (0..5).map(|_| debounce.touch()).collect();
        let applied: Vec<u64> = numbers
            .into_iter()
            .filter(|number| debounce.is_latest(*number))
            .collect();
        assert_eq!(applied, [5], "the query ran once, for the final keystroke");
    }

    #[test]
    fn overlapping_ranges_merge() {
        assert_eq!(merge(vec![0..3, 2..5, 7..8]), [0..5, 7..8]);
    }
}
