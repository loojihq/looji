//! Structured facts beside the chunks: the issues and commits as records a tool can filter, and the questions about the
//! code that search alone answers badly, such as "where is this name defined" and "where is it used".

use rusqlite::{OptionalExtension, params};

use crate::{
    Result,
    chunk::{CommitDoc, IssueDoc, SourceKind},
    store::{Hit, KnowledgeStore},
    tokens::identifier_parts,
};

/// Which issues a tool wants. Every field that is set must match.
#[derive(Debug, Clone, Default)]
pub struct IssueFilter {
    /// The status as the app names it, such as "In progress". Case doesn't matter.
    pub status: Option<String>,
    /// Part of the assignee's name.
    pub assignee: Option<String>,
    pub tag: Option<String>,
    /// A word that must be in the title or the description.
    pub text: Option<String>,
    pub limit: usize,
}

const TAG_SEPARATOR: char = '\u{1f}';

impl KnowledgeStore {
    /// Records an issue's facts, replacing the ones it had.
    pub fn put_issue(&self, project: &str, issue: &IssueDoc) -> Result<()> {
        self.connection.execute(
            "INSERT INTO issue_facts (project, id, title, status, priority, assignee, tags, description)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (project, id) DO UPDATE SET
                 title = excluded.title, status = excluded.status, priority = excluded.priority,
                 assignee = excluded.assignee, tags = excluded.tags, description = excluded.description",
            params![
                project,
                issue.id,
                issue.title,
                issue.status,
                issue.priority,
                issue.assignee,
                issue.tags.join(&TAG_SEPARATOR.to_string()),
                issue.description
            ],
        )?;
        Ok(())
    }

