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
    Migration {
        version: 7,
        name: "task_activity",
        sql: include_str!("sql/0007_task_activity.sql"),
    },
    Migration {
        version: 8,
        name: "manual_order",
        sql: include_str!("sql/0008_manual_order.sql"),
    },
    Migration {
        version: 9,
        name: "task_fields",
        sql: include_str!("sql/0009_task_fields.sql"),
    },
    Migration {
        version: 10,
        name: "task_events",
        sql: include_str!("sql/0010_task_events.sql"),
    },
    Migration {
        version: 11,
        name: "project_team_members",
        sql: include_str!("sql/0011_team_members.sql"),
    },
    Migration {
        version: 12,
        name: "normalised_tags",
        sql: include_str!("sql/0012_normalise_tags.sql"),
    },
    Migration {
        version: 13,
        name: "project_filters",
        sql: include_str!("sql/0013_project_filters.sql"),
    },
    Migration {
        version: 14,
        name: "task_list_index",
        sql: include_str!("sql/0014_task_list_index.sql"),
    },
    Migration {
        version: 15,
        name: "project_list_view",
        sql: include_str!("sql/0015_project_list_view.sql"),
    },
    Migration {
        version: 16,
        name: "task_start_date",
        sql: include_str!("sql/0016_task_start_date.sql"),
    },
    Migration {
        version: 17,
        name: "project_view_state",
        sql: include_str!("sql/0017_project_view_state.sql"),
    },
    Migration {
        version: 18,
        name: "project_working_schedule",
        sql: include_str!("sql/0018_project_working_schedule.sql"),
    },
    Migration {
        version: 19,
        name: "plan_drafts",
        sql: include_str!("sql/0019_plan_drafts.sql"),
    },
    Migration {
        version: 20,
        name: "plan_draft_review",
        sql: include_str!("sql/0020_plan_draft_review.sql"),
    },
    Migration {
        version: 21,
        name: "plan_draft_refinement",
        sql: include_str!("sql/0021_plan_draft_refinement.sql"),
    },
    Migration {
        version: 22,
        name: "plan_draft_publish",
        sql: include_str!("sql/0022_plan_draft_publish.sql"),
    },
    Migration {
        version: 23,
        name: "project_repository",
        sql: include_str!("sql/0023_project_repository.sql"),
    },
    Migration {
        version: 24,
        name: "repo_index",
        sql: include_str!("sql/0024_repo_index.sql"),
    },
    Migration {
        version: 25,
        name: "task_chat_and_notes",
        sql: include_str!("sql/0025_task_chat_and_notes.sql"),
    },
    Migration {
        version: 26,
        name: "chat_reply_state",
        sql: include_str!("sql/0026_chat_reply_state.sql"),
    },
    Migration {
        version: 27,
        name: "team_per_workspace",
        sql: include_str!("sql/0027_team_per_workspace.sql"),
    },
    Migration {
        version: 28,
        name: "backlog_column",
        sql: include_str!("sql/0028_backlog_column.sql"),
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
    fn the_backlog_rebuild_keeps_the_history_that_references_tasks() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        // Up to the team migration: the version before the Backlog column.
        migrate(&mut connection, &MIGRATIONS[..27]).unwrap();
        connection
            .execute_batch(
                "INSERT INTO workspaces (id, name, created_at, updated_at)
                     VALUES ('w', 'Work', 0, 0);
                 INSERT INTO projects (id, workspace_id, name, created_at, updated_at)
                     VALUES ('p', 'w', 'Website', 0, 0);
                 INSERT INTO tasks (id, project_id, title, status, position, created_at, updated_at)
                     VALUES ('t', 'p', 'Write copy', 'done', 'a0', 0, 0);
                 INSERT INTO task_activity (id, task_id, kind, from_status, to_status, created_at)
                     VALUES ('a', 't', 'status_changed', 'todo', 'done', 0);
                 INSERT INTO task_events (id, task_id, kind, payload, created_at)
                     VALUES ('e', 't', 'created', '[]', 0);",
            )
            .unwrap();

        migrate(&mut connection, MIGRATIONS).unwrap();

        let count = |sql: &str| -> i64 { connection.query_row(sql, [], |row| row.get(0)).unwrap() };
        assert_eq!(count("SELECT COUNT(*) FROM tasks"), 1);
        assert_eq!(
            count("SELECT COUNT(*) FROM task_activity"),
            1,
            "the rebuild must not cascade into activity"
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM task_events"),
            1,
            "the rebuild must not cascade into events"
        );
        assert_eq!(
            count("SELECT show_backlog FROM projects WHERE id = 'p'"),
            1,
            "existing projects must default to showing Backlog"
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

    #[test]
    fn tasks_from_before_the_history_keep_their_status_and_start_with_no_history() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        migrate(&mut connection, &MIGRATIONS[..6]).unwrap();
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

        let status: String = connection
            .query_row("SELECT status FROM tasks WHERE id = 't'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(status, "in_progress");
        let history: i64 = connection
            .query_row("SELECT COUNT(*) FROM task_activity", [], |row| row.get(0))
            .unwrap();
        assert_eq!(history, 0);
    }

    #[test]
    fn existing_tasks_are_renumbered_in_their_current_order() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        migrate(&mut connection, &MIGRATIONS[..7]).unwrap();
        // Keys out of insertion order, and a key ending in `0`, as older builds produced.
        connection
            .execute_batch(
                "INSERT INTO workspaces (id, name, icon_file, created_at, updated_at)
                     VALUES ('w', 'Work', NULL, 0, 0);
                 INSERT INTO projects (id, workspace_id, name, description, created_at, updated_at)
                     VALUES ('p', 'w', 'Website', NULL, 0, 0);
                 INSERT INTO tasks (id, project_id, title, description, status, position,
                                    estimate_minutes, assignee_id, created_at, updated_at)
                     VALUES ('third', 'p', 'C', '', 'todo', 'zz', NULL, NULL, 0, 0),
                            ('first', 'p', 'A', '', 'todo', '00000000000000000100', NULL, NULL, 0, 0),
                            ('second', 'p', 'B', '', 'todo', '1', NULL, NULL, 0, 0),
                            ('done', 'p', 'D', '', 'done', 'b', NULL, NULL, 0, 0);",
            )
            .unwrap();

        migrate(&mut connection, MIGRATIONS).unwrap();

        let mut statement = connection
            .prepare(
                "SELECT id, position FROM tasks WHERE status = 'todo'
                 ORDER BY position COLLATE BINARY, id",
            )
            .unwrap();
        let order: Vec<(String, String)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let ids: Vec<&str> = order.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["first", "second", "third"]);
        assert!(order.iter().all(|(_, key)| !key.ends_with('0')));
    }

    #[test]
    fn existing_tasks_upgrade_with_no_priority_due_date_or_tags() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        migrate(&mut connection, &MIGRATIONS[..8]).unwrap();
        connection
            .execute_batch(
                "INSERT INTO workspaces (id, name, icon_file, created_at, updated_at)
                     VALUES ('w', 'Work', NULL, 0, 0);
                 INSERT INTO projects (id, workspace_id, name, description, created_at, updated_at)
                     VALUES ('p', 'w', 'Website', NULL, 0, 0);
                 INSERT INTO tasks (id, project_id, title, description, status, position,
                                    estimate_minutes, assignee_id, created_at, updated_at)
                     VALUES ('t', 'p', 'Write copy', '', 'todo', 'a00000001', 90, NULL, 0, 0);",
            )
            .unwrap();

        migrate(&mut connection, MIGRATIONS).unwrap();

        let (title, estimate, priority, due): (
            String,
            Option<i64>,
            Option<String>,
            Option<String>,
        ) = connection
            .query_row(
                "SELECT title, estimate_minutes, priority, due_on FROM tasks WHERE id = 't'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(title, "Write copy");
        assert_eq!(estimate, Some(90));
        assert_eq!(priority, None);
        assert_eq!(due, None);
        let tags: i64 = connection
            .query_row("SELECT COUNT(*) FROM task_tags", [], |row| row.get(0))
            .unwrap();
        assert_eq!(tags, 0);
    }

    #[test]
    fn tasks_assigned_to_the_old_workspace_members_upgrade_unassigned_and_keep_their_data() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        migrate(&mut connection, &MIGRATIONS[..10]).unwrap();
        connection
            .execute_batch(
                "INSERT INTO workspaces (id, name, icon_file, created_at, updated_at)
                     VALUES ('w', 'Work', NULL, 0, 0);
                 INSERT INTO projects (id, workspace_id, name, description, created_at, updated_at)
                     VALUES ('p', 'w', 'Website', NULL, 0, 0);
                 INSERT INTO team_members (id, workspace_id, name, created_at, updated_at)
                     VALUES ('m', 'w', 'Dana', 0, 0);
                 INSERT INTO tasks (id, project_id, title, description, status, position,
                                    estimate_minutes, assignee_id, created_at, updated_at)
                     VALUES ('t', 'p', 'Write copy', '', 'todo', 'a0', 90, 'm', 0, 0);",
            )
            .unwrap();

        migrate(&mut connection, MIGRATIONS).unwrap();

        let (title, estimate, assignee): (String, Option<i64>, Option<String>) = connection
            .query_row(
                "SELECT title, estimate_minutes, assignee_id FROM tasks WHERE id = 't'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(title, "Write copy");
        assert_eq!(estimate, Some(90));
        assert_eq!(assignee, None, "existing tasks start unassigned");
        let members: i64 = connection
            .query_row("SELECT COUNT(*) FROM team_members", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            members, 0,
            "the workspace-scoped rows have no project to move to"
        );
    }

    #[test]
    fn a_member_colour_outside_the_palette_is_refused_by_storage() {
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
            "INSERT INTO team_members (id, project_id, name, colour, created_at, updated_at)
             VALUES ('m', 'p', 'Dana', 6, 0, 0)",
            [],
        );
        assert!(inserted.is_err());
    }

    #[test]
    fn existing_tags_are_normalised_and_duplicates_merge() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        migrate(&mut connection, &MIGRATIONS[..11]).unwrap();
        connection
            .execute_batch(
                "INSERT INTO workspaces (id, name, icon_file, created_at, updated_at)
                     VALUES ('w', 'Work', NULL, 0, 0);
                 INSERT INTO projects (id, workspace_id, name, description, created_at, updated_at)
                     VALUES ('p', 'w', 'Website', NULL, 0, 0);
                 INSERT INTO tasks (id, project_id, title, description, status, position,
                                    estimate_minutes, assignee_id, created_at, updated_at)
                     VALUES ('t', 'p', 'Write copy', '', 'todo', 'a0', NULL, NULL, 0, 0);
                 INSERT INTO task_tags (task_id, tag) VALUES
                     ('t', 'Web'), ('t', ' web '), ('t', 'web'),
                     ('t', 'Design  Doc'), ('t', 'design doc'), ('t', '   ');",
            )
            .unwrap();

        migrate(&mut connection, MIGRATIONS).unwrap();

        let mut statement = connection
            .prepare("SELECT tag FROM task_tags WHERE task_id = 't' ORDER BY tag")
            .unwrap();
        let tags: Vec<String> = statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(tags, ["design doc", "web"]);
    }

    #[test]
    fn a_database_from_before_the_list_keeps_its_tasks_and_gets_the_position_index_with_the_id() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        migrate(&mut connection, &MIGRATIONS[..13]).unwrap();
        connection
            .execute_batch(
                "INSERT INTO workspaces (id, name, icon_file, created_at, updated_at)
                     VALUES ('w', 'Work', NULL, 0, 0);
                 INSERT INTO projects (id, workspace_id, name, description, created_at, updated_at)
                     VALUES ('p', 'w', 'Website', NULL, 0, 0);
                 INSERT INTO tasks (id, project_id, title, description, status, position,
                                    estimate_minutes, assignee_id, created_at, updated_at)
                     VALUES ('t1', 'p', 'First', '', 'todo', 'a0', NULL, NULL, 0, 0),
                            ('t2', 'p', 'Second', '', 'done', 'a1', NULL, NULL, 0, 0);",
            )
            .unwrap();

        migrate(&mut connection, MIGRATIONS).unwrap();

        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2, "the rebuild keeps every task");
        let sql: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = 'tasks_by_project_position'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            sql.contains("position, id"),
            "the index ends in the id: {sql}"
        );
    }

    #[test]
    fn a_database_from_before_list_settings_keeps_its_tasks_and_gets_the_table() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        migrate(&mut connection, &MIGRATIONS[..14]).unwrap();
        connection
            .execute_batch(
                "INSERT INTO workspaces (id, name, icon_file, created_at, updated_at)
                     VALUES ('w', 'Work', NULL, 0, 0);
                 INSERT INTO projects (id, workspace_id, name, description, created_at, updated_at)
                     VALUES ('p', 'w', 'Website', NULL, 0, 0);
                 INSERT INTO tasks (id, project_id, title, description, status, position,
                                    estimate_minutes, assignee_id, created_at, updated_at)
                     VALUES ('t', 'p', 'Write copy', '', 'todo', 'a0', 30, NULL, 0, 0);",
            )
            .unwrap();

        migrate(&mut connection, MIGRATIONS).unwrap();

        let title: String = connection
            .query_row("SELECT title FROM tasks WHERE id = 't'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(title, "Write copy", "existing tasks are kept");
        let table: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'project_list_view'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table, 1, "the settings table exists");
        let rows: i64 = connection
            .query_row("SELECT COUNT(*) FROM project_list_view", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(rows, 0, "a project with no saved settings has no row");
    }

    #[test]
    fn tasks_from_before_the_start_date_keep_their_data_with_no_start() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        migrate(&mut connection, &MIGRATIONS[..15]).unwrap();
        connection
            .execute_batch(
                "INSERT INTO workspaces (id, name, icon_file, created_at, updated_at)
                     VALUES ('w', 'Work', NULL, 0, 0);
                 INSERT INTO projects (id, workspace_id, name, description, created_at, updated_at)
                     VALUES ('p', 'w', 'Website', NULL, 0, 0);
                 INSERT INTO tasks (id, project_id, title, description, status, position,
                                    estimate_minutes, assignee_id, due_on, created_at, updated_at)
                     VALUES ('t', 'p', 'Write copy', '', 'todo', 'a0', NULL, NULL, '2026-10-16', 0, 0);",
            )
            .unwrap();

        migrate(&mut connection, MIGRATIONS).unwrap();

        let (title, due, start): (String, Option<String>, Option<String>) = connection
            .query_row(
                "SELECT title, due_on, start_on FROM tasks WHERE id = 't'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(title, "Write copy");
        assert_eq!(due.as_deref(), Some("2026-10-16"), "the due date is kept");
        assert_eq!(start, None, "existing tasks have no start");
    }

    #[test]
    fn a_database_from_before_view_state_keeps_its_projects_and_gets_the_table() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        migrate(&mut connection, &MIGRATIONS[..16]).unwrap();
        connection
            .execute_batch(
                "INSERT INTO workspaces (id, name, icon_file, created_at, updated_at)
                     VALUES ('w', 'Work', NULL, 0, 0);
                 INSERT INTO projects (id, workspace_id, name, description, created_at, updated_at)
                     VALUES ('p', 'w', 'Website', NULL, 0, 0);",
            )
            .unwrap();

        migrate(&mut connection, MIGRATIONS).unwrap();

        let name: String = connection
            .query_row("SELECT name FROM projects WHERE id = 'p'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(name, "Website", "the project is kept");
        let rows: i64 = connection
            .query_row("SELECT COUNT(*) FROM project_view_state", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(rows, 0, "a project with no saved view has no row");
    }

    #[test]
    fn a_project_from_before_working_hours_reads_back_as_monday_to_friday_nine_to_five() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        migrate(&mut connection, &MIGRATIONS[..17]).unwrap();
        connection
            .execute_batch(
                "INSERT INTO workspaces (id, name, icon_file, created_at, updated_at)
                     VALUES ('w', 'Work', NULL, 0, 0);
                 INSERT INTO projects (id, workspace_id, name, description, created_at, updated_at)
                     VALUES ('p', 'w', 'Website', NULL, 0, 0);",
            )
            .unwrap();

        migrate(&mut connection, MIGRATIONS).unwrap();

        let (name, start, end, mask): (String, i64, i64, i64) = connection
            .query_row(
                "SELECT name, working_start_min, working_end_min, workdays_mask FROM projects WHERE id = 'p'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(name, "Website", "the project is kept");
        assert_eq!(
            (start, end, mask),
            (540, 1020, 31),
            "Monday to Friday, 09:00 to 17:00"
        );
    }

    #[test]
    fn a_project_from_before_the_repository_connection_keeps_its_data_and_gets_no_path() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        migrate(&mut connection, &MIGRATIONS[..22]).unwrap();
        connection
            .execute_batch(
                "INSERT INTO workspaces (id, name, icon_file, created_at, updated_at)
                     VALUES ('w', 'Work', NULL, 0, 0);
                 INSERT INTO projects (id, workspace_id, name, description, created_at, updated_at)
                     VALUES ('p', 'w', 'Website', NULL, 0, 0);",
            )
            .unwrap();

        migrate(&mut connection, MIGRATIONS).unwrap();

        let name: String = connection
            .query_row("SELECT name FROM projects WHERE id = 'p'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(name, "Website", "the project is kept");
        let rows: i64 = connection
            .query_row("SELECT COUNT(*) FROM project_repository", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(rows, 0, "a project with no folder connected has no row");
    }

    #[test]
    fn a_project_from_before_the_repository_index_keeps_its_data_and_gets_empty_caches() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        migrate(&mut connection, &MIGRATIONS[..23]).unwrap();
        connection
            .execute_batch(
                "INSERT INTO workspaces (id, name, icon_file, created_at, updated_at)
                     VALUES ('w', 'Work', NULL, 0, 0);
                 INSERT INTO projects (id, workspace_id, name, description, created_at, updated_at)
                     VALUES ('p', 'w', 'Website', NULL, 0, 0);",
            )
            .unwrap();

        migrate(&mut connection, MIGRATIONS).unwrap();

        let name: String = connection
            .query_row("SELECT name FROM projects WHERE id = 'p'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(name, "Website", "the project is kept");
        for table in [
            "index_entries",
            "index_files",
            "index_symbols",
            "task_threads",
            "task_messages",
        ] {
            let rows: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(rows, 0, "{table} starts empty");
        }
    }
}
