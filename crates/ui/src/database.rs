//! The app's store and file locations, shared with views through a gpui global.

use std::path::PathBuf;

use gpui::Global;
use looji_persistence::SqliteStore;

pub struct AppDatabase {
    pub store: SqliteStore,
    /// The database file. Background work opens its own connection to it, since WAL mode
    /// lets that run alongside the UI's connection.
    pub database_path: PathBuf,
    /// Where workspace icon files live. Read by the views that render or replace icons.
    pub icons_dir: PathBuf,
    /// Where files attached to tasks are copied. Read by the task views that show or add media.
    pub attachments_dir: PathBuf,
    /// The knowledge database: chunks of the repository and the project, and their search index. It is all derived
    /// data, so it can be deleted and rebuilt.
    pub knowledge_path: PathBuf,
}

impl Global for AppDatabase {}