    pub fn remove_issue(&self, project: &str, id: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM issue_facts WHERE project = ?1 AND id = ?2",
            params![project, id],
        )?;
        Ok(())
    }

    fn read_issue(row: &rusqlite::Row<'_>) -> rusqlite::Result<IssueDoc> {
        let tags: String = row.get(6)?;
        Ok(IssueDoc {
            id: row.get(0)?,
            title: row.get(1)?,
            status: row.get(2)?,
            priority: row.get(3)?,
            assignee: row.get(4)?,
            description: row.get(5)?,
            tags: tags
                .split(TAG_SEPARATOR)
                .filter(|tag| !tag.is_empty())
                .map(str::to_string)
                .collect(),
        })
    }

    /// The issues that match, in the order the board lists them.
    pub fn issues(&self, project: &str, filter: &IssueFilter) -> Result<Vec<IssueDoc>> {
        let mut statement = self.connection.prepare(
            "SELECT id, title, status, priority, assignee, description, tags
             FROM issue_facts WHERE project = ?1 ORDER BY rowid",
        )?;
        let rows = statement.query_map(params![project], Self::read_issue)?;
        let limit = if filter.limit == 0 {
            usize::MAX
        } else {
            filter.limit
        };
        let mut found = Vec::new();
        for issue in rows {
            let issue = issue?;
            let matches = filter
                .status
                .as_ref()
                .is_none_or(|status| issue.status.eq_ignore_ascii_case(status))
                && filter.assignee.as_ref().is_none_or(|name| {
                    issue.assignee.as_ref().is_some_and(|assignee| {
                        assignee.to_lowercase().contains(&name.to_lowercase())
                    })
                })
                && filter
                    .tag
                    .as_ref()
                    .is_none_or(|tag| issue.tags.iter().any(|have| have.eq_ignore_ascii_case(tag)))
                && filter.text.as_ref().is_none_or(|text| {
                    let text = text.to_lowercase();
                    issue.title.to_lowercase().contains(&text)
                        || issue.description.to_lowercase().contains(&text)
                });
            if matches {
                found.push(issue);
                if found.len() == limit {
                    break;
                }
            }
        }
        Ok(found)
    }

    /// One issue, by its id.
    pub fn issue(&self, project: &str, id: &str) -> Result<Option<IssueDoc>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id, title, status, priority, assignee, description, tags
                 FROM issue_facts WHERE project = ?1 AND id = ?2",
                params![project, id],
                Self::read_issue,
            )
            .optional()?)
    }

    /// Records a commit's facts. A commit never changes, so a second write is the same.
    pub fn put_commit(&self, project: &str, commit: &CommitDoc) -> Result<()> {
        self.connection.execute(
            "INSERT OR REPLACE INTO commit_facts (project, sha, author, date, subject, message, files)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                project,
                commit.sha,
                commit.author,
                commit.date,
                commit.message.lines().next().unwrap_or_default(),
                commit.message,
                commit.files.join("\n")
            ],
        )?;
        Ok(())
    }

    fn read_commit(row: &rusqlite::Row<'_>) -> rusqlite::Result<CommitDoc> {
        let files: String = row.get(4)?;
        Ok(CommitDoc {
            sha: row.get(0)?,
            author: row.get(1)?,
            date: row.get(2)?,
            message: row.get(3)?,
            files: files.lines().map(str::to_string).collect(),
        })
    }

    /// The newest commits, newest first. With `path`, only the commits that touched a file whose path contains it.
    pub fn commits(
        &self,
        project: &str,
        path: Option<&str>,
        limit: usize,
    ) -> Result<Vec<CommitDoc>> {
        let mut statement = self.connection.prepare(
            "SELECT sha, author, date, message, files FROM commit_facts
             WHERE project = ?1 ORDER BY date DESC, rowid DESC",
        )?;
        let rows = statement.query_map(params![project], Self::read_commit)?;
        let mut found = Vec::new();
        for commit in rows {
            let commit = commit?;
            if path.is_none_or(|fragment| commit.files.iter().any(|file| file.contains(fragment))) {
                found.push(commit);
                if found.len() == limit {
                    break;
                }
            }
        }
        Ok(found)
    }

    /// A commit by its id, or by the start of it.
    pub fn commit(&self, project: &str, sha: &str) -> Result<Option<CommitDoc>> {
        Ok(self
            .connection
            .query_row(
                "SELECT sha, author, date, message, files FROM commit_facts
                 WHERE project = ?1 AND sha LIKE ?2 || '%' LIMIT 1",
                params![project, sha],
                Self::read_commit,
            )
            .optional()?)
    }

    /// The chunks of code that define something whose name contains `name`, shortest name first. The names are the
    /// labels the chunker gave: `fn login`, `type Session`, `mod auth`.
    pub fn find_labelled(&self, project: &str, name: &str, limit: usize) -> Result<Vec<Hit>> {
        let pattern = format!(
            "%{}%",
            name.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let mut statement = self.connection.prepare(
            "SELECT id, kind, source, path, label, start_line, end_line, text FROM chunks
             WHERE project = ?1 AND kind = 'code' AND label LIKE ?2 ESCAPE '\\'
             ORDER BY length(label), path, start_line LIMIT ?3",
        )?;
        let rows = statement.query_map(params![project, pattern, limit as i64], |row| {
            Ok(Hit {
                id: row.get(0)?,
                kind: SourceKind::Code,
                source: row.get(2)?,
                path: row.get(3)?,
                label: row.get(4)?,
                start_line: row.get(5)?,
                end_line: row.get(6)?,
                text: row.get(7)?,
                score: 1.0,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// The chunks that contain `needle` as a whole word, found through the index and then checked against the text.
    /// This is a text search, so it finds uses by name and cannot tell two things with the same name apart.
    pub fn find_text(&self, project: &str, needle: &str, limit: usize) -> Result<Vec<Hit>> {
        let parts = identifier_parts(needle);
        let whole = needle.to_lowercase();
        let query = if parts.len() > 1 {
            whole.clone()
        } else {
            parts.first().cloned().unwrap_or(whole)
        };
        if query.is_empty() || !query.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Ok(Vec::new());
        }
        let mut statement = self.connection.prepare(
            "SELECT c.id, c.kind, c.source, c.path, c.label, c.start_line, c.end_line, c.text
             FROM chunks_fts JOIN chunks c ON c.id = chunks_fts.rowid
             WHERE chunks_fts MATCH ?1 AND c.project = ?2 AND c.kind IN ('code', 'doc')
             ORDER BY c.path, c.start_line LIMIT 400",
        )?;
        let rows = statement.query_map(params![format!("\"{query}\""), project], |row| {
            let kind: String = row.get(1)?;
            Ok(Hit {
                id: row.get(0)?,
                kind: SourceKind::from_stored(&kind).unwrap_or(SourceKind::Code),
                source: row.get(2)?,
                path: row.get(3)?,
                label: row.get(4)?,
                start_line: row.get(5)?,
                end_line: row.get(6)?,
                text: row.get(7)?,
                score: 1.0,
            })
        })?;
        let mut found = Vec::new();
        for hit in rows {
            let hit = hit?;
            if contains_word(&hit.text, needle) {
                found.push(hit);
                if found.len() == limit {
                    break;
                }
            }
        }
        Ok(found)
    }

    /// The files in the knowledge base, with their kinds and sizes, for the repository map.
    pub fn files(&self, project: &str) -> Result<Vec<(String, SourceKind, u64)>> {
        let mut statement = self.connection.prepare(
            "SELECT ref, kind, bytes FROM sources
             WHERE project = ?1 AND kind IN ('code', 'doc') AND chunks > 0 ORDER BY ref",
        )?;
        let rows = statement.query_map(params![project], |row| {
            let kind: String = row.get(1)?;
            Ok((
                row.get::<_, String>(0)?,
                SourceKind::from_stored(&kind).unwrap_or(SourceKind::Code),
                row.get::<_, i64>(2)?.max(0) as u64,
            ))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}

/// Whether `word` appears in `text` with no letter, digit, or underscore on either side.
pub fn contains_word(text: &str, word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    let is_part = |c: char| c.is_alphanumeric() || c == '_';
    text.match_indices(word).any(|(start, found)| {
        let before = text[..start].chars().next_back();
        let after = text[start + found.len()..].chars().next();
        !before.is_some_and(is_part) && !after.is_some_and(is_part)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        chunk::{Chunk, SourceKind},
        store::SourceMeta,
    };

    fn issue(
        id: &str,
        title: &str,
        status: &str,
        assignee: Option<&str>,
        tags: &[&str],
    ) -> IssueDoc {
        IssueDoc {
            id: id.into(),
            title: title.into(),
            description: format!("Details about {title}."),
            status: status.into(),
            priority: Some("High".into()),
            assignee: assignee.map(str::to_string),
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
        }
    }

    fn code(path: &str, label: &str, text: &str, start: u32) -> Chunk {
        Chunk {
            kind: SourceKind::Code,
            source: path.into(),
            path: Some(path.into()),
            label: Some(label.into()),
            start_line: start,
            end_line: start + text.lines().count() as u32 - 1,
            text: text.into(),
        }
    }

    #[test]
    fn issues_are_filtered_by_status_assignee_tag_and_text() {
        let store = KnowledgeStore::open_in_memory().unwrap();
        store
            .put_issue(
                "p",
                &issue(
                    "1",
                    "Fix login timeout",
                    "In progress",
                    Some("Maya"),
                    &["auth"],
                ),
            )
            .unwrap();
        store
            .put_issue(
                "p",
                &issue("2", "Add refunds", "Todo", None, &["payments", "money"]),
            )
            .unwrap();
        store
            .put_issue(
                "p",
                &issue("3", "Polish checkout", "Todo", Some("Sam Lee"), &[]),
            )
            .unwrap();
        let find = |filter: IssueFilter| -> Vec<String> {
            store
                .issues("p", &filter)
                .unwrap()
                .into_iter()
                .map(|issue| issue.id)
                .collect()
        };

        assert_eq!(find(IssueFilter::default()), ["1", "2", "3"]);
        assert_eq!(
            find(IssueFilter {
                status: Some("todo".into()),
                ..Default::default()
            }),
            ["2", "3"]
        );
        assert_eq!(
            find(IssueFilter {
                assignee: Some("lee".into()),
                ..Default::default()
            }),
            ["3"]
        );
        assert_eq!(
            find(IssueFilter {
                tag: Some("Money".into()),
                ..Default::default()
            }),
            ["2"]
        );
        assert_eq!(
            find(IssueFilter {
                text: Some("timeout".into()),
                ..Default::default()
            }),
            ["1"]
        );
        assert_eq!(
            find(IssueFilter {
                status: Some("Todo".into()),
                limit: 1,
                ..Default::default()
            }),
            ["2"]
        );
        assert_eq!(
            store.issue("p", "2").unwrap().unwrap().tags,
            ["payments", "money"]
        );
        store.remove_issue("p", "2").unwrap();
        assert_eq!(store.issue("p", "2").unwrap(), None);
    }

    #[test]
    fn commits_come_newest_first_and_can_be_limited_to_a_path() {
        let store = KnowledgeStore::open_in_memory().unwrap();
        for (sha, date, files) in [
            ("aaa111", 100, vec!["src/auth.rs"]),
            ("bbb222", 300, vec!["src/pay.rs", "README.md"]),
            ("ccc333", 200, vec!["src/auth.rs"]),
        ] {
            store
                .put_commit(
                    "p",
                    &CommitDoc {
                        sha: sha.into(),
                        author: "Maya".into(),
                        date,
                        message: format!("Change {sha}\n\nbody"),
                        files: files.into_iter().map(str::to_string).collect(),
                    },
                )
                .unwrap();
        }
        let shas = |path: Option<&str>| -> Vec<String> {
            store
                .commits("p", path, 10)
                .unwrap()
                .into_iter()
                .map(|commit| commit.sha)
                .collect()
        };
        assert_eq!(shas(None), ["bbb222", "ccc333", "aaa111"]);
        assert_eq!(shas(Some("auth")), ["ccc333", "aaa111"]);
        assert_eq!(store.commit("p", "ccc").unwrap().unwrap().sha, "ccc333");
        assert_eq!(store.commit("p", "zzz").unwrap(), None);
    }

    #[test]
    fn a_definition_is_found_by_part_of_its_name() {
        let mut store = KnowledgeStore::open_in_memory().unwrap();
        store
            .replace_source(
                "p",
                SourceKind::Code,
                "a.rs",
                &SourceMeta::new("1", 1, None),
                &[
                    code("a.rs", "fn verify_password", "fn verify_password() {}", 1),
                    code("a.rs", "fn verify_token", "fn verify_token() {}", 5),
                ],
            )
            .unwrap();
        let hits = store.find_labelled("p", "verify_pass", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].label.as_deref(), Some("fn verify_password"));
        assert_eq!(store.find_labelled("p", "verify", 10).unwrap().len(), 2);
        // An underscore in the name is a character, not a wildcard.
        assert!(
            store
                .find_labelled("p", "verify%pass", 10)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn uses_are_found_as_whole_words_only() {
        let mut store = KnowledgeStore::open_in_memory().unwrap();
        store
            .replace_source(
                "p",
                SourceKind::Code,
                "a.rs",
                &SourceMeta::new("1", 1, None),
                &[code(
                    "a.rs",
                    "fn sign_in",
                    "fn sign_in() {\n    verify_password(x);\n}",
                    1,
                )],
            )
            .unwrap();
        store
            .replace_source(
                "p",
                SourceKind::Code,
                "b.rs",
                &SourceMeta::new("1", 1, None),
                &[code(
                    "b.rs",
                    "fn other",
                    "fn other() {\n    verify_password_strength(x);\n}",
                    1,
                )],
            )
            .unwrap();
        let hits = store.find_text("p", "verify_password", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path.as_deref(), Some("a.rs"));
    }

    #[test]
    fn a_word_is_not_found_inside_a_longer_one() {
        assert!(contains_word("call login(x)", "login"));
        assert!(contains_word("login", "login"));
        assert!(!contains_word("relogin(x)", "login"));
        assert!(!contains_word("login_form(x)", "login"));
        assert!(!contains_word("", "login"));
    }
}
