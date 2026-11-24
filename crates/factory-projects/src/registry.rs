use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rand::RngCore;
use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};

use crate::{ProjectError, Result};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProjectId(String);

impl ProjectId {
    pub fn parse(value: &str) -> Result<Self> {
        if value.len() != 34
            || !value.starts_with("p_")
            || !value[2..]
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(ProjectError::InvalidId);
        }
        Ok(Self(value.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    fn new() -> Self {
        let mut bytes = [0; 16];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Self(format!(
            "p_{}",
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ))
    }
}
impl TryFrom<String> for ProjectId {
    type Error = ProjectError;
    fn try_from(value: String) -> Result<Self> {
        Self::parse(&value)
    }
}
impl From<ProjectId> for String {
    fn from(value: ProjectId) -> Self {
        value.0
    }
}
impl std::fmt::Display for ProjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub project_id: ProjectId,
    pub canonical_root: PathBuf,
    pub display_name: String,
    pub created_at: String,
    pub last_opened_at: Option<String>,
    pub registry_schema_version: u32,
    pub root_identity: String,
    pub git_identity: String,
    pub database_identity: Option<String>,
}

pub struct ProjectRegistry {
    connection: Mutex<Connection>,
}

impl ProjectRegistry {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(path).map_err(registry_error)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(registry_error)?;
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(registry_error)?;
        if version > 1 {
            return Err(ProjectError::Registry(
                "unsupported registry schema; upgrade ASF or restore a verified copy".into(),
            ));
        }
        let existing_tables: i64 = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'", [], |row| row.get(0)).map_err(registry_error)?;
        if version == 0 && existing_tables != 0 {
            return Err(ProjectError::Registry(
                "unrecognized database; refusing to turn it into an application registry".into(),
            ));
        }
        if version == 1 {
            let has_projects: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='projects')", [], |row| row.get(0)).map_err(registry_error)?;
            if !has_projects {
                return Err(ProjectError::Registry("registry table is missing; restore a verified copy, no empty replacement created".into()));
            }
        }
        let integrity: String = conn
            .query_row("PRAGMA quick_check", [], |row| row.get(0))
            .map_err(registry_error)?;
        if integrity != "ok" {
            return Err(ProjectError::Registry(
                "integrity check failed; restore a verified registry backup".into(),
            ));
        }
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(registry_error)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(registry_error)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS projects (project_id TEXT PRIMARY KEY, canonical_root TEXT NOT NULL UNIQUE, display_name TEXT NOT NULL, created_at TEXT NOT NULL, last_opened_at TEXT, registry_schema_version INTEGER NOT NULL CHECK (registry_schema_version = 1), root_identity TEXT NOT NULL UNIQUE, git_identity TEXT NOT NULL UNIQUE, database_identity TEXT UNIQUE); PRAGMA user_version=1;").map_err(registry_error)?;
        tx.commit().map_err(registry_error)?;
        let registry = Self {
            connection: Mutex::new(conn),
        };
        registry.list()?;
        Ok(registry)
    }

    pub fn list(&self) -> Result<Vec<ProjectEntry>> {
        let conn = self
            .connection
            .lock()
            .map_err(|_| ProjectError::Registry("registry lock failed".into()))?;
        read_entries(&conn)
    }

    pub fn get(&self, id: &ProjectId) -> Result<ProjectEntry> {
        self.list()?
            .into_iter()
            .find(|entry| &entry.project_id == id)
            .ok_or(ProjectError::NotFound)
    }

    pub(crate) fn register(
        &self,
        root: &Path,
        root_key: &str,
        git_key: &str,
        db_key: Option<&str>,
        name: &str,
        maximum: usize,
    ) -> Result<ProjectEntry> {
        validate_name(name)?;
        let mut conn = self
            .connection
            .lock()
            .map_err(|_| ProjectError::Registry("registry lock failed".into()))?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(registry_error)?;
        let entries = read_entries(&tx)?;
        if let Some(existing) = entries.iter().find(|entry| entry.root_identity == root_key) {
            return Ok(existing.clone());
        }
        if entries.len() >= maximum {
            return Err(ProjectError::Limit("registered projects"));
        }
        check_conflicts(&entries, None, root, git_key, db_key)?;
        let entry = ProjectEntry {
            project_id: ProjectId::new(),
            canonical_root: root.to_owned(),
            display_name: name.into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            last_opened_at: None,
            registry_schema_version: 1,
            root_identity: root_key.into(),
            git_identity: git_key.into(),
            database_identity: db_key.map(str::to_owned),
        };
        tx.execute(
            "INSERT INTO projects VALUES (?1,?2,?3,?4,NULL,1,?5,?6,?7)",
            params![
                entry.project_id.as_str(),
                root.to_str(),
                name,
                entry.created_at,
                root_key,
                git_key,
                db_key
            ],
        )
        .map_err(registry_error)?;
        tx.commit().map_err(registry_error)?;
        Ok(entry)
    }

    pub(crate) fn opened(&self, id: &ProjectId) -> Result<()> {
        let conn = self
            .connection
            .lock()
            .map_err(|_| ProjectError::Registry("registry lock failed".into()))?;
        let changed = conn
            .execute(
                "UPDATE projects SET last_opened_at=?2 WHERE project_id=?1",
                params![id.as_str(), chrono::Utc::now().to_rfc3339()],
            )
            .map_err(registry_error)?;
        if changed == 0 {
            return Err(ProjectError::NotFound);
        }
        Ok(())
    }

    pub(crate) fn rename(&self, id: &ProjectId, name: &str) -> Result<()> {
        validate_name(name)?;
        let changed = self
            .connection
            .lock()
            .map_err(|_| ProjectError::Registry("registry lock failed".into()))?
            .execute(
                "UPDATE projects SET display_name=?2 WHERE project_id=?1",
                params![id.as_str(), name],
            )
            .map_err(registry_error)?;
        if changed == 0 {
            return Err(ProjectError::NotFound);
        }
        Ok(())
    }

    pub(crate) fn remove(&self, id: &ProjectId) -> Result<()> {
        let changed = self
            .connection
            .lock()
            .map_err(|_| ProjectError::Registry("registry lock failed".into()))?
            .execute("DELETE FROM projects WHERE project_id=?1", [id.as_str()])
            .map_err(registry_error)?;
        if changed == 0 {
            return Err(ProjectError::NotFound);
        }
        Ok(())
    }

    pub(crate) fn relink(
        &self,
        id: &ProjectId,
        root: &Path,
        root_key: &str,
        git_key: &str,
        db_key: Option<&str>,
    ) -> Result<()> {
        let mut conn = self
            .connection
            .lock()
            .map_err(|_| ProjectError::Registry("registry lock failed".into()))?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(registry_error)?;
        let entries = read_entries(&tx)?;
        if !entries.iter().any(|entry| &entry.project_id == id) {
            return Err(ProjectError::NotFound);
        }
        check_conflicts(&entries, Some(id), root, git_key, db_key)?;
        if entries
            .iter()
            .any(|entry| &entry.project_id != id && entry.root_identity == root_key)
        {
            return Err(ProjectError::Conflict(
                "directory is already registered".into(),
            ));
        }
        tx.execute("UPDATE projects SET canonical_root=?2,root_identity=?3,git_identity=?4,database_identity=?5 WHERE project_id=?1", params![id.as_str(), root.to_str(), root_key, git_key, db_key]).map_err(registry_error)?;
        tx.commit().map_err(registry_error)?;
        Ok(())
    }
}

