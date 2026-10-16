//! The paged read behind the list view. The page query and the count are built from one clause, so the
//! count is the number of tasks the page is drawn from. Both run in one read transaction, so a write
//! between them can't make the total disagree with the rows.
//!
//! Pages are keyset pages. Each starts after the sort value and id of the last row of the one before, so
//! a row moved or inserted before the cursor doesn't shift the pages after it.
//!
//! Sorting is in SQL, so a page is the right slice of the whole sorted list, not of the rows loaded so far.
//! Every sort is ordered by its value and then by the task id, so rows that share a value are never split
//! unevenly across pages. An empty value is `NULL`, and the ordering puts those rows last in both
//! directions.

use looji_domain::{
    DomainError, ListSort, PAGE_SIZE, PageCursor, Priority, ProjectId, RepoResult, SortColumn,
    SortDirection, SortValue, TagMatch, Task, TaskPage, TaskQuery, TaskRecord,
};
use rusqlite::{Connection, Row, functions::FunctionFlags, params_from_iter, types::Value};

use super::task::{TASK_COLUMNS, load_tags, read_task};
use super::{id_text, query_all, storage};
use crate::SqliteStore;

/// Registers `looji_contains(text, term)`: the same case-insensitive test as the in-memory matcher, so
/// the database filters exactly the tasks the board would show. Every connection needs it.
pub(crate) fn register_functions(connection: &Connection) -> rusqlite::Result<()> {
    connection.create_scalar_function(
        "looji_contains",
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |context| {
            let text: String = context.get(0)?;
            let term: String = context.get(1)?;
            Ok(looji_domain::contains_ignoring_case(&text, &term))
        },
    )
}

/// `count` placeholders, comma separated.
fn marks(count: usize) -> String {
    vec!["?"; count].join(", ")
}

/// The database expression a sort orders by. Each one is fixed here, so no text from the user or from
/// storage is ever part of the SQL. Priority and status are ranked, so they sort in their own order rather
/// than alphabetically. An empty value is `NULL`.
fn sort_expression(column: SortColumn) -> &'static str {
    match column {
        SortColumn::Manual => "tasks.position",
        SortColumn::Title => "tasks.title",
        SortColumn::Status => {
            "CASE tasks.status WHEN 'backlog' THEN 0 WHEN 'todo' THEN 1 \
             WHEN 'in_progress' THEN 2 WHEN 'in_review' THEN 3 WHEN 'done' THEN 4 END"
        }
        SortColumn::Priority => {
            "CASE tasks.priority WHEN 'low' THEN 1 WHEN 'medium' THEN 2 WHEN 'high' THEN 3 END"
        }
        SortColumn::Assignee => {
            "(SELECT team_members.name FROM team_members WHERE team_members.id = tasks.assignee_id)"
        }
        // A task with several tags sorts by the first of them, alphabetically.
        SortColumn::Tags => {
            "(SELECT MIN(task_tags.tag) FROM task_tags WHERE task_tags.task_id = tasks.id)"
        }
        SortColumn::DueDate => "tasks.due_on",
        SortColumn::Estimate => "tasks.estimate_minutes",
    }
}

