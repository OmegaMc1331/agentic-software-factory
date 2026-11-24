use crate::{IdeError, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};
use std::path::Path;
use std::sync::Mutex;

pub struct Store {
    connection: Option<Mutex<Connection>>,
}

impl Store {
    pub fn unavailable() -> Self {
        Self { connection: None }
    }
    pub fn open(path: &Path) -> Result<Self> {
        if let Ok(metadata) = std::fs::symlink_metadata(path) {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(IdeError::Forbidden);
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err(IdeError::Forbidden);
                }
            }
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > 2 {
            return Err(IdeError::Storage);
        }
        let tables: usize = connection.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        if version == 0 && tables != 0 {
            return Err(IdeError::Storage);
        }
        connection.execute_batch("PRAGMA journal_mode=WAL; BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS ide_records(project TEXT NOT NULL, kind TEXT NOT NULL, id TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY(project,kind,id)); CREATE TABLE IF NOT EXISTS ide_audit(id INTEGER PRIMARY KEY AUTOINCREMENT, project TEXT NOT NULL, created_at TEXT NOT NULL, action TEXT NOT NULL, resource TEXT NOT NULL); PRAGMA user_version=2; COMMIT;")?;
        Ok(Self {
            connection: Some(Mutex::new(connection)),
        })
    }
    pub fn list<T: DeserializeOwned>(&self, project: &str, kind: &str) -> Result<Vec<T>> {
        let connection = self
            .connection
            .as_ref()
            .ok_or(IdeError::Storage)?
            .lock()
            .map_err(|_| IdeError::Storage)?;
        let mut statement = connection.prepare(
            "SELECT body FROM ide_records WHERE project=? AND kind=? ORDER BY id LIMIT 129",
        )?;
        let rows = statement.query_map(params![project, kind], |row| row.get::<_, String>(0))?;
        let mut values = Vec::new();
        for row in rows {
            values.push(serde_json::from_str(&row?).map_err(|_| IdeError::Storage)?);
        }
        if values.len() > 128 {
            return Err(IdeError::Limit);
        }
        Ok(values)
    }
    pub fn get<T: DeserializeOwned>(&self, project: &str, kind: &str, id: &str) -> Result<T> {
        let body: Option<String> = self
            .connection
            .as_ref()
            .ok_or(IdeError::Storage)?
            .lock()
            .map_err(|_| IdeError::Storage)?
            .query_row(
                "SELECT body FROM ide_records WHERE project=? AND kind=? AND id=?",
                params![project, kind, id],
                |row| row.get(0),
            )
            .optional()?;
        serde_json::from_str(&body.ok_or(IdeError::NotFound)?).map_err(|_| IdeError::Storage)
    }
    pub fn put(&self, project: &str, kind: &str, id: &str, value: &impl Serialize) -> Result<()> {
        self.write(project, kind, id, value, false)
    }
    pub fn insert(
        &self,
        project: &str,
        kind: &str,
        id: &str,
        value: &impl Serialize,
    ) -> Result<()> {
        self.write(project, kind, id, value, true)
    }
    fn write(
        &self,
        project: &str,
        kind: &str,
        id: &str,
        value: &impl Serialize,
        create_only: bool,
    ) -> Result<()> {
        let body = serde_json::to_string(value).map_err(|_| IdeError::Invalid)?;
        if body.len() > 262_144 {
            return Err(IdeError::Limit);
        }
        let mut connection = self
            .connection
            .as_ref()
            .ok_or(IdeError::Storage)?
            .lock()
            .map_err(|_| IdeError::Storage)?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let count: usize = transaction.query_row(
            "SELECT count(*) FROM ide_records WHERE project=? AND kind=?",
            params![project, kind],
            |row| row.get(0),
        )?;
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM ide_records WHERE project=? AND kind=? AND id=?)",
            params![project, kind, id],
            |row| row.get(0),
        )?;
        if exists && create_only {
            return Err(IdeError::Conflict);
        }
        if count >= 128 && !exists {
            return Err(IdeError::Limit);
        }
        transaction.execute("INSERT INTO ide_records VALUES(?,?,?,?) ON CONFLICT(project,kind,id) DO UPDATE SET body=excluded.body",params![project,kind,id,body])?;
        transaction.commit()?;
        Ok(())
    }
    pub fn remove(&self, project: &str, kind: &str, id: &str) -> Result<()> {
        self.connection
            .as_ref()
            .ok_or(IdeError::Storage)?
            .lock()
            .map_err(|_| IdeError::Storage)?
            .execute(
                "DELETE FROM ide_records WHERE project=? AND kind=? AND id=?",
                params![project, kind, id],
            )?;
        Ok(())
    }
    pub fn audit(&self, project: &str, action: &str, resource: &str) -> Result<()> {
        if action.len() > 80 || resource.len() > 512 {
            return Err(IdeError::Invalid);
        }
        let mut connection = self
            .connection
            .as_ref()
            .ok_or(IdeError::Storage)?
            .lock()
            .map_err(|_| IdeError::Storage)?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO ide_audit(project,created_at,action,resource) VALUES(?,?,?,?)",
            params![project, chrono::Utc::now().to_rfc3339(), action, resource],
        )?;
        transaction.execute("DELETE FROM ide_audit WHERE project=? AND id NOT IN (SELECT id FROM ide_audit WHERE project=? ORDER BY id DESC LIMIT 1000)",params![project,project])?;
        transaction.commit()?;
        Ok(())
    }
    pub fn audit_entries(&self, project: &str) -> Result<Vec<serde_json::Value>> {
        let connection = self
            .connection
            .as_ref()
            .ok_or(IdeError::Storage)?
            .lock()
            .map_err(|_| IdeError::Storage)?;
        let mut statement=connection.prepare("SELECT id,created_at,action,resource FROM ide_audit WHERE project=? ORDER BY id DESC LIMIT 100")?;
        let entries=statement.query_map([project],|row|Ok(serde_json::json!({"id":row.get::<_,i64>(0)?,"created_at":row.get::<_,String>(1)?,"action":row.get::<_,String>(2)?,"resource":row.get::<_,String>(3)?})))?;
        Ok(entries.collect::<std::result::Result<Vec<_>, _>>()?)
    }
}
