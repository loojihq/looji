//! The application's schema migrations, in the order they are applied.
//!
//! Append new entries; never edit one that has shipped.

use crate::Migration;

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial_schema",
        sql: include_str!("sql/0001_initial.sql"),
    },
    Migration {
        version: 2,
        name: "settings",
        sql: include_str!("sql/0002_settings.sql"),
    },
    Migration {
        version: 3,
        name: "workspace_name_index",
        sql: include_str!("sql/0003_workspace_name_index.sql"),
    },
];