/// The WHERE clause for a query in a project, and the values its placeholders take, in order. Each
/// condition mirrors one in [`TaskQuery::matches`].
fn query_clause(project: ProjectId, query: &TaskQuery) -> (String, Vec<Value>) {
    let mut sql = String::from("tasks.project_id = ?");
    let mut values = vec![Value::Text(id_text(project.as_uuid()))];

    // A project with no visible statuses shows nothing.
    if query.statuses.is_empty() {
        sql.push_str(" AND 0");
    } else {
        sql.push_str(&format!(
            " AND tasks.status IN ({})",
            marks(query.statuses.len())
        ));
        values.extend(
            query
                .statuses
                .iter()
                .map(|status| Value::Text(status.as_str().to_string())),
        );
    }

    let filter = &query.filter;
    if !filter.statuses.is_empty() {
        sql.push_str(&format!(
            " AND tasks.status IN ({})",
            marks(filter.statuses.len())
        ));
        values.extend(
            filter
                .statuses
                .iter()
                .map(|status| Value::Text(status.as_str().to_string())),
        );
    }

    if !filter.priorities.is_empty() {
        let named: Vec<Priority> = filter.priorities.iter().flatten().copied().collect();
        let no_priority = filter.priorities.contains(&None);
        sql.push_str(" AND (");
        if !named.is_empty() {
            sql.push_str(&format!("tasks.priority IN ({})", marks(named.len())));
            values.extend(named.iter().map(|p| Value::Text(p.as_str().to_string())));
        }
        if no_priority {
            if !named.is_empty() {
                sql.push_str(" OR ");
            }
            sql.push_str("tasks.priority IS NULL");
        }
        sql.push(')');
    }

    if !filter.assignees.is_empty() {
        let named: Vec<String> = filter
            .assignees
            .iter()
            .flatten()
            .map(|id| id_text(id.as_uuid()))
            .collect();
        let unassigned = filter.assignees.contains(&None);
        sql.push_str(" AND (");
        if !named.is_empty() {
            sql.push_str(&format!("tasks.assignee_id IN ({})", marks(named.len())));
            values.extend(named.into_iter().map(Value::Text));
        }
        if unassigned {
            if filter.assignees.iter().any(Option::is_some) {
                sql.push_str(" OR ");
            }
            sql.push_str("tasks.assignee_id IS NULL");
        }
        sql.push(')');
    }

    const HAS_TAG: &str = "EXISTS (SELECT 1 FROM task_tags WHERE task_tags.task_id = tasks.id";
    if !filter.tags.is_empty() {
        match filter.tag_match {
            TagMatch::Any => {
                sql.push_str(&format!(
                    " AND {HAS_TAG} AND task_tags.tag IN ({}))",
                    marks(filter.tags.len())
                ));
                values.extend(filter.tags.iter().map(|tag| Value::Text(tag.clone())));
            }
            TagMatch::All => {
                for tag in &filter.tags {
                    sql.push_str(&format!(" AND {HAS_TAG} AND task_tags.tag = ?)"));
                    values.push(Value::Text(tag.clone()));
                }
            }
        }
    }

    // Each search term must match the title, the description, or a tag, as in `term_matches`.
    for term in &query.terms {
        sql.push_str(&format!(
            " AND (looji_contains(tasks.title, ?) OR looji_contains(tasks.description, ?) \
             OR {HAS_TAG} AND looji_contains(task_tags.tag, ?)))"
        ));
        values.extend([term, term, term].map(|term| Value::Text(term.clone())));
    }

    (sql, values)
}

/// A sort value as a database value, for a cursor's placeholder.
fn value_of(value: &SortValue) -> Value {
    match value {
        SortValue::Text(text) => Value::Text(text.clone()),
        SortValue::Integer(number) => Value::Integer(*number),
    }
}

/// The database value a stored sort key reads as. Only text and whole numbers are sort values, and a `NULL`
/// means the task has no value for the field.
fn sort_value_of(value: Value) -> Option<SortValue> {
    match value {
        Value::Text(text) => Some(SortValue::Text(text)),
        Value::Integer(number) => Some(SortValue::Integer(number)),
        _ => None,
    }
}

/// A task row, with the sort value the page was ordered by, which the next cursor needs.
fn read_with_key(row: &Row<'_>) -> RepoResult<(TaskRecord, Option<SortValue>)> {
    let record = read_task(row)?;
    let key: Value = row.get(12).map_err(storage)?;
    Ok((record, sort_value_of(key)))
}

