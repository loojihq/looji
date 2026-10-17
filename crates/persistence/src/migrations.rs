//! Versioned schema migrations, recorded in the `schema_migrations` table.

use rusqlite::{Connection, TransactionBehavior, params};

use crate::{Error, Result};

/// One schema change. Once a migration has shipped it is never edited; add a new one instead.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

const CREATE_SCHEMA_MIGRATIONS: &str = "CREATE TABLE IF NOT EXISTS schema_migrations (
    version    INTEGER PRIMARY KEY,
    name       TEXT    NOT NULL,
    applied_at TEXT    NOT NULL
)";

/// Applies every migration in `migrations` that the database hasn't recorded yet, in order.
///
/// Returns the versions applied by this call, which is empty when the schema is already current.
pub fn migrate(connection: &mut Connection, migrations: &[Migration]) -> Result<Vec<u32>> {
    migrate_with_hook(connection, migrations, |_| Ok(()))
}

/// Like [`migrate`], but calls `before_apply` with the pending versions first.
/// The hook only runs when there is something to apply.
/// The versions that would be applied, after checking that the database and the list agree. Creates the bookkeeping
/// table when it's missing. Nothing else is written.
pub(crate) fn pending_versions(
    connection: &Connection,
    migrations: &[Migration],
) -> Result<Vec<u32>> {
    Ok(plan(connection, migrations)?
        .iter()
        .map(|migration| migration.version)
        .collect())
}

/// Whether the database already has migrations applied, so a pending one is an upgrade and not a first run.
pub(crate) fn has_applied(connection: &Connection) -> Result<bool> {
    connection.execute_batch(CREATE_SCHEMA_MIGRATIONS)?;
    Ok(!applied_versions(connection)?.is_empty())
}

pub(crate) fn migrate_with_hook(
    connection: &mut Connection,
    migrations: &[Migration],
    before_apply: impl FnOnce(&[u32]) -> Result<()>,
) -> Result<Vec<u32>> {
    let pending = plan(connection, migrations)?;
    if pending.is_empty() {
        return Ok(Vec::new());
    }
    let pending_versions: Vec<u32> = pending.iter().map(|migration| migration.version).collect();
    before_apply(&pending_versions)?;
    for migration in pending {
        apply(connection, migration)?;
    }
    Ok(pending_versions)
}

fn plan<'a>(connection: &Connection, migrations: &'a [Migration]) -> Result<Vec<&'a Migration>> {
    validate(migrations)?;
    connection.execute_batch(CREATE_SCHEMA_MIGRATIONS)?;

    let applied = applied_versions(connection)?;
    let latest = migrations.last().map_or(0, |migration| migration.version);

    // Refuse rather than guess: this database was written by a newer looji.
    if let Some(&newest) = applied.last()
        && newest > latest
    {
        return Err(Error::DatabaseAhead {
            version: newest,
            latest,
        });
    }
    for &version in &applied {
        if !migrations
            .iter()
            .any(|migration| migration.version == version)
        {
            return Err(Error::MissingMigration { version });
        }
    }

    let pending: Vec<&Migration> = migrations
        .iter()
        .filter(|migration| !applied.contains(&migration.version))
        .collect();
    Ok(pending)
}

/// Checks that versions are strictly increasing, so the order is unambiguous.
fn validate(migrations: &[Migration]) -> Result<()> {
    for pair in migrations.windows(2) {
        let (previous, next) = (pair[0].version, pair[1].version);
        if next == previous {
            return Err(Error::InvalidMigrations(format!(
                "version {next} is listed more than once"
            )));
        }
        if next < previous {
            return Err(Error::InvalidMigrations(format!(
                "version {next} is listed after version {previous}; versions must ascend"
            )));
        }
    }
    Ok(())
}

/// Migrations that rebuild a table other tables reference with ON DELETE CASCADE. Dropping that table
/// with foreign keys on would delete those rows, so they're off while these run. The foreign key check
/// still runs before the migration commits.
const REBUILDS_WITHOUT_FOREIGN_KEYS: &[u32] = &[28];

/// Runs one migration and its bookkeeping row in a single transaction.
/// Any failure drops the transaction, which rolls back the migration's changes.
fn apply(connection: &mut Connection, migration: &Migration) -> Result<()> {
    if !REBUILDS_WITHOUT_FOREIGN_KEYS.contains(&migration.version) {
        return apply_in_transaction(connection, migration);
    }
    // Foreign keys can't change inside a transaction, so they're switched here, around it.
    connection
        .pragma_update(None, "foreign_keys", false)
        .map_err(|err| failed(migration, err))?;
    let result = apply_in_transaction(connection, migration);
    // Back on even when the migration failed, so the connection is left as it was found.
    let restored = connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(|err| failed(migration, err));
    result.and(restored)
}

fn apply_in_transaction(connection: &mut Connection, migration: &Migration) -> Result<()> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|err| failed(migration, err))?;

    transaction
        .execute_batch(migration.sql)
        .map_err(|err| failed(migration, err))?;

    // Catches rows that reference missing parents. This is the check that
    // makes it safe to rebuild tables while foreign keys are enforced.
    let has_violations = transaction
        .prepare("PRAGMA foreign_key_check")
        .and_then(|mut statement| statement.exists([]))
        .map_err(|err| failed(migration, err))?;
    if has_violations {
        return Err(failed(
            migration,
            "the migration left foreign key violations",
        ));
    }

    transaction
        .execute(
            "INSERT INTO schema_migrations (version, name, applied_at)
             VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![migration.version, migration.name],
        )
        .map_err(|err| failed(migration, err))?;

    transaction.commit().map_err(|err| failed(migration, err))
}

fn failed(migration: &Migration, reason: impl ToString) -> Error {
    Error::MigrationFailed {
        version: migration.version,
        name: migration.name.to_string(),
        reason: reason.to_string(),
    }
}

fn applied_versions(connection: &Connection) -> Result<Vec<u32>> {
    let mut statement =
        connection.prepare("SELECT version FROM schema_migrations ORDER BY version")?;
    let versions = statement
        .query_map([], |row| row.get::<_, u32>(0))?
        .collect::<rusqlite::Result<Vec<u32>>>()?;
    Ok(versions)
}
