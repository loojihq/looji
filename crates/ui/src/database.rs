//! The app's store, shared with views through a gpui global.

use gpui::Global;
use looji_persistence::SqliteStore;

pub struct AppDatabase(
    // Read by the features that load and save data; held here so it lives as long as the app.
    #[allow(dead_code)] pub SqliteStore,
);

impl Global for AppDatabase {}