/// The statement for one page: the rows after the cursor, in the query's order, one more than the page
/// holds, so the extra row says whether another page follows.
fn page_statement(
    project: ProjectId,
    query: &TaskQuery,
    after: Option<&PageCursor>,
    limit: usize,
) -> (String, Vec<Value>) {
    let (clause, mut values) = query_clause(project, query);
    let expression = sort_expression(query.sort.column);
    let mut sql =
        format!("SELECT {TASK_COLUMNS}, {expression} AS sort_key FROM tasks WHERE {clause}");

    if query.sort.column == SortColumn::Manual {
        if let Some(cursor) = after {
            sql.push_str(" AND (tasks.position, tasks.id) > (?, ?)");
            values.push(cursor.value.as_ref().map_or(Value::Null, value_of));
            values.push(Value::Text(id_text(cursor.id.as_uuid())));
        }
        sql.push_str(" ORDER BY tasks.position COLLATE BINARY, tasks.id");
    } else {
        let direction = sort_direction_sql(query.sort);
        if let Some(cursor) = after {
            match &cursor.value {
                // The page ended in the empty values, which all come last, so only later ids remain.
                None => {
                    sql.push_str(&format!(" AND ({expression}) IS NULL AND tasks.id > ?"));
                    values.push(Value::Text(id_text(cursor.id.as_uuid())));
                }
                // Everything after a value is either a larger (or, descending, smaller) value, an equal value
                // with a later id, or an empty value.
                Some(value) => {
                    let after_value = match query.sort.direction {
                        SortDirection::Ascending => ">",
                        SortDirection::Descending => "<",
                    };
                    sql.push_str(&format!(
                        " AND (({expression}) IS NULL OR ({expression}) {after_value} ? \
                         OR (({expression}) = ? AND tasks.id > ?))"
                    ));
                    values.push(value_of(value));
                    values.push(value_of(value));
                    values.push(Value::Text(id_text(cursor.id.as_uuid())));
                }
            }
        }
        // Empty values come last in both directions, then the values in order, then the id to break ties.
        sql.push_str(&format!(
            " ORDER BY ({expression}) IS NULL, ({expression}) {direction}, tasks.id"
        ));
    }
    sql.push_str(" LIMIT ?");
    values.push(Value::Integer(i64::try_from(limit + 1).unwrap_or(i64::MAX)));
    (sql, values)
}

fn sort_direction_sql(sort: ListSort) -> &'static str {
    match sort.direction {
        SortDirection::Ascending => "ASC",
        SortDirection::Descending => "DESC",
    }
}

/// One page of tasks, with the total and the cursor for the next page, read in one transaction.
pub(super) fn list_page(
    connection: &Connection,
    project: ProjectId,
    query: &TaskQuery,
    after: Option<&PageCursor>,
    limit: usize,
) -> RepoResult<TaskPage> {
    let transaction = connection.unchecked_transaction().map_err(storage)?;
    let (clause, clause_values) = query_clause(project, query);

    let total: i64 = transaction
        .query_row(
            &format!("SELECT COUNT(*) FROM tasks WHERE {clause}"),
            params_from_iter(clause_values.iter()),
            |row| row.get(0),
        )
        .map_err(storage)?;

    let (sql, values) = page_statement(project, query, after, limit);
    let mut rows = query_all(
        &transaction,
        &sql,
        params_from_iter(values.iter()),
        read_with_key,
    )?;
    let has_next = rows.len() > limit;
    rows.truncate(limit);
    let next = if has_next {
        rows.last().map(|(record, value)| PageCursor {
            value: value.clone(),
            id: record.id,
        })
    } else {
        None
    };
    let mut records: Vec<TaskRecord> = rows.into_iter().map(|(record, _)| record).collect();

    if !records.is_empty() {
        let keys: Vec<String> = records
            .iter()
            .map(|record| id_text(record.id.as_uuid()))
            .collect();
        let mut tags = load_tags(
            &transaction,
            &format!(
                "SELECT task_id, tag FROM task_tags WHERE task_id IN ({}) ORDER BY tag",
                marks(keys.len())
            ),
            params_from_iter(keys.iter()),
        )?;
        for record in &mut records {
            record.tags = tags
                .remove(&id_text(record.id.as_uuid()))
                .unwrap_or_default();
        }
    }
    transaction.commit().map_err(storage)?;

    let tasks = records
        .into_iter()
        .map(|record| Task::try_from(record).map_err(DomainError::into))
        .collect::<RepoResult<Vec<Task>>>()?;
    Ok(TaskPage {
        tasks,
        has_next,
        next,
        total: u64::try_from(total).unwrap_or(0),
    })
}

impl SqliteStore {
    /// The query plan for a page of the list, one line per step. A diagnostic: it checks how a page walks
    /// the position index, and that the manual order doesn't sort the rows itself.
    pub fn list_page_query_plan(
        &self,
        project: ProjectId,
        query: &TaskQuery,
        after: Option<&PageCursor>,
    ) -> RepoResult<Vec<String>> {
        let (sql, values) = page_statement(project, query, after, PAGE_SIZE);
        let mut statement = self
            .connection
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .map_err(storage)?;
        let rows = statement
            .query_map(params_from_iter(values.iter()), |row| {
                row.get::<_, String>(3)
            })
            .map_err(storage)?;
        rows.collect::<rusqlite::Result<Vec<String>>>()
            .map_err(storage)
    }
}
