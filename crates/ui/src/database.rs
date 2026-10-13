//! The open database, shared with views through a gpui global.

use gpui::Global;
use looji_persistence::Connection;

pub struct AppDatabase(
    // Read by the persistence-backed features; held here so it lives as long as the app.
    #[allow(dead_code)] pub Connection,
);

impl Global for AppDatabase {}
