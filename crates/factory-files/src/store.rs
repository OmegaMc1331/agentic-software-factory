use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::path::Guard;
use crate::types::{nonce, valid_token};
use crate::{native, Draft, DraftEnvelope, FileError, Limits, Recovery, Result};

pub(crate) struct Store {
    root: PathBuf,
    identity: String,
}

impl Store {
    pub fn new(root: &Path, identity: &str) -> Self {
        Self {
            root: root.to_path_buf(),
            identity: identity.to_owned(),
        }
    }

    pub fn connect(&self) -> Result<(Connection, Guard)> {
        let guard = Guard::root(&self.root, &self.identity)?.descend(".factory")?;
        let path = guard.absolute.join("editor-state.sqlite3");
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let candidate = guard.absolute.join(format!("editor-state.sqlite3{suffix}"));
            match std::fs::symlink_metadata(&candidate) {
                Ok(metadata) => {
                    if native::is_link(&metadata) || !metadata.is_file() {
                        return Err(FileError::Storage);
                    }
                    let file = native::open_metadata(&candidate)?;
                    if native::links(&file)? != 1 {
                        return Err(FileError::Storage);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(FileError::Storage),
            }
        }
        let mut connection = Connection::open(&path).map_err(|_| FileError::Storage)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(3))
            .map_err(|_| FileError::Storage)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| FileError::Storage)?;
        let version: i64 = transaction
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(|_| FileError::Storage)?;
        if version == 0 {
            let count: i64 = transaction.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'", [], |r| r.get(0)).map_err(|_| FileError::Storage)?;
            if count != 0 {
                return Err(FileError::Storage);
            }
            transaction.execute_batch("CREATE TABLE drafts(id TEXT PRIMARY KEY, revision INTEGER NOT NULL, document TEXT NOT NULL, bytes INTEGER NOT NULL, dirty INTEGER NOT NULL, updated_at TEXT NOT NULL); CREATE TABLE file_operations(id TEXT PRIMARY KEY, root_id TEXT NOT NULL, path TEXT NOT NULL, kind TEXT NOT NULL, phase TEXT NOT NULL, temporary TEXT NOT NULL, backup TEXT NOT NULL, created_at TEXT NOT NULL); PRAGMA user_version=1;").map_err(|_| FileError::Storage)?;
        } else if version != 1 {
            return Err(FileError::Storage);
        }
        transaction.commit().map_err(|_| FileError::Storage)?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(|_| FileError::Storage)?;
        guard.verify()?;
        Ok((connection, guard))
    }

    pub fn draft(&self, id: &str) -> Result<DraftEnvelope> {
        if !valid_token(id, "d_") {
            return Err(FileError::Invalid);
        }
        let (connection, _guard) = self.connect()?;
        let (revision, document): (u64, String) = connection
            .query_row(
                "SELECT revision,document FROM drafts WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|_| FileError::Storage)?
            .ok_or(FileError::NotFound)?;
        let document: Draft = serde_json::from_str(&document).map_err(|_| FileError::Storage)?;
        if document.schema != 1 || document.id != id {
            return Err(FileError::Storage);
        }
        Ok(DraftEnvelope { revision, document })
    }

    pub fn save_draft(
        &self,
        document: Draft,
        expected: u64,
        limits: &Limits,
    ) -> Result<DraftEnvelope> {
        if !valid_token(&document.id, "d_")
            || document.schema != 1
            || document.content.len() > limits.edit_bytes
            || document.base_content.len() > limits.edit_bytes
            || document.base_revision.len() > 128
            || !["lf", "crlf", "mixed", "none"].contains(&document.newline.as_str())
        {
            return Err(FileError::Invalid);
        }
        let (mut connection, _guard) = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| FileError::Storage)?;
        let previous: Option<u64> = transaction
            .query_row(
                "SELECT revision FROM drafts WHERE id=?1",
                [&document.id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| FileError::Storage)?;
        if previous.unwrap_or(0) != expected {
            return Err(FileError::DraftConflict);
        }
        transaction
            .execute(
                "DELETE FROM drafts WHERE dirty=0 AND updated_at < datetime('now','-7 days')",
                [],
            )
            .map_err(|_| FileError::Storage)?;
        let serialized = serde_json::to_string(&document).map_err(|_| FileError::Invalid)?;
        loop {
            let used: (usize, usize) = transaction
                .query_row(
                    "SELECT count(*),coalesce(sum(bytes),0) FROM drafts WHERE id<>?1",
                    [&document.id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|_| FileError::Storage)?;
            if used.0 < limits.draft_records && used.1 + serialized.len() <= limits.draft_bytes {
                break;
            }
            let removed=transaction.execute("DELETE FROM drafts WHERE id IN (SELECT id FROM drafts WHERE dirty=0 AND id<>?1 ORDER BY updated_at,id LIMIT 1)",[&document.id]).map_err(|_|FileError::Storage)?;
            if removed == 0 {
                return Err(FileError::Limit);
            }
        }
        let revision = expected.checked_add(1).ok_or(FileError::Limit)?;
        transaction.execute("INSERT INTO drafts(id,revision,document,bytes,dirty,updated_at) VALUES(?1,?2,?3,?4,?5,datetime('now')) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,document=excluded.document,bytes=excluded.bytes,dirty=excluded.dirty,updated_at=excluded.updated_at", params![document.id,revision,serialized,serialized.len(),document.dirty]).map_err(|_| FileError::Storage)?;
        transaction.commit().map_err(|_| FileError::Storage)?;
        Ok(DraftEnvelope { revision, document })
    }

    pub fn discard(&self, id: &str, expected: u64) -> Result<()> {
        if !valid_token(id, "d_") {
            return Err(FileError::Invalid);
        }
        let (connection, _guard) = self.connect()?;
        let removed = connection
            .execute(
                "DELETE FROM drafts WHERE id=?1 AND revision=?2",
                params![id, expected],
            )
            .map_err(|_| FileError::Storage)?;
        if removed != 1 {
            return Err(FileError::DraftConflict);
        }
        Ok(())
    }

    pub fn begin(
        &self,
        root_id: &str,
        path: &str,
        kind: &str,
        temporary: &str,
    ) -> Result<Recovery> {
        let (mut connection, _guard) = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| FileError::Storage)?;
        transaction.execute("DELETE FROM file_operations WHERE phase IN ('committed','failed','restored','purged') AND id NOT IN (SELECT id FROM file_operations WHERE phase IN ('committed','failed','restored','purged') ORDER BY created_at DESC,id DESC LIMIT 500)", []).map_err(|_| FileError::Storage)?;
        let pending: usize = transaction.query_row("SELECT count(*) FROM file_operations WHERE phase IN ('prepared','uncertain','deleted')", [], |r| r.get(0)).map_err(|_| FileError::Storage)?;
        if pending >= 128 {
            return Err(FileError::Limit);
        }
        let entry = Recovery {
            id: format!("u_{}", nonce()),
            root_id: root_id.into(),
            path: path.into(),
            kind: kind.into(),
            phase: "prepared".into(),
            temporary: temporary.into(),
            backup: format!("{}.recovery", nonce()),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        transaction.execute("INSERT INTO file_operations(id,root_id,path,kind,phase,temporary,backup,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![entry.id,entry.root_id,entry.path,entry.kind,entry.phase,entry.temporary,entry.backup,entry.created_at]).map_err(|_| FileError::Storage)?;
        transaction.commit().map_err(|_| FileError::Storage)?;
        Ok(entry)
    }

    pub fn finish(&self, id: &str, phase: &str) -> Result<()> {
        let (connection, _guard) = self.connect()?;
        connection
            .execute(
                "UPDATE file_operations SET phase=?2 WHERE id=?1",
                params![id, phase],
            )
            .map_err(|_| FileError::Storage)?;
        Ok(())
    }

    pub fn recovery(&self) -> Result<Vec<Recovery>> {
        let (connection, _guard) = self.connect()?;
        let mut statement = connection.prepare("SELECT id,root_id,path,kind,phase,temporary,backup,created_at FROM file_operations WHERE phase IN ('prepared','uncertain','deleted') ORDER BY created_at,id LIMIT 128").map_err(|_| FileError::Storage)?;
        let entries = statement
            .query_map([], |r| {
                Ok(Recovery {
                    id: r.get(0)?,
                    root_id: r.get(1)?,
                    path: r.get(2)?,
                    kind: r.get(3)?,
                    phase: r.get(4)?,
                    temporary: r.get(5)?,
                    backup: r.get(6)?,
                    created_at: r.get(7)?,
                })
            })
            .map_err(|_| FileError::Storage)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| FileError::Storage)?;
        Ok(entries)
    }
}
