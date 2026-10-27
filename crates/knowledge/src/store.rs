//! The knowledge database: chunks, their full-text index, and what is known about each source. Everything in it is
//! derived from the repository and the project, so the file can be deleted and rebuilt, and a change in how it is
//! laid out drops the old tables rather than migrating them.

use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    Error, Result,
    chunk::{Chunk, SourceKind},
    tokens::{index_text, match_expression, path_text},
};

/// Bumped when the tables change. An older file is emptied and rebuilt.
const SCHEMA_VERSION: i32 = 3;

/// A chunk as it comes back from a search.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub id: i64,
    pub kind: SourceKind,
    pub source: String,
    pub path: Option<String>,
    pub label: Option<String>,
    pub start_line: u32,
    pub end_line: u32,
    pub text: String,
    /// Higher is better. Only comparable within one search.
    pub score: f64,
}

/// What is recorded about a source besides its chunks.
#[derive(Debug, Clone, Copy)]
pub struct SourceMeta<'a> {
    /// A hash of the source's content. A source whose hash hasn't changed isn't read again.
    pub hash: &'a str,
    pub bytes: u64,
    /// Something to show in the index health view, such as why a file was held back.
    pub note: Option<&'a str>,
}

impl<'a> SourceMeta<'a> {
    pub fn new(hash: &'a str, bytes: u64, note: Option<&'a str>) -> Self {
        Self { hash, bytes, note }
    }
}

/// A search over one project's chunks.
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    pub text: String,
    /// The kinds to search. Empty means all of them.
    pub kinds: Vec<SourceKind>,
    pub limit: usize,
    /// Files the question is already about. Their chunks rank higher.
    pub boost_paths: Vec<String>,
    /// Sources (issue ids, commits) the question is already about.
    pub boost_sources: Vec<String>,
}

/// What the knowledge base holds for a project, for the index health view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Health {
    pub sources: usize,
    pub chunks: usize,
    pub code_chunks: usize,
    pub doc_chunks: usize,
    pub issue_chunks: usize,
    pub commit_chunks: usize,
    /// When the newest source was indexed, in milliseconds since the epoch.
    pub last_indexed_at: Option<i64>,
    /// Sources with something to say: files held back as secrets, files read as plain text because they didn't parse,
    /// and files that had secret text taken out. Each is a path and a note.
    pub notes: Vec<(String, String)>,
    /// The size of the database file's pages, in bytes.
    pub database_bytes: u64,
}

pub struct KnowledgeStore {
    pub(crate) connection: Connection,
}

pub(crate) fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

