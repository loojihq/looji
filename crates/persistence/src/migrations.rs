//! Versioned schema migrations, tracked with SQLite's `user_version`.

use rusqlite::Connection;

use crate::Result;

/// Ordered schema changes. Entry `n` brings the schema from version `n` to `n + 1`.
/// Append new entries; never edit one that has shipped.
const MIGRATIONS: &[&str] = &[];

pub(crate) fn apply(connection: &mut Connection) -> Result<()> {
    let current: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;

    for (index, sql) in MIGRATIONS.iter().enumerate() {
        let version = index as i64 + 1;
        if version <= current {
            continue;
        }

        let transaction = connection.transaction()?;
        transaction.execute_batch(sql)?;
        transaction.pragma_update(None, "user_version", version)?;
        transaction.commit()?;
    }

    Ok(())
}
