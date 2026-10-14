//! Repository implementations for [`SqliteStore`](crate::SqliteStore).
//!
//! All SQL lives here. Each aggregate reads rows into a domain `*Record`, and the
//! record is validated on the way into an entity, so a bad row fails loudly.

mod column;
mod project;
pub(crate) mod settings;
mod task;
pub(crate) mod task_history;
pub(crate) mod task_move;
mod team;
mod workspace;

use rusqlite::{Connection, Row, types::FromSql};
use uuid::Uuid;

use looji_domain::{DomainError, Position, RepoError, RepoResult, Status, Timestamp};

pub(super) fn storage(err: rusqlite::Error) -> RepoError {
    RepoError::Storage(err.to_string())
}

/// Runs a query and reads every row with `read`, failing on the first bad row.
fn query_all<T>(
    connection: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
    read: fn(&Row<'_>) -> RepoResult<T>,
) -> RepoResult<Vec<T>> {
    let mut statement = connection.prepare(sql).map_err(storage)?;
    let rows = statement
        .query_map(params, |row| Ok(read(row)))
        .map_err(storage)?;
    rows.map(|row| row.map_err(storage).and_then(|result| result))
        .collect()
}

/// Like [`query_all`], for queries keyed by primary key.
fn query_one<T>(
    connection: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
    read: fn(&Row<'_>) -> RepoResult<T>,
) -> RepoResult<Option<T>> {
    Ok(query_all(connection, sql, params, read)?.pop())
}

fn execute(connection: &Connection, sql: &str, params: impl rusqlite::Params) -> RepoResult<()> {
    connection.execute(sql, params).map_err(storage)?;
    Ok(())
}

fn get<T: FromSql>(row: &Row<'_>, index: usize) -> RepoResult<T> {
    row.get(index).map_err(storage)
}

fn read_id<T>(row: &Row<'_>, index: usize, make: fn(Uuid) -> T) -> RepoResult<T> {
    let text: String = get(row, index)?;
    let uuid = Uuid::parse_str(&text).map_err(|_| DomainError::InvalidId(text.clone()))?;
    Ok(make(uuid))
}

fn read_optional_id<T>(row: &Row<'_>, index: usize, make: fn(Uuid) -> T) -> RepoResult<Option<T>> {
    let text: Option<String> = get(row, index)?;
    text.map(|text| {
        Uuid::parse_str(&text)
            .map(make)
            .map_err(|_| DomainError::InvalidId(text).into())
    })
    .transpose()
}

fn read_timestamp(row: &Row<'_>, index: usize) -> RepoResult<Timestamp> {
    Ok(Timestamp::from_unix_millis(get(row, index)?))
}

fn read_position(row: &Row<'_>, index: usize) -> RepoResult<Position> {
    let key: String = get(row, index)?;
    Ok(Position::new(key)?)
}

fn read_status(row: &Row<'_>, index: usize) -> RepoResult<Status> {
    let text: String = get(row, index)?;
    Ok(text.parse()?)
}

/// Stored form of a UUID: hyphenated text.
fn id_text(id: Uuid) -> String {
    id.to_string()
}
