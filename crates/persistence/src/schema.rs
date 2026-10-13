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
    Migration {
        version: 4,
        name: "workspace_icon",
        sql: include_str!("sql/0004_workspace_icon.sql"),
    },
    Migration {
        version: 5,
        name: "project_description",
        sql: include_str!("sql/0005_project_description.sql"),
    },
    Migration {
        version: 6,
        name: "in_review_column",
        sql: include_str!("sql/0006_in_review_column.sql"),
    },
];

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::MIGRATIONS;
    use crate::migrate;

    #[test]
    fn projects_from_before_the_in_review_column_keep_their_data_and_show_both_columns() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        // Everything up to and including the description migration: the previous version.
        migrate(&mut connection, &MIGRATIONS[..5]).unwrap();
        connection
            .execute_batch(
                "INSERT INTO workspaces (id, name, icon_file, created_at, updated_at)
                     VALUES ('w', 'Work', NULL, 0, 0);
                 INSERT INTO projects (id, workspace_id, name, description, created_at, updated_at)
                     VALUES ('p', 'w', 'Website', NULL, 0, 0);
                 INSERT INTO tasks (id, project_id, title, description, status, position,
                                    estimate_minutes, assignee_id, created_at, updated_at)
                     VALUES ('t', 'p', 'Write copy', '', 'in_progress', 'a0', NULL, NULL, 0, 0);",
            )
            .unwrap();

        migrate(&mut connection, MIGRATIONS).unwrap();

        let shown: bool = connection
            .query_row(
                "SELECT show_in_review FROM projects WHERE id = 'p'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(shown, "existing projects must default to showing In review");
        let status: String = connection
            .query_row("SELECT status FROM tasks WHERE id = 't'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            status, "in_progress",
            "the rebuild must keep existing tasks"
        );
    }

    #[test]
    fn the_status_check_accepts_in_review_after_the_rebuild() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        migrate(&mut connection, MIGRATIONS).unwrap();
        connection
            .execute_batch(
                "INSERT INTO workspaces (id, name, icon_file, created_at, updated_at)
                     VALUES ('w', 'Work', NULL, 0, 0);
                 INSERT INTO projects (id, workspace_id, name, description, created_at, updated_at)
                     VALUES ('p', 'w', 'Website', NULL, 0, 0);",
            )
            .unwrap();

        let inserted = connection.execute(
            "INSERT INTO tasks (id, project_id, title, description, status, position,
                                estimate_minutes, assignee_id, created_at, updated_at)
             VALUES ('t', 'p', 'Review', '', 'in_review', 'a0', NULL, NULL, 0, 0)",
            [],
        );
        assert!(inserted.is_ok(), "{inserted:?}");
    }
}