impl KnowledgeStore {
    /// Opens the database at `path`, creating it when it isn't there.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| Error::Io(err.to_string()))?;
        }
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        Self::prepare(connection)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(connection: Connection) -> Result<Self> {
        let version: i32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != SCHEMA_VERSION {
            connection.execute_batch(
                "DROP TABLE IF EXISTS chunks_fts;
                 DROP TABLE IF EXISTS chunks;
                 DROP TABLE IF EXISTS sources;
                 DROP TABLE IF EXISTS meta;
                 DROP TABLE IF EXISTS commit_links;
                 DROP TABLE IF EXISTS issue_facts;
                 DROP TABLE IF EXISTS commit_facts;
                 DROP TABLE IF EXISTS chunk_vectors;",
            )?;
        }
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS sources (
                 project    TEXT    NOT NULL,
                 kind       TEXT    NOT NULL,
                 ref        TEXT    NOT NULL,
                 hash       TEXT    NOT NULL,
                 chunks     INTEGER NOT NULL,
                 bytes      INTEGER NOT NULL,
                 indexed_at INTEGER NOT NULL,
                 note       TEXT,
                 PRIMARY KEY (project, kind, ref)
             );
             CREATE TABLE IF NOT EXISTS chunks (
                 id         INTEGER PRIMARY KEY,
                 project    TEXT    NOT NULL,
                 kind       TEXT    NOT NULL,
                 source     TEXT    NOT NULL,
                 path       TEXT,
                 label      TEXT,
                 start_line INTEGER NOT NULL,
                 end_line   INTEGER NOT NULL,
                 text       TEXT    NOT NULL
             );
             CREATE INDEX IF NOT EXISTS chunks_by_source ON chunks (project, kind, source);
             CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(terms, path_terms, label_terms);
             CREATE TABLE IF NOT EXISTS meta (
                 project TEXT NOT NULL,
                 key     TEXT NOT NULL,
                 value   TEXT NOT NULL,
                 PRIMARY KEY (project, key)
             );
             CREATE TABLE IF NOT EXISTS issue_facts (
                 project     TEXT NOT NULL,
                 id          TEXT NOT NULL,
                 title       TEXT NOT NULL,
                 status      TEXT NOT NULL,
                 priority    TEXT,
                 assignee    TEXT,
                 tags        TEXT NOT NULL,
                 description TEXT NOT NULL,
                 PRIMARY KEY (project, id)
             );
             CREATE TABLE IF NOT EXISTS commit_facts (
                 project TEXT    NOT NULL,
                 sha     TEXT    NOT NULL,
                 author  TEXT    NOT NULL,
                 date    INTEGER NOT NULL,
                 subject TEXT    NOT NULL,
                 message TEXT    NOT NULL,
                 files   TEXT    NOT NULL,
                 PRIMARY KEY (project, sha)
             );
             CREATE TABLE IF NOT EXISTS chunk_vectors (
                 chunk_id INTEGER PRIMARY KEY,
                 project  TEXT NOT NULL,
                 model    TEXT NOT NULL,
                 vector   BLOB NOT NULL
             );
             CREATE INDEX IF NOT EXISTS chunk_vectors_by_project ON chunk_vectors (project, model);
             CREATE TABLE IF NOT EXISTS commit_links (
                 project TEXT NOT NULL,
                 sha     TEXT NOT NULL,
                 issue   TEXT NOT NULL,
                 PRIMARY KEY (project, sha, issue)
             );",
        )?;
        connection.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(Self { connection })
    }

    /// The content hash recorded for a source, if it has been indexed.
    pub fn source_hash(
        &self,
        project: &str,
        kind: SourceKind,
        reference: &str,
    ) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT hash FROM sources WHERE project = ?1 AND kind = ?2 AND ref = ?3",
                params![project, kind.stored(), reference],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Every source of a kind that has been indexed, by reference.
    pub fn source_refs(&self, project: &str, kind: SourceKind) -> Result<Vec<String>> {
        let mut statement = self
            .connection
            .prepare("SELECT ref FROM sources WHERE project = ?1 AND kind = ?2 ORDER BY ref")?;
        let rows = statement.query_map(params![project, kind.stored()], |row| row.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Replaces everything a source contributed with `chunks`, and records its hash. All in one transaction, so a
    /// search never sees a source half-replaced.
    pub fn replace_source(
        &mut self,
        project: &str,
        kind: SourceKind,
        reference: &str,
        meta: &SourceMeta<'_>,
        chunks: &[Chunk],
    ) -> Result<()> {
        let SourceMeta { hash, bytes, note } = *meta;
        let transaction = self.connection.transaction()?;
        delete_chunks(&transaction, project, kind, reference)?;
        for chunk in chunks {
            transaction.execute(
                "INSERT INTO chunks (project, kind, source, path, label, start_line, end_line, text)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    project,
                    chunk.kind.stored(),
                    chunk.source,
                    chunk.path,
                    chunk.label,
                    chunk.start_line,
                    chunk.end_line,
                    chunk.text
                ],
            )?;
            let id = transaction.last_insert_rowid();
            transaction.execute(
                "INSERT INTO chunks_fts (rowid, terms, path_terms, label_terms) VALUES (?1, ?2, ?3, ?4)",
                params![
                    id,
                    index_text(&chunk.text),
                    chunk.path.as_deref().map(path_text).unwrap_or_default(),
                    chunk.label.as_deref().map(index_text).unwrap_or_default(),
                ],
            )?;
        }
        transaction.execute(
            "INSERT INTO sources (project, kind, ref, hash, chunks, bytes, indexed_at, note)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (project, kind, ref) DO UPDATE SET
                 hash = excluded.hash, chunks = excluded.chunks, bytes = excluded.bytes,
                 indexed_at = excluded.indexed_at, note = excluded.note",
            params![
                project,
                kind.stored(),
                reference,
                hash,
                chunks.len() as i64,
                bytes as i64,
                now_millis(),
                note
            ],
        )?;
        Ok(transaction.commit()?)
    }

    /// Forgets a source and its chunks. A source that was never indexed is fine.
    pub fn remove_source(
        &mut self,
        project: &str,
        kind: SourceKind,
        reference: &str,
    ) -> Result<()> {
        let transaction = self.connection.transaction()?;
        delete_chunks(&transaction, project, kind, reference)?;
        transaction.execute(
            "DELETE FROM sources WHERE project = ?1 AND kind = ?2 AND ref = ?3",
            params![project, kind.stored(), reference],
        )?;
        Ok(transaction.commit()?)
    }

    /// Forgets everything about a project.
    pub fn clear_project(&mut self, project: &str) -> Result<()> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "DELETE FROM chunks_fts WHERE rowid IN (SELECT id FROM chunks WHERE project = ?1)",
            params![project],
        )?;
        for table in ["chunks", "sources", "meta", "commit_links", "chunk_vectors"] {
            transaction.execute(
                &format!("DELETE FROM {table} WHERE project = ?1"),
                params![project],
            )?;
        }
        Ok(transaction.commit()?)
    }

    /// The chunks that best match a question, best first. The full-text index finds the candidates and ranks them
    /// with BM25, weighting a match in a path or a symbol name above one in the body. Then the boosts are applied.
    pub fn search(&self, project: &str, query: &SearchQuery) -> Result<Vec<Hit>> {
        let Some(expression) = match_expression(&query.text) else {
            return Ok(Vec::new());
        };
        let limit = query.limit.max(1);
        let kinds: Vec<&str> = query.kinds.iter().map(|kind| kind.stored()).collect();
        let kind_filter = if kinds.is_empty() {
            String::new()
        } else {
            format!(
                " AND c.kind IN ({})",
                kinds
                    .iter()
                    .map(|kind| format!("'{kind}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let sql = format!(
            "SELECT c.id, c.kind, c.source, c.path, c.label, c.start_line, c.end_line, c.text,
                    bm25(chunks_fts, 1.0, 3.0, 5.0) AS rank
             FROM chunks_fts JOIN chunks c ON c.id = chunks_fts.rowid
             WHERE chunks_fts MATCH ?1 AND c.project = ?2{kind_filter}
             ORDER BY rank LIMIT ?3"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let rows =
            statement.query_map(params![expression, project, (limit * 4) as i64], |row| {
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
                    // BM25 in SQLite is lower for better matches, and negative.
                    score: -row.get::<_, f64>(8)?,
                })
            })?;
        let mut hits: Vec<Hit> = rows.collect::<std::result::Result<_, _>>()?;
        for hit in &mut hits {
            if hit
                .path
                .as_ref()
                .is_some_and(|path| query.boost_paths.contains(path))
            {
                hit.score *= 1.5;
            }
            if query.boost_sources.contains(&hit.source) {
                hit.score *= 1.3;
            }
        }
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.id.cmp(&b.id))
        });
        hits.truncate(limit);
        Ok(hits)
    }

    /// What the knowledge base holds for a project.
    pub fn health(&self, project: &str) -> Result<Health> {
        let sources = self.connection.query_row(
            "SELECT COUNT(*) FROM sources WHERE project = ?1",
            params![project],
            |row| row.get::<_, i64>(0),
        )? as usize;
        let mut health = Health {
            sources,
            ..Health::default()
        };
        let mut statement = self
            .connection
            .prepare("SELECT kind, COUNT(*) FROM chunks WHERE project = ?1 GROUP BY kind")?;
        let rows = statement.query_map(params![project], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
        })?;
        for row in rows {
            let (kind, count) = row?;
            health.chunks += count;
            match SourceKind::from_stored(&kind) {
                Some(SourceKind::Code) => health.code_chunks = count,
                Some(SourceKind::Doc) => health.doc_chunks = count,
                Some(SourceKind::Issue) => health.issue_chunks = count,
                Some(SourceKind::Commit) => health.commit_chunks = count,
                None => {}
            }
        }
        health.last_indexed_at = self.connection.query_row(
            "SELECT MAX(indexed_at) FROM sources WHERE project = ?1",
            params![project],
            |row| row.get(0),
        )?;
        let mut notes = self.connection.prepare(
            "SELECT ref, note FROM sources WHERE project = ?1 AND note IS NOT NULL ORDER BY ref",
        )?;
        health.notes = notes
            .query_map(params![project], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let pages: i64 = self
            .connection
            .pragma_query_value(None, "page_count", |row| row.get(0))?;
        let size: i64 = self
            .connection
            .pragma_query_value(None, "page_size", |row| row.get(0))?;
        health.database_bytes = (pages * size).max(0) as u64;
        Ok(health)
    }

    pub fn meta(&self, project: &str, key: &str) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE project = ?1 AND key = ?2",
                params![project, key],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn set_meta(&self, project: &str, key: &str, value: &str) -> Result<()> {
        self.connection.execute(
            "INSERT INTO meta (project, key, value) VALUES (?1, ?2, ?3)
             ON CONFLICT (project, key) DO UPDATE SET value = excluded.value",
            params![project, key, value],
        )?;
        Ok(())
    }

    /// Records that a commit relates to an issue.
    pub fn link_commit(&self, project: &str, sha: &str, issue: &str) -> Result<()> {
        self.connection.execute(
            "INSERT OR IGNORE INTO commit_links (project, sha, issue) VALUES (?1, ?2, ?3)",
            params![project, sha, issue],
        )?;
        Ok(())
    }

    /// The commits linked to an issue, as they were recorded.
    pub fn commits_for_issue(&self, project: &str, issue: &str) -> Result<Vec<String>> {
        let mut statement = self.connection.prepare(
            "SELECT sha FROM commit_links WHERE project = ?1 AND issue = ?2 ORDER BY sha",
        )?;
        let rows = statement.query_map(params![project, issue], |row| row.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}

fn delete_chunks(
    connection: &Connection,
    project: &str,
    kind: SourceKind,
    reference: &str,
) -> Result<()> {
    connection.execute(
        "DELETE FROM chunks_fts WHERE rowid IN
             (SELECT id FROM chunks WHERE project = ?1 AND kind = ?2 AND source = ?3)",
        params![project, kind.stored(), reference],
    )?;
    connection.execute(
        "DELETE FROM chunk_vectors WHERE chunk_id IN
             (SELECT id FROM chunks WHERE project = ?1 AND kind = ?2 AND source = ?3)",
        params![project, kind.stored(), reference],
    )?;
    connection.execute(
        "DELETE FROM chunks WHERE project = ?1 AND kind = ?2 AND source = ?3",
        params![project, kind.stored(), reference],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(path: &str, label: &str, text: &str) -> Chunk {
        Chunk {
            kind: SourceKind::Code,
            source: path.to_string(),
            path: Some(path.to_string()),
            label: Some(label.to_string()),
            start_line: 1,
            end_line: 5,
            text: text.to_string(),
        }
    }

    fn query(text: &str) -> SearchQuery {
        SearchQuery {
            text: text.to_string(),
            limit: 10,
            ..Default::default()
        }
    }

    fn store() -> KnowledgeStore {
        KnowledgeStore::open_in_memory().unwrap()
    }

    #[test]
    fn a_question_finds_a_chunk_by_the_words_of_its_identifiers() {
        let mut store = store();
        store
            .replace_source(
                "p",
                SourceKind::Code,
                "src/auth.rs",
                &SourceMeta::new("h1", 10, None),
                &[code(
                    "src/auth.rs",
                    "fn verify_password",
                    "fn verify_password(input: &str) -> bool { input.len() > 8 }",
                )],
            )
            .unwrap();
        let hits = store
            .search("p", &query("how do we verify a password"))
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path.as_deref(), Some("src/auth.rs"));
    }

    #[test]
    fn a_match_in_the_name_ranks_above_a_match_in_the_body() {
        let mut store = store();
        store
            .replace_source(
                "p",
                SourceKind::Code,
                "a.rs",
                &SourceMeta::new("1", 1, None),
                &[code("a.rs", "fn unrelated", "calls retry once")],
            )
            .unwrap();
        store
            .replace_source(
                "p",
                SourceKind::Code,
                "b.rs",
                &SourceMeta::new("2", 1, None),
                &[code("b.rs", "fn retry", "does something else entirely")],
            )
            .unwrap();
        let hits = store.search("p", &query("retry")).unwrap();
        assert_eq!(hits[0].path.as_deref(), Some("b.rs"));
    }

    #[test]
    fn replacing_a_source_replaces_its_chunks_and_nothing_else() {
        let mut store = store();
        store
            .replace_source(
                "p",
                SourceKind::Code,
                "a.rs",
                &SourceMeta::new("1", 1, None),
                &[code("a.rs", "fn legacy_handler", "old")],
            )
            .unwrap();
        store
            .replace_source(
                "p",
                SourceKind::Code,
                "b.rs",
                &SourceMeta::new("1", 1, None),
                &[code("b.rs", "fn keep", "keep")],
            )
            .unwrap();
        store
            .replace_source(
                "p",
                SourceKind::Code,
                "a.rs",
                &SourceMeta::new("2", 1, None),
                &[code("a.rs", "fn fresh_routine", "new")],
            )
            .unwrap();
        assert!(
            store
                .search("p", &query("legacy_handler"))
                .unwrap()
                .is_empty()
        );
        assert_eq!(store.search("p", &query("fresh_routine")).unwrap().len(), 1);
        assert_eq!(store.search("p", &query("keep")).unwrap().len(), 1);
        assert_eq!(
            store
                .source_hash("p", SourceKind::Code, "a.rs")
                .unwrap()
                .as_deref(),
            Some("2")
        );
    }

    #[test]
    fn removing_a_source_takes_it_out_of_search_and_the_health_counts() {
        let mut store = store();
        store
            .replace_source(
                "p",
                SourceKind::Code,
                "a.rs",
                &SourceMeta::new("1", 1, None),
                &[code("a.rs", "fn gone", "gone")],
            )
            .unwrap();
        store.remove_source("p", SourceKind::Code, "a.rs").unwrap();
        store
            .remove_source("p", SourceKind::Code, "never.rs")
            .unwrap();
        assert!(store.search("p", &query("gone")).unwrap().is_empty());
        assert_eq!(store.health("p").unwrap().sources, 0);
        assert_eq!(
            store.source_hash("p", SourceKind::Code, "a.rs").unwrap(),
            None
        );
    }

    #[test]
    fn projects_do_not_see_each_others_chunks() {
        let mut store = store();
        store
            .replace_source(
                "one",
                SourceKind::Code,
                "a.rs",
                &SourceMeta::new("1", 1, None),
                &[code("a.rs", "fn shared_name", "x")],
            )
            .unwrap();
        assert_eq!(store.search("one", &query("shared_name")).unwrap().len(), 1);
        assert!(
            store
                .search("two", &query("shared_name"))
                .unwrap()
                .is_empty()
        );
        store.clear_project("one").unwrap();
        assert!(
            store
                .search("one", &query("shared_name"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_search_can_be_limited_to_some_kinds_and_boosted_toward_a_file() {
        let mut store = store();
        let issue = Chunk {
            kind: SourceKind::Issue,
            source: "t1".into(),
            path: None,
            label: Some("Fix upload".into()),
            start_line: 0,
            end_line: 0,
            text: "Fix upload\nStatus: Todo\n\nupload fails".into(),
        };
        store
            .replace_source(
                "p",
                SourceKind::Issue,
                "t1",
                &SourceMeta::new("1", 1, None),
                &[issue],
            )
            .unwrap();
        store
            .replace_source(
                "p",
                SourceKind::Code,
                "a.rs",
                &SourceMeta::new("1", 1, None),
                &[code("a.rs", "fn upload", "upload bytes")],
            )
            .unwrap();
        store
            .replace_source(
                "p",
                SourceKind::Code,
                "b.rs",
                &SourceMeta::new("1", 1, None),
                &[code("b.rs", "fn upload", "upload bytes")],
            )
            .unwrap();

        let mut issues_only = query("upload");
        issues_only.kinds = vec![SourceKind::Issue];
        let hits = store.search("p", &issues_only).unwrap();
        assert!(hits.iter().all(|hit| hit.kind == SourceKind::Issue));

        let mut boosted = query("upload");
        boosted.kinds = vec![SourceKind::Code];
        boosted.boost_paths = vec!["b.rs".to_string()];
        assert_eq!(
            store.search("p", &boosted).unwrap()[0].path.as_deref(),
            Some("b.rs")
        );
    }

    #[test]
    fn health_counts_chunks_by_kind_and_lists_notes() {
        let mut store = store();
        store
            .replace_source(
                "p",
                SourceKind::Code,
                "a.rs",
                &SourceMeta::new("1", 100, Some("read as plain text: syntax error on line 3")),
                &[code("a.rs", "fn a", "a")],
            )
            .unwrap();
        store
            .replace_source(
                "p",
                SourceKind::Code,
                ".env",
                &SourceMeta::new("0", 0, Some("held back: looks like a secret file")),
                &[],
            )
            .unwrap();
        let health = store.health("p").unwrap();
        assert_eq!(health.sources, 2);
        assert_eq!(health.code_chunks, 1);
        assert_eq!(health.notes.len(), 2);
        assert!(health.last_indexed_at.is_some());
        assert!(health.database_bytes > 0);
    }

    #[test]
    fn a_question_of_only_stopwords_finds_nothing_and_does_not_fail() {
        let store = store();
        assert!(store.search("p", &query("what is the")).unwrap().is_empty());
        assert!(store.search("p", &query("")).unwrap().is_empty());
    }

    #[test]
    fn meta_and_commit_links_are_stored_per_project() {
        let store = store();
        store.set_meta("p", "head", "abc").unwrap();
        store.set_meta("p", "head", "def").unwrap();
        assert_eq!(store.meta("p", "head").unwrap().as_deref(), Some("def"));
        assert_eq!(store.meta("q", "head").unwrap(), None);
        store.link_commit("p", "c1", "t1").unwrap();
        store.link_commit("p", "c1", "t1").unwrap();
        store.link_commit("p", "c2", "t1").unwrap();
        assert_eq!(store.commits_for_issue("p", "t1").unwrap(), ["c1", "c2"]);
    }

    #[test]
    fn an_older_layout_is_dropped_and_rebuilt() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("knowledge.db");
        {
            let mut store = KnowledgeStore::open(&path).unwrap();
            store
                .replace_source(
                    "p",
                    SourceKind::Code,
                    "a.rs",
                    &SourceMeta::new("1", 1, None),
                    &[code("a.rs", "fn a", "a")],
                )
                .unwrap();
            store
                .connection
                .pragma_update(None, "user_version", 0)
                .unwrap();
        }
        let store = KnowledgeStore::open(&path).unwrap();
        assert_eq!(store.health("p").unwrap().sources, 0);
    }
}
