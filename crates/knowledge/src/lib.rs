//! The project knowledge base. The repository's code and docs, the project's issues, and the repository's commits are
//! cut into chunks, stored in SQLite with a full-text index, and searched with BM25 ranking over identifier-aware
//! words. Everything here is derived data: it is rebuilt from the sources, and kept current as they change.
//!
//! The pieces:
//! - `tokens`: how text becomes searchable words.
//! - `chunk`: how a source is cut.
//! - `safety`: what is never indexed, and how secrets are kept out.
//! - `store`: the database and search.
//! - `sync`: reading a repository into the database, all at once or one change at a time.
//! - `git`: commits, and which issues they are about.
//! - `watch`: noticing changes in the repository.

pub mod chunk;
pub mod drift;
pub mod facts;
pub mod git;
pub mod hints;
pub mod mcp;
pub mod related;
pub mod safety;
pub mod store;
pub mod sync;
pub mod tokens;
pub mod tools;
pub mod vectors;
pub mod watch;

pub use chunk::{Chunk, CommitDoc, IssueDoc, SourceKind};
pub use facts::IssueFilter;
pub use store::{Health, Hit, KnowledgeStore, SearchQuery, SourceMeta};
pub use sync::{SyncReport, Syncer};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("the knowledge database failed: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("a file could not be read or written: {0}")]
    Io(String),
    #[error("the source parser could not start: {0}")]
    Parser(String),
    #[error("git failed: {0}")]
    Git(String),
    #[error("the file watcher failed: {0}")]
    Watch(String),
    #[error("the embedding model failed: {0}")]
    Embed(String),
}

pub type Result<T> = std::result::Result<T, Error>;
