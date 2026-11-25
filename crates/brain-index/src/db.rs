//! The one place a connection to any of brain's databases opens: the index and the activity
//! store.

use std::path::Path;

use anyhow::{Context, bail};
use brain_platform::fs::{create_dir_private, ensure_private_file};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

/// The index's migrations in order; a database's schema version is how many have run.
pub const INDEX_MIGRATIONS: &[&str] = &[include_str!("../migrations/0001_notes.sql")];

/// Opens the database at `path`, making it owner-only whether it is new or not, and brings its
/// schema up to date with `migrations`. SQLite creates the WAL and shared-memory files with the
/// database file's mode, so they stay private too. Processes opening the same database at once
/// share one file and one migration. A database from a newer brain is refused rather than read
/// wrong.
pub fn open(path: &Path, migrations: &[&str]) -> anyhow::Result<Connection> {
    let (dir, name) = match (path.parent(), path.file_name()) {
        (Some(dir), Some(name)) => (dir, name),
        _ => bail!("{} is not a file path", path.display()),
    };
    create_dir_private(dir)?;
    ensure_private_file(dir, Path::new(name))?;
    let mut conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
    conn.pragma_update(None, "busy_timeout", BUSY_MS as i64)?;
    use_wal(&conn)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&mut conn, migrations).with_context(|| format!("migrating {}", path.display()))?;
    Ok(conn)
}

const BUSY_MS: u64 = 5000;

/// Switching to WAL takes an exclusive lock, and SQLite answers a second connection switching at
/// the same moment with SQLITE_BUSY at once rather than waiting. WAL is kept in the file, so that
/// connection only has to try again until the first has switched it.
fn use_wal(conn: &Connection) -> anyhow::Result<()> {
    const STEP_MS: u64 = 10;
    for _ in 0..BUSY_MS / STEP_MS {
        match conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get::<_, String>(0)) {
            Ok(mode) if mode.eq_ignore_ascii_case("wal") => return Ok(()),
            Ok(mode) => bail!("this database cannot use WAL; SQLite kept it in {mode} mode"),
            Err(e) if e.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy) => {
                std::thread::sleep(std::time::Duration::from_millis(STEP_MS));
            }
            Err(e) => return Err(e.into()),
        }
    }
    bail!("this database stayed locked for {BUSY_MS} ms while switching to WAL")
}

/// How many of `migrations` have run on `conn`'s database; one from a newer brain is refused.
fn version(conn: &Connection, migrations: &[&str]) -> anyhow::Result<usize> {
    let has_meta: bool = conn.query_row(
        "SELECT count(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'meta'",
        [],
        |r| r.get(0),
    )?;
    let version: usize = if has_meta {
        conn.query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .map(|v| v.parse())
        .transpose()
        .context("schema_version is not a number")?
        .unwrap_or(0)
    } else {
        0
    };
    if version > migrations.len() {
        bail!(
            "this database has schema version {version}, newer than this brain's {}; update brain",
            migrations.len()
        );
    }
    Ok(version)
}

fn migrate(conn: &mut Connection, migrations: &[&str]) -> anyhow::Result<()> {
    // A database already up to date is only read, so opening it never waits on a writer.
    if version(conn, migrations)? == migrations.len() {
        return Ok(());
    }
    // Immediate: the write lock is taken before the version is read again, so two openers never
    // both read the old version and race to upgrade.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version = version(&tx, migrations)?;
    for sql in &migrations[version..] {
        tx.execute_batch(sql)?;
    }
    tx.execute(
        "INSERT INTO meta(key, value) VALUES ('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [migrations.len().to_string()],
    )?;
    tx.commit()?;
    Ok(())
}