fn read_entries(conn: &Connection) -> Result<Vec<ProjectEntry>> {
    let mut stmt = conn.prepare("SELECT project_id, canonical_root, display_name, created_at, last_opened_at, registry_schema_version, root_identity, git_identity, database_identity FROM projects ORDER BY project_id").map_err(registry_error)?;
    let records = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, u32>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<String>>(8)?,
            ))
        })
        .map_err(registry_error)?;
    let mut entries = Vec::new();
    for record in records {
        let (id, root, name, created, opened, version, root_key, git_key, db_key) =
            record.map_err(registry_error)?;
        if version != 1 || !Path::new(&root).is_absolute() {
            return Err(ProjectError::Registry(
                "invalid registry entry; repair required".into(),
            ));
        }
        let id = ProjectId::parse(&id)
            .map_err(|_| ProjectError::Registry("invalid stored project identity".into()))?;
        entries.push(ProjectEntry {
            project_id: id,
            canonical_root: root.into(),
            display_name: name,
            created_at: created,
            last_opened_at: opened,
            registry_schema_version: version,
            root_identity: root_key,
            git_identity: git_key,
            database_identity: db_key,
        });
    }
    Ok(entries)
}

fn check_conflicts(
    entries: &[ProjectEntry],
    except: Option<&ProjectId>,
    root: &Path,
    git: &str,
    db: Option<&str>,
) -> Result<()> {
    for entry in entries
        .iter()
        .filter(|entry| Some(&entry.project_id) != except)
    {
        if root.starts_with(&entry.canonical_root) || entry.canonical_root.starts_with(root) {
            return Err(ProjectError::Conflict(
                "overlapping project roots are not allowed".into(),
            ));
        }
        if entry.git_identity == git {
            return Err(ProjectError::Conflict(
                "Git common directory already belongs to another project".into(),
            ));
        }
        if db.is_some() && entry.database_identity.as_deref() == db {
            return Err(ProjectError::Conflict(
                "database already belongs to another project".into(),
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_name(name: &str) -> Result<()> {
    if name.trim().is_empty() || name.len() > 240 || name.chars().any(char::is_control) {
        return Err(ProjectError::Invalid(
            "display name must contain 1–240 bytes without control characters".into(),
        ));
    }
    Ok(())
}

fn registry_error(error: rusqlite::Error) -> ProjectError {
    ProjectError::Registry(error.to_string())
}

pub fn application_registry_path() -> Result<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|p| p.join("AgenticSoftwareFactory"));
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|p| p.join("Library/Application Support/AgenticSoftwareFactory"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|p| p.join(".local/share"))
        })
        .map(|p| p.join("agentic-software-factory"));
    base.filter(|p| p.is_absolute())
        .map(|p| p.join("registry.sqlite3"))
        .ok_or_else(|| {
            ProjectError::Invalid(
                "application data location unavailable; pass --registry explicitly".into(),
            )
        })
}
