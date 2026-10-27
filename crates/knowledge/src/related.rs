//! Issues that look like another one. A task is compared with the project's other issues by the words in their titles
//! and descriptions, so a second task for the same job is noticed and a task that touches the same subject is offered
//! as related. This reads the issue facts directly, since a project has hundreds of issues and not millions, and it
//! needs no model. Better matching for paraphrases comes with embeddings later.

use std::collections::HashSet;

use crate::{Result, facts::IssueFilter, store::KnowledgeStore, tokens::query_terms};

/// How alike two titles must be for the second to be called a possible duplicate.
pub const DUPLICATE_TITLE_SIMILARITY: f32 = 0.6;
/// How alike two issues must be, overall, to be listed as related.
pub const RELATED_SIMILARITY: f32 = 0.2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelatedIssue {
    pub id: String,
    pub title: String,
    pub status: String,
    /// 0 to 100. Half the titles' overlap and half the whole text's.
    pub percent: u32,
    /// The titles are so alike that this is probably the same job.
    pub possible_duplicate: bool,
}

fn terms(text: &str) -> HashSet<String> {
    query_terms(text).into_iter().collect()
}

/// How much two word sets share, from 0 to 1: the plain overlap, averaged with how much of the smaller set is inside
/// the larger. Two empty sets share nothing.
fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let shared = a.intersection(b).count() as f32;
    let overlap = shared / (a.len() + b.len() - a.intersection(b).count()) as f32;
    let containment = shared / a.len().min(b.len()) as f32;
    // A short title inside a longer one counts for more than the plain overlap says.
    (overlap + containment) / 2.0
}

impl KnowledgeStore {
    /// The project's other issues that look like this one, most alike first. `exclude` is the issue itself, when it
    /// already exists.
    pub fn related_issues(
        &self,
        project: &str,
        title: &str,
        description: &str,
        exclude: Option<&str>,
        limit: usize,
    ) -> Result<Vec<RelatedIssue>> {
        let title_terms = terms(title);
        let all_terms = terms(&format!("{title} {description}"));
        if title_terms.is_empty() {
            return Ok(Vec::new());
        }
        let filter = IssueFilter {
            limit: usize::MAX,
            ..IssueFilter::default()
        };
        let mut found: Vec<RelatedIssue> = self
            .issues(project, &filter)?
            .into_iter()
            .filter(|issue| Some(issue.id.as_str()) != exclude)
            .filter_map(|issue| {
                let title_similarity = jaccard(&title_terms, &terms(&issue.title));
                let overall = jaccard(
                    &all_terms,
                    &terms(&format!("{} {}", issue.title, issue.description)),
                );
                let similarity = 0.5 * title_similarity + 0.5 * overall;
                (similarity >= RELATED_SIMILARITY).then_some(RelatedIssue {
                    possible_duplicate: title_similarity >= DUPLICATE_TITLE_SIMILARITY,
                    id: issue.id,
                    title: issue.title,
                    status: issue.status,
                    percent: (similarity * 100.0).round() as u32,
                })
            })
            .collect();
        found.sort_by_key(|issue| std::cmp::Reverse(issue.percent));
        found.truncate(limit);
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::IssueDoc;

    fn store(issues: &[(&str, &str, &str)]) -> KnowledgeStore {
        let store = KnowledgeStore::open_in_memory().unwrap();
        for (id, title, description) in issues {
            store
                .put_issue(
                    "p",
                    &IssueDoc {
                        id: id.to_string(),
                        title: title.to_string(),
                        description: description.to_string(),
                        status: "To do".into(),
                        priority: None,
                        assignee: None,
                        tags: Vec::new(),
                    },
                )
                .unwrap();
        }
        store
    }

    #[test]
    fn a_second_task_for_the_same_job_is_a_possible_duplicate() {
        let store = store(&[
            ("a", "Fix login timeout when idle", ""),
            ("b", "Add export to PDF", ""),
        ]);
        let found = store
            .related_issues("p", "Fix the login timeout", "", None, 5)
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "a");
        assert!(found[0].possible_duplicate, "{found:?}");
    }

    #[test]
    fn a_task_on_the_same_subject_is_related_but_not_a_duplicate() {
        let store = store(&[(
            "a",
            "Rate limit the login endpoint",
            "Too many password attempts lock the account",
        )]);
        let found = store
            .related_issues(
                "p",
                "Show a message when the account is locked",
                "After too many password attempts",
                None,
                5,
            )
            .unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(!found[0].possible_duplicate);
    }

    #[test]
    fn unrelated_issues_and_the_issue_itself_are_left_out() {
        let store = store(&[
            ("a", "Fix login timeout", ""),
            ("b", "Design the pricing page", "Three tiers"),
        ]);
        let found = store
            .related_issues("p", "Fix login timeout", "", Some("a"), 5)
            .unwrap();
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn the_most_alike_issue_comes_first_and_the_list_is_limited() {
        let store = store(&[
            ("a", "Login page styling", ""),
            ("b", "Fix login timeout when idle", ""),
            ("c", "Fix login timeout", ""),
        ]);
        let found = store
            .related_issues("p", "Fix login timeout", "", None, 2)
            .unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].id, "c");
    }

    #[test]
    fn a_title_with_no_usable_words_matches_nothing() {
        let store = store(&[("a", "Fix login timeout", "")]);
        assert!(
            store
                .related_issues("p", "the a", "", None, 5)
                .unwrap()
                .is_empty()
        );
    }
}
