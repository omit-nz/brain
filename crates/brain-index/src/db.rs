//! The one place a connection to the index opens.

use std::path::Path;

use anyhow::{Context, bail};
use brain_platform::fs::{Privacy, create_dir_private, write_atomic};
use rusqlite::{Connection, OptionalExtension};

/// Every migration in order; the schema version is how many have run.
const MIGRATIONS: &[&str] = &[include_str!("../migrations/0001_notes.sql")];

/// Opens the index at `path`, creating it owner-only, and brings its schema up to date. SQLite
/// creates the WAL and shared-memory files with the database file's mode, so they stay private
/// too. A database from a newer brain is refused rather than read wrong.
pub fn open(path: &Path) -> anyhow::Result<Connection> {
    if !path.exists() {
        let (dir, name) = match (path.parent(), path.file_name()) {
            (Some(dir), Some(name)) => (dir, name),
            _ => bail!("{} is not a file path", path.display()),
        };
        create_dir_private(dir)?;
        write_atomic(dir, Path::new(name), b"", Privacy::Private)?;
    }
    let mut conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&mut conn).with_context(|| format!("migrating {}", path.display()))?;
    Ok(conn)
}

fn migrate(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn.transaction()?;
    let has_meta: bool = tx.query_row(
        "SELECT count(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'meta'",
        [],
        |r| r.get(0),
    )?;
    let version: usize = if has_meta {
        tx.query_row(
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
    if version > MIGRATIONS.len() {
        bail!(
            "this index has schema version {version}, newer than this brain's {}; update brain",
            MIGRATIONS.len()
        );
    }
    for sql in &MIGRATIONS[version..] {
        tx.execute_batch(sql)?;
    }
    tx.execute(
        "INSERT INTO meta(key, value) VALUES ('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [MIGRATIONS.len().to_string()],
    )?;
    tx.commit()?;
    Ok(())
}
