//! The app's store and file locations, shared with views through a gpui global.

use std::path::PathBuf;

use gpui::Global;
use looji_persistence::SqliteStore;

pub struct AppDatabase {
    pub store: SqliteStore,
    /// Where workspace icon files live. Read by the views that render or replace icons.
    pub icons_dir: PathBuf,
}

impl Global for AppDatabase {}
