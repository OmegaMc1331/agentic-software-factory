use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::{Connection, OpenFlags};

use crate::{DbError, Result};

static BACKUP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn before_upgrade(path: &Path, latest: usize) -> Result<()> {
    if !path.is_file() || std::fs::metadata(path)?.len() == 0 {
        return Ok(());
    }
    let source = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    source.busy_timeout(std::time::Duration::from_secs(5))?;
    let has_versions: bool = source.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_migrations')", [], |row| row.get(0))?;
    let versions: Vec<usize> = if has_versions {
        source
            .prepare("SELECT version FROM schema_migrations ORDER BY version")?
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    if versions
        .iter()
        .enumerate()
        .any(|(index, version)| *version != index + 1)
        || versions.last().is_some_and(|version| *version > latest)
    {
        return Err(DbError::Corrupt(
            "unsupported or non-contiguous migration history; original database retained".into(),
        ));
    }
    if versions.len() == latest {
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| DbError::Corrupt("database path has no parent".into()))?;
    let backup_dir = parent.join("backups");
    if let Ok(meta) = std::fs::symlink_metadata(&backup_dir) {
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(DbError::Corrupt(
                "backup location must be a real directory".into(),
            ));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(DbError::Corrupt(
                    "backup location must not be a reparse point".into(),
                ));
            }
        }
    }
    std::fs::create_dir_all(&backup_dir)?;
    let filename = format!(
        "db-before-v{latest}-{}-{}-{}.sqlite3",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        std::process::id(),
        BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let destination = backup_dir.join(filename);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)?;
    source.backup("main", &destination, None::<fn(rusqlite::backup::Progress)>)?;
    let verified = Connection::open_with_flags(&destination, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    let integrity: String = verified.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(DbError::Corrupt(
            "migration backup failed verification; upgrade refused".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_backup_precedes_upgrade_and_preserves_old_rows() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("db.sqlite3");
        let source = Connection::open(&path).unwrap();
        source.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, applied_at TEXT); INSERT INTO schema_migrations VALUES(1,'old'); CREATE TABLE proof(value TEXT); INSERT INTO proof VALUES('preserved');").unwrap();
        before_upgrade(&path, 2).unwrap();
        let backup = std::fs::read_dir(dir.path().join("backups"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let snapshot = Connection::open(backup).unwrap();
        assert_eq!(
            snapshot
                .query_row("SELECT value FROM proof", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "preserved"
        );
        assert_eq!(
            snapshot
                .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            1
        );
    }

    #[test]
    fn migration_batch_failure_rolls_back_all_pending_steps() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE proof(value TEXT); INSERT INTO proof VALUES('preserved');",
        )
        .unwrap();
        assert!(crate::migrate_schemas(
            &mut conn,
            &["CREATE TABLE pending(value TEXT);", "INVALID SQL"]
        )
        .is_err());
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('pending','schema_migrations')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(
            conn.query_row("SELECT value FROM proof", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "preserved"
        );
    }
}
