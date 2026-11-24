use crate::{now, DbError, FactoryDb, Result};
use factory_types::memory::*;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};

pub(crate) const SCHEMA: &str = "
CREATE TABLE project_memory_settings(project_id TEXT PRIMARY KEY, revision INTEGER NOT NULL, generation INTEGER NOT NULL, payload TEXT NOT NULL);
CREATE TABLE project_memories(id TEXT PRIMARY KEY, project_id TEXT NOT NULL, revision INTEGER NOT NULL, category TEXT NOT NULL, status TEXT NOT NULL, title TEXT NOT NULL, content TEXT NOT NULL, subject TEXT NOT NULL, scope TEXT NOT NULL, fingerprint TEXT NOT NULL, updated_at TEXT NOT NULL, payload TEXT NOT NULL);
CREATE INDEX project_memory_page ON project_memories(project_id,status,category);
CREATE INDEX project_memory_fingerprint ON project_memories(project_id,fingerprint);
CREATE INDEX project_memory_subject ON project_memories(project_id,subject,scope);
CREATE UNIQUE INDEX project_memory_active_subject ON project_memories(project_id,subject,scope) WHERE status='active' AND subject<>'';
CREATE UNIQUE INDEX project_memory_unique_claim ON project_memories(project_id,fingerprint) WHERE status<>'superseded';
CREATE TABLE project_memory_revisions(memory_id TEXT NOT NULL REFERENCES project_memories(id) ON DELETE CASCADE, revision INTEGER NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(memory_id,revision));
CREATE VIRTUAL TABLE project_memory_search USING fts5(memory_id UNINDEXED, revision UNINDEXED, title, content, tokenize='unicode61');
CREATE TABLE project_memory_deleted(project_id TEXT NOT NULL, fingerprint TEXT NOT NULL, deleted_at TEXT NOT NULL, PRIMARY KEY(project_id,fingerprint));
CREATE TABLE project_memory_extractions(id TEXT PRIMARY KEY, project_id TEXT NOT NULL, source_key TEXT NOT NULL, state TEXT NOT NULL, payload TEXT NOT NULL, UNIQUE(project_id,source_key));
CREATE TABLE project_memory_candidates(id TEXT PRIMARY KEY, project_id TEXT NOT NULL, extraction_id TEXT NOT NULL REFERENCES project_memory_extractions(id), state TEXT NOT NULL, fingerprint TEXT NOT NULL, payload TEXT NOT NULL);
CREATE INDEX project_memory_candidate_page ON project_memory_candidates(project_id,state);
CREATE TABLE project_memory_usage(id INTEGER PRIMARY KEY, project_id TEXT NOT NULL, session_id INTEGER NOT NULL UNIQUE REFERENCES agent_sessions(id), payload TEXT NOT NULL);
CREATE INDEX project_memory_usage_session ON project_memory_usage(project_id,session_id);
CREATE TABLE project_memory_storage(id INTEGER PRIMARY KEY CHECK(id=1), bytes INTEGER NOT NULL CHECK(bytes>=0));
INSERT INTO project_memory_storage VALUES(1,0);
CREATE TRIGGER memory_revision_bytes_insert AFTER INSERT ON project_memory_revisions BEGIN UPDATE project_memory_storage SET bytes=bytes+length(CAST(new.payload AS BLOB)); END;
CREATE TRIGGER memory_revision_bytes_delete AFTER DELETE ON project_memory_revisions BEGIN UPDATE project_memory_storage SET bytes=bytes-length(CAST(old.payload AS BLOB)); END;
CREATE TRIGGER memory_candidate_bytes_insert AFTER INSERT ON project_memory_candidates BEGIN UPDATE project_memory_storage SET bytes=bytes+length(CAST(new.payload AS BLOB)); END;
CREATE TRIGGER memory_candidate_bytes_update AFTER UPDATE ON project_memory_candidates BEGIN UPDATE project_memory_storage SET bytes=bytes+length(CAST(new.payload AS BLOB))-length(CAST(old.payload AS BLOB)); END;
CREATE TRIGGER memory_candidate_bytes_delete AFTER DELETE ON project_memory_candidates BEGIN UPDATE project_memory_storage SET bytes=bytes-length(CAST(old.payload AS BLOB)); END;
CREATE TRIGGER memory_usage_bytes_insert AFTER INSERT ON project_memory_usage BEGIN UPDATE project_memory_storage SET bytes=bytes+length(CAST(new.payload AS BLOB)); END;
";

pub(crate) const PERFORMANCE_SCHEMA: &str = "
DROP TABLE IF EXISTS project_memory_search;
CREATE VIRTUAL TABLE project_memory_search USING fts5(memory_id UNINDEXED, revision UNINDEXED, title, content, tokenize='unicode61');
INSERT INTO project_memory_search(rowid,memory_id,revision,title,content) SELECT rowid,id,revision,title,content FROM project_memories;
CREATE INDEX project_memory_recent ON project_memories(project_id);
";

pub fn memory_hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
pub fn memory_normalize(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn memory_fingerprint(m: &ProjectMemory) -> String {
    memory_hash(&format!(
        "{}:{}",
        memory_normalize(&m.scope),
        memory_normalize(&m.content)
    ))
}

fn bind(conn: &Connection, project: &str) -> Result<()> {
    if project.is_empty() || project.len() > 4096 {
        return Err(DbError::Corrupt("Invalid memory project".into()));
    }
    let owner: Option<String> = conn
        .query_row(
            "SELECT project_id FROM project_memory_settings LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if owner.as_deref().is_some_and(|id| id != project) {
        return Err(DbError::NotFound("project memory"));
    }
    Ok(())
}
fn bump(conn: &Connection, project: &str) -> Result<()> {
    conn.execute("INSERT INTO project_memory_settings VALUES(?1,0,1,?2) ON CONFLICT(project_id) DO UPDATE SET generation=generation+1",params![project,serde_json::to_string(&MemorySettings::default())?])?;
    Ok(())
}
fn stored(conn: &Connection, project: &str, id: &str) -> Result<ProjectMemory> {
    let payload: String = conn
        .query_row(
            "SELECT payload FROM project_memories WHERE project_id=?1 AND id=?2",
            params![project, id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(DbError::NotFound("project memory"))?;
    let m: ProjectMemory = serde_json::from_str(&payload)?;
    if m.project_id != project || m.id != id || m.schema_version != 1 {
        return Err(DbError::Corrupt("Memory identity/schema mismatch".into()));
    }
    Ok(m)
}
fn budget(conn: &Connection, extra: usize) -> Result<()> {
    let bytes: i64 = conn.query_row(
        "SELECT bytes FROM project_memory_storage WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    if bytes.saturating_add(extra as i64) > 128 * 1024 * 1024 {
        return Err(DbError::Busy(
            "Project memory retention budget reached; history is retained".into(),
        ));
    }
    Ok(())
}
fn write(conn: &Connection, m: &ProjectMemory) -> Result<()> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM project_memory_revisions WHERE memory_id=?1",
        [&m.id],
        |r| r.get(0),
    )?;
    if count >= 256 {
        return Err(DbError::Busy("Memory revision limit reached".into()));
    }
    let payload = serde_json::to_string(m)?;
    budget(conn, payload.len())?;
    conn.execute("INSERT INTO project_memories(id,project_id,revision,category,status,title,content,subject,scope,fingerprint,updated_at,payload) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,category=excluded.category,status=excluded.status,title=excluded.title,content=excluded.content,subject=excluded.subject,scope=excluded.scope,fingerprint=excluded.fingerprint,updated_at=excluded.updated_at,payload=excluded.payload",params![m.id,m.project_id,m.revision,m.category.as_str(),m.status.as_str(),m.title,m.content,memory_normalize(&m.subject),memory_normalize(&m.scope),memory_fingerprint(m),m.updated_at,payload])?;
    conn.execute(
        "INSERT INTO project_memory_revisions VALUES(?1,?2,?3)",
        params![m.id, m.revision, payload],
    )?;
    let rowid: i64 = conn.query_row(
        "SELECT rowid FROM project_memories WHERE id=?1",
        [&m.id],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO project_memory_search(rowid,memory_id,revision,title,content) VALUES(?1,?2,?3,?4,?5)",
        params![rowid,m.id, m.revision, m.title, m.content],
    )?;
    Ok(())
}
fn save(tx: &Transaction<'_>, m: &ProjectMemory, expected: Option<i64>) -> Result<()> {
    bind(tx, &m.project_id)?;
    let current: Option<i64> = tx
        .query_row(
            "SELECT revision FROM project_memories WHERE project_id=?1 AND id=?2",
            params![m.project_id, m.id],
            |r| r.get(0),
        )
        .optional()?;
    if current != expected || m.revision != expected.unwrap_or(0) + 1 {
        return Err(DbError::Conflict {
            expected: expected.unwrap_or(0),
            current: current.unwrap_or(0),
        });
    }
    if expected.is_none() {
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM project_memories", [], |r| r.get(0))?;
        if count >= 20000 {
            return Err(DbError::Busy("Project memory record limit reached".into()));
        }
        let deletions: i64 =
            tx.query_row("SELECT COUNT(*) FROM project_memory_deleted", [], |r| {
                r.get(0)
            })?;
        if deletions >= 100000 {
            return Err(DbError::Busy(
                "Memory deletion ledger is full; existing records remain editable and deletable"
                    .into(),
            ));
        }
    }
    let duplicate:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM project_memories WHERE project_id=?1 AND id<>?2 AND fingerprint=?3 AND status<>'superseded')",params![m.project_id,m.id,memory_fingerprint(m)],|r|r.get(0))?;
    if duplicate && m.status != MemoryStatus::Superseded {
        return Err(DbError::Busy(
            "Duplicate memory claim; inspect its existing record".into(),
        ));
    }
    write(tx, m)?;
    if m.status == MemoryStatus::Disputed && !m.subject.is_empty() {
        let active:Option<String>=tx.query_row("SELECT id FROM project_memories WHERE project_id=?1 AND id<>?2 AND status='active' AND subject=?3 AND scope=?4",params![m.project_id,m.id,memory_normalize(&m.subject),memory_normalize(&m.scope)],|r|r.get(0)).optional()?;
        if let Some(related) = active {
            let mut other = stored(tx, &m.project_id, &related)?;
            if other.status == MemoryStatus::Active
                && memory_normalize(&other.subject) == memory_normalize(&m.subject)
                && memory_normalize(&other.scope) == memory_normalize(&m.scope)
                && memory_fingerprint(&other) != memory_fingerprint(m)
            {
                other.status = MemoryStatus::Disputed;
                other.revision += 1;
                other.updated_at = now();
                other.change_reason =
                    "Conflicting scoped claim accepted; explicit resolution required".into();
                if !other.related_ids.contains(&m.id) {
                    other.related_ids.push(m.id.clone());
                }
                write(tx, &other)?;
            }
        }
    }
    bump(tx, &m.project_id)
}

fn validate_context(conn: &Connection, project: &str, context: &MemoryContext) -> Result<()> {
    bind(conn, project)?;
    let (generation, payload): (i64, String) = conn.query_row(
        "SELECT generation,payload FROM project_memory_settings WHERE project_id=?1",
        [project],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let settings: MemorySettings = serde_json::from_str(&payload)?;
    if context.project_id != project
        || context.generation != generation
        || context.settings_revision != settings.revision
    {
        return Err(DbError::Conflict {
            expected: context.generation,
            current: generation,
        });
    }
    if !context.records.is_empty() && (!settings.enabled || !settings.retrieval_enabled) {
        return Err(DbError::Conflict {
            expected: context.settings_revision,
            current: settings.revision,
        });
    }
    for selected in &context.records {
        let current = stored(conn, project, &selected.id)?;
        if selected.project_id != project
            || selected.schema_version != 1
            || selected != &current
            || current.status != MemoryStatus::Active
            || !settings.categories.contains(&current.category)
        {
            return Err(DbError::Conflict {
                expected: selected.revision,
                current: current.revision,
            });
        }
    }
    if context.records.len() > settings.max_context_records
        || serde_json::to_vec(context)?.len() > settings.max_context_bytes
    {
        return Err(DbError::Busy("Memory context budget exceeded".into()));
    }
    Ok(())
}

impl FactoryDb {
    pub fn memory_bind_project(&self, project: &str) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        bind(&tx, project)?;
        tx.execute(
            "INSERT OR IGNORE INTO project_memory_settings VALUES(?1,0,0,?2)",
            params![project, serde_json::to_string(&MemorySettings::default())?],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn memory_source_message(
        &self,
        conversation: &str,
        message: i64,
    ) -> Result<factory_types::ConversationMessage> {
        self.conn.query_row("SELECT id,conversation_id,role,content,status,run_id,parent_message_id,provenance,created_at FROM conversation_messages WHERE conversation_id=?1 AND id=?2",params![conversation,message],|r|Ok(factory_types::ConversationMessage{id:r.get(0)?,conversation_id:r.get(1)?,role:r.get(2)?,content:r.get(3)?,status:r.get(4)?,run_id:r.get(5)?,parent_message_id:r.get(6)?,provenance:serde_json::from_str(&r.get::<_,String>(7)?).unwrap_or_default(),created_at:r.get(8)?})).optional()?.ok_or(DbError::NotFound("conversation message"))
    }
    pub fn memory_settings(&self, project: &str) -> Result<MemorySettings> {
        bind(&self.conn, project)?;
        let payload: Option<String> = self
            .conn
            .query_row(
                "SELECT payload FROM project_memory_settings WHERE project_id=?1",
                [project],
                |r| r.get(0),
            )
            .optional()?;
        payload
            .map(|p| serde_json::from_str(&p).map_err(Into::into))
            .unwrap_or_else(|| Ok(MemorySettings::default()))
    }
    pub fn memory_generation(&self, project: &str) -> Result<i64> {
        bind(&self.conn, project)?;
        Ok(self
            .conn
            .query_row(
                "SELECT generation FROM project_memory_settings WHERE project_id=?1",
                [project],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }
    pub fn memory_save_settings(
        &self,
        project: &str,
        settings: &MemorySettings,
    ) -> Result<MemorySettings> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        bind(&tx, project)?;
        let current = self.memory_settings(project)?;
        if current.revision != settings.revision {
            return Err(DbError::Conflict {
                expected: settings.revision,
                current: current.revision,
            });
        }
        let mut next = settings.clone();
        next.revision += 1;
        bump(&tx, project)?;
        tx.execute(
            "UPDATE project_memory_settings SET revision=?2,payload=?3 WHERE project_id=?1",
            params![project, next.revision, serde_json::to_string(&next)?],
        )?;
        tx.commit()?;
        Ok(next)
    }
    pub fn memory_get(&self, project: &str, id: &str) -> Result<ProjectMemory> {
        bind(&self.conn, project)?;
        stored(&self.conn, project, id)
    }
    pub fn memory_save(&self, memory: &ProjectMemory, expected: Option<i64>) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        save(&tx, memory, expected)?;
        tx.commit()?;
        Ok(())
    }
    pub fn memory_revisions(
        &self,
        project: &str,
        id: &str,
        before: Option<i64>,
    ) -> Result<Vec<ProjectMemory>> {
        self.memory_get(project, id)?;
        let mut q=self.conn.prepare("SELECT payload FROM project_memory_revisions WHERE memory_id=?1 AND (?2 IS NULL OR revision<?2) ORDER BY revision DESC LIMIT 50")?;
        let rows = q.query_map(params![id, before], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn memory_delete(&self, project: &str, id: &str, expected: i64) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let m = stored(&tx, project, id)?;
        if m.revision != expected {
            return Err(DbError::Conflict {
                expected,
                current: m.revision,
            });
        }
        tx.execute(
            "INSERT OR IGNORE INTO project_memory_deleted VALUES(?1,?2,?3)",
            params![project, memory_fingerprint(&m), now()],
        )?;
        tx.execute("DELETE FROM project_memory_candidates WHERE project_id=?1 AND (fingerprint=?2 OR EXISTS(SELECT 1 FROM json_each(json_extract(payload,'$.memory.relatedIds')) WHERE value=?3) OR json_extract(payload,'$.acceptedMemoryId')=?3)",params![project,memory_fingerprint(&m),id])?;
        tx.execute("DELETE FROM project_memory_search WHERE memory_id=?1", [id])?;
        tx.execute(
            "DELETE FROM project_memories WHERE project_id=?1 AND id=?2",
            params![project, id],
        )?;
        bump(&tx, project)?;
        tx.commit()?;
        Ok(())
    }
    pub fn memory_was_deleted(&self, project: &str, m: &ProjectMemory) -> Result<bool> {
        bind(&self.conn, project)?;
        Ok(self.conn.query_row("SELECT EXISTS(SELECT 1 FROM project_memory_deleted WHERE project_id=?1 AND fingerprint=?2)",params![project,memory_fingerprint(m)],|r|r.get(0))?)
    }
    pub fn memory_search(&self, project: &str, request: &MemorySearch) -> Result<MemoryPage> {
        bind(&self.conn, project)?;
        let tokens: Vec<String> = memory_normalize(&request.query)
            .split_whitespace()
            .take(12)
            .map(|s| format!("\"{s}\"*"))
            .collect();
        let fts = tokens.join(" OR ");
        let run = |indexed: bool| -> Result<MemoryPage> {
            let table = if indexed && !fts.is_empty() {
                "project_memory_search JOIN project_memories m ON m.rowid=project_memory_search.rowid AND m.id=project_memory_search.memory_id AND m.revision=project_memory_search.revision"
            } else {
                "project_memories m INDEXED BY project_memory_recent"
            };
            let order = if indexed && !fts.is_empty() {
                "project_memory_search.rowid"
            } else {
                "m.rowid"
            };
            let matches = if indexed && !fts.is_empty() {
                "AND project_memory_search MATCH ?9"
            } else if !fts.is_empty() {
                "AND EXISTS(SELECT 1 FROM json_each(?10) WHERE instr(lower(m.title||' '||m.content),value)>0)"
            } else {
                ""
            };
            let sql=format!("SELECT m.rowid,m.payload FROM {table} WHERE m.project_id=?1 AND (?2 IS NULL OR m.category=?2) AND (?3 IS NULL OR m.status=?3) AND (?4 IS NULL OR m.rowid<?4) AND (?5 IS NULL OR julianday(m.updated_at)>=julianday(?5)) AND (?6 IS NULL OR EXISTS(SELECT 1 FROM json_each(json_extract(m.payload,'$.provenance')) WHERE json_extract(value,'$.source.reference.kind')=?6)) {matches} AND ?9 IS NOT NULL AND ?10 IS NOT NULL ORDER BY {order} DESC LIMIT ?7 OFFSET ?8");
            let limit = request.limit.unwrap_or(50).clamp(1, 100);
            let needle = serde_json::to_string(
                &memory_normalize(&request.query)
                    .split_whitespace()
                    .take(12)
                    .collect::<Vec<_>>(),
            )?;
            let mut q = self.conn.prepare(&sql)?;
            let rows = q
                .query_map(
                    params![
                        project,
                        request.category.map(|c| c.as_str()),
                        request.status.map(|s| s.as_str()),
                        request.before,
                        request.updated_after,
                        request.source_type,
                        limit + 1,
                        0,
                        fts,
                        needle
                    ],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
                )?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let more = rows.len() > limit;
            let cursor = if more {
                rows.get(limit - 1).map(|r| r.0)
            } else {
                None
            };
            let items = rows
                .into_iter()
                .take(limit)
                .map(|(_, p)| serde_json::from_str(&p).map_err(Into::into))
                .collect::<Result<Vec<_>>>()?;
            Ok(MemoryPage {
                items,
                next_cursor: cursor,
                search_mode: if indexed {
                    "sqlite_fts5"
                } else {
                    "bounded_literal_fallback"
                }
                .into(),
            })
        };
        run(true).or_else(|_| run(false))
    }
    pub fn memory_rebuild_index(&self, project: &str) -> Result<usize> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        bind(&tx, project)?;
        tx.execute_batch("CREATE INDEX IF NOT EXISTS project_memory_recent ON project_memories(project_id); DROP TABLE IF EXISTS project_memory_search; CREATE VIRTUAL TABLE project_memory_search USING fts5(memory_id UNINDEXED, revision UNINDEXED, title, content, tokenize='unicode61'); INSERT INTO project_memory_search(rowid,memory_id,revision,title,content) SELECT rowid,id,revision,title,content FROM project_memories;")?;
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM project_memories", [], |r| r.get(0))?;
        tx.commit()?;
        Ok(count as usize)
    }
    pub fn memory_context_candidates(
        &self,
        project: &str,
        query: &str,
        categories: &[MemoryCategory],
    ) -> Result<MemoryPage> {
        bind(&self.conn, project)?;
        let normalized = memory_normalize(query);
        let words = normalized.split_whitespace().take(12).collect::<Vec<_>>();
        if words.is_empty() || categories.is_empty() {
            return Ok(MemoryPage {
                items: vec![],
                next_cursor: None,
                search_mode: "sqlite_fts5".into(),
            });
        }
        let terms = words
            .iter()
            .map(|w| format!("\"{w}\"*"))
            .collect::<Vec<_>>()
            .join(" OR ");
        let categories =
            serde_json::to_string(&categories.iter().map(|c| c.as_str()).collect::<Vec<_>>())?;
        let fetch = |indexed: bool| -> Result<MemoryPage> {
            let sql = if indexed {
                "SELECT m.rowid,m.payload FROM project_memory_search JOIN project_memories m ON m.id=memory_id AND m.revision=project_memory_search.revision WHERE m.project_id=?1 AND m.status='active' AND m.category IN (SELECT value FROM json_each(?2)) AND project_memory_search MATCH ?3 ORDER BY bm25(project_memory_search,0,0,5,1),m.id LIMIT 65"
            } else {
                "SELECT m.rowid,m.payload FROM project_memories m WHERE m.project_id=?1 AND m.status='active' AND m.category IN (SELECT value FROM json_each(?2)) AND EXISTS(SELECT 1 FROM json_each(?3) WHERE instr(lower(m.title||' '||m.content),value)>0) ORDER BY m.rowid DESC LIMIT 65"
            };
            let search = if indexed {
                terms.clone()
            } else {
                serde_json::to_string(&words)?
            };
            let mut stmt = self.conn.prepare(sql)?;
            let rows = stmt
                .query_map(params![project, categories, search], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let cursor = if rows.len() > 64 {
                rows.get(63).map(|r| r.0)
            } else {
                None
            };
            Ok(MemoryPage {
                items: rows
                    .into_iter()
                    .take(64)
                    .map(|(_, p)| Ok(serde_json::from_str(&p)?))
                    .collect::<Result<Vec<_>>>()?,
                next_cursor: cursor,
                search_mode: if indexed {
                    "sqlite_fts5_ranked"
                } else {
                    "bounded_literal_fallback"
                }
                .into(),
            })
        };
        fetch(true).or_else(|_| fetch(false))
    }
    pub fn memory_similar(&self, project: &str, m: &ProjectMemory) -> Result<Vec<ProjectMemory>> {
        bind(&self.conn, project)?;
        let mut q=self.conn.prepare("SELECT payload,id FROM project_memories WHERE project_id=?1 AND id<>?2 AND status<>'superseded' AND fingerprint=?3 UNION SELECT payload,id FROM project_memories WHERE project_id=?1 AND id<>?2 AND status<>'superseded' AND subject<>'' AND subject=?4 AND scope=?5 ORDER BY id LIMIT 32")?;
        let rows = q.query_map(
            params![
                project,
                m.id,
                memory_fingerprint(m),
                memory_normalize(&m.subject),
                memory_normalize(&m.scope)
            ],
            |r| r.get::<_, String>(0),
        )?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn memory_resolve(
        &self,
        project: &str,
        id: &str,
        target: &str,
        expected: i64,
        target_expected: i64,
    ) -> Result<ProjectMemory> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let mut old = stored(&tx, project, id)?;
        let mut new = stored(&tx, project, target)?;
        if id == target {
            return Err(DbError::Corrupt(
                "Cannot supersede a memory with itself".into(),
            ));
        }
        if old.revision != expected {
            return Err(DbError::Conflict {
                expected,
                current: old.revision,
            });
        }
        if new.revision != target_expected {
            return Err(DbError::Conflict {
                expected: target_expected,
                current: new.revision,
            });
        }
        old.revision += 1;
        old.status = MemoryStatus::Superseded;
        old.related_ids = vec![target.into()];
        old.updated_at = now();
        old.change_reason = "Explicit user supersession".into();
        new.revision += 1;
        new.status = MemoryStatus::Active;
        new.related_ids = vec![id.into()];
        new.updated_at = now();
        new.change_reason = "User selected current scoped claim; inspect provenance".into();
        write(&tx, &old)?;
        write(&tx, &new)?;
        bump(&tx, project)?;
        tx.commit()?;
        Ok(new)
    }
    pub fn memory_record_usage(
        &self,
        project: &str,
        usage: &MemoryUsage,
        context: &MemoryContext,
    ) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        bind(&tx, project)?;
        validate_context(&tx, project, context)?;
        let generation = self.memory_generation(project)?;
        if context.project_id != project || generation != context.generation {
            return Err(DbError::Conflict {
                expected: context.generation,
                current: generation,
            });
        }
        for selected in &context.records {
            let current = stored(&tx, project, &selected.id)?;
            if current.revision != selected.revision || current.status != MemoryStatus::Active {
                return Err(DbError::Conflict {
                    expected: selected.revision,
                    current: current.revision,
                });
            }
        }
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM project_memory_usage", [], |r| {
            r.get(0)
        })?;
        if count >= 100000 {
            return Err(DbError::Busy("Memory usage audit limit reached".into()));
        }
        let payload = serde_json::to_string(usage)?;
        budget(&tx, payload.len())?;
        tx.execute(
            "INSERT INTO project_memory_usage(project_id,session_id,payload) VALUES(?1,?2,?3)",
            params![project, usage.agent_session_id, payload],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn memory_validate_context(&self, project: &str, context: &MemoryContext) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Deferred)?;
        validate_context(&tx, project, context)?;
        tx.commit()?;
        Ok(())
    }
    pub fn memory_usage(
        &self,
        project: &str,
        session: Option<i64>,
        before: Option<i64>,
    ) -> Result<Vec<MemoryUsage>> {
        bind(&self.conn, project)?;
        let mut q=self.conn.prepare("SELECT id,payload FROM project_memory_usage WHERE project_id=?1 AND (?2 IS NULL OR session_id=?2) AND (?3 IS NULL OR id<?3) ORDER BY id DESC LIMIT 100")?;
        let rows = q.query_map(params![project, session, before], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.map(|r| {
            let (id, p) = r?;
            let mut usage: MemoryUsage = serde_json::from_str(&p)?;
            usage.id = id;
            Ok(usage)
        })
        .collect()
    }
    pub fn memory_claim_extraction(&self, extraction: &MemoryExtraction) -> Result<bool> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        bind(&tx, &extraction.project_id)?;
        let count: i64 =
            tx.query_row("SELECT COUNT(*) FROM project_memory_extractions", [], |r| {
                r.get(0)
            })?;
        if count >= 10000 {
            return Err(DbError::Busy(
                "Memory extraction history limit reached".into(),
            ));
        }
        let n = tx.execute(
            "INSERT OR IGNORE INTO project_memory_extractions VALUES(?1,?2,?3,?4,?5)",
            params![
                extraction.id,
                extraction.project_id,
                extraction.source_key,
                extraction.state,
                serde_json::to_string(extraction)?
            ],
        )?;
        tx.commit()?;
        Ok(n == 1)
    }
    pub fn memory_finish_extraction(&self, extraction: &MemoryExtraction) -> Result<()> {
        bind(&self.conn, &extraction.project_id)?;
        self.conn.execute("UPDATE project_memory_extractions SET state=?2,payload=json_set(?3,'$.agentSessionId',COALESCE(json_extract(?3,'$.agentSessionId'),json_extract(payload,'$.agentSessionId'))) WHERE id=?1 AND project_id=?4 AND state='running'",params![extraction.id,extraction.state,serde_json::to_string(extraction)?,extraction.project_id])?;
        Ok(())
    }
    pub fn memory_attach_extraction_session(
        &self,
        project: &str,
        extraction: &str,
        session: i64,
    ) -> Result<()> {
        bind(&self.conn, project)?;
        if self.conn.execute("UPDATE project_memory_extractions SET payload=json_set(payload,'$.agentSessionId',?3) WHERE id=?1 AND project_id=?2 AND state='running' AND json_extract(payload,'$.agentSessionId') IS NULL",params![extraction,project,session])?!=1{return Err(DbError::Busy("Extractor ownership changed before process launch".into()));}
        Ok(())
    }
    pub fn memory_extractions(&self, project: &str) -> Result<Vec<MemoryExtraction>> {
        bind(&self.conn, project)?;
        let mut q=self.conn.prepare("SELECT payload FROM project_memory_extractions WHERE project_id=?1 ORDER BY rowid DESC LIMIT 100")?;
        let rows = q.query_map([project], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn memory_extraction(&self, project: &str, id: &str) -> Result<MemoryExtraction> {
        bind(&self.conn, project)?;
        let payload: String = self
            .conn
            .query_row(
                "SELECT payload FROM project_memory_extractions WHERE project_id=?1 AND id=?2",
                params![project, id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(DbError::NotFound("memory extraction"))?;
        let extraction: MemoryExtraction = serde_json::from_str(&payload)?;
        if extraction.project_id != project || extraction.id != id {
            return Err(DbError::Corrupt(
                "Memory extraction identity mismatch".into(),
            ));
        }
        Ok(extraction)
    }
    pub fn memory_save_candidates(
        &self,
        project: &str,
        candidates: &[MemoryCandidate],
        settings_revision: i64,
    ) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        bind(&tx, project)?;
        let settings = self.memory_settings(project)?;
        if !settings.enabled
            || settings.extraction_mode == ExtractionMode::Disabled
            || settings.revision != settings_revision
        {
            return Err(DbError::Busy(
                "Memory extraction policy changed; candidates not accepted".into(),
            ));
        }
        let count: i64 =
            tx.query_row("SELECT COUNT(*) FROM project_memory_candidates", [], |r| {
                r.get(0)
            })?;
        if candidates.len() > 8 || count + candidates.len() as i64 > 20000 {
            return Err(DbError::Busy("Memory candidate limit reached".into()));
        }
        for c in candidates {
            if c.project_id != project || c.memory.project_id != project {
                return Err(DbError::NotFound("memory candidate"));
            }
            let deleted:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM project_memory_deleted WHERE project_id=?1 AND fingerprint=?2)",params![project,memory_fingerprint(&c.memory)],|r|r.get(0))?;
            if deleted {
                continue;
            }
            let payload = serde_json::to_string(c)?;
            budget(&tx, payload.len())?;
            tx.execute(
                "INSERT INTO project_memory_candidates VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    c.id,
                    project,
                    c.extraction_id,
                    c.state,
                    memory_fingerprint(&c.memory),
                    payload
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn memory_candidates(
        &self,
        project: &str,
        before: Option<i64>,
    ) -> Result<(Vec<MemoryCandidate>, Option<i64>)> {
        bind(&self.conn, project)?;
        let mut q=self.conn.prepare("SELECT rowid,payload FROM project_memory_candidates WHERE project_id=?1 AND state='pending' AND (?2 IS NULL OR rowid<?2) ORDER BY rowid DESC LIMIT 51")?;
        let rows = q
            .query_map(params![project, before], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let cursor = if rows.len() > 50 {
            rows.get(49).map(|r| r.0)
        } else {
            None
        };
        Ok((
            rows.into_iter()
                .take(50)
                .map(|(_, p)| Ok(serde_json::from_str(&p)?))
                .collect::<Result<Vec<_>>>()?,
            cursor,
        ))
    }
    pub fn memory_candidate(&self, project: &str, id: &str) -> Result<MemoryCandidate> {
        bind(&self.conn, project)?;
        let p: String = self
            .conn
            .query_row(
                "SELECT payload FROM project_memory_candidates WHERE project_id=?1 AND id=?2",
                params![project, id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(DbError::NotFound("memory candidate"))?;
        Ok(serde_json::from_str(&p)?)
    }
    pub fn memory_decide_candidate(
        &self,
        project: &str,
        id: &str,
        accepted: Option<&ProjectMemory>,
    ) -> Result<MemoryCandidate> {
        self.memory_decide_candidate_policy(project, id, accepted, None)
    }
    pub fn memory_decide_candidate_automatic(
        &self,
        project: &str,
        id: &str,
        memory: &ProjectMemory,
        expected_policy: i64,
    ) -> Result<MemoryCandidate> {
        self.memory_decide_candidate_policy(project, id, Some(memory), Some(expected_policy))
    }
    fn memory_decide_candidate_policy(
        &self,
        project: &str,
        id: &str,
        accepted: Option<&ProjectMemory>,
        automatic_policy: Option<i64>,
    ) -> Result<MemoryCandidate> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let mut c = self.memory_candidate(project, id)?;
        if c.state != "pending" {
            tx.commit()?;
            return Ok(c);
        }
        if let Some(m) = accepted {
            if let Some(expected) = automatic_policy {
                let settings = self.memory_settings(project)?;
                if settings.revision != expected
                    || !settings.enabled
                    || settings.extraction_mode != ExtractionMode::Automatic
                    || !settings.categories.contains(&m.category)
                    || !c.issue.is_empty()
                    || m.status != MemoryStatus::Active
                    || m.provenance.iter().any(|p| {
                        p.validation_status != "repository_content_checked"
                            || p.evidence_excerpt.is_none()
                    })
                {
                    return Err(DbError::Busy(
                        "Automatic memory acceptance policy changed or requires review".into(),
                    ));
                }
            }
            if self.memory_was_deleted(project, m)? {
                return Err(DbError::NotFound("deleted candidate memory"));
            }
            save(&tx, m, None)?;
            c.state = "accepted".into();
            c.accepted_memory_id = Some(m.id.clone());
        } else {
            c.state = "rejected".into();
        }
        tx.execute(
            "UPDATE project_memory_candidates SET state=?2,payload=?3 WHERE id=?1",
            params![id, c.state, serde_json::to_string(&c)?],
        )?;
        tx.commit()?;
        Ok(c)
    }
}

#[cfg(test)]
mod performance_tests {
    use super::*;
    fn record(n: i64) -> ProjectMemory {
        ProjectMemory {
            schema_version: 1,
            id: format!("pm_{n:032x}"),
            project_id: format!("p_{:032x}", 1),
            revision: 1,
            category: MemoryCategory::GeneralFacts,
            title: format!("Adapter {n}"),
            content: format!("Component {n} uses an adapter"),
            subject: format!("component {n}"),
            scope: format!("component {n}"),
            status: MemoryStatus::Active,
            origin: MemoryOrigin::UserAuthored,
            provenance: vec![],
            created_at: now(),
            updated_at: now(),
            change_reason: "Database fixture".into(),
            related_ids: vec![],
        }
    }
    #[test]
    fn derived_index_upgrade_retains_records_revisions_order_and_deletion_semantics() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut conn = Connection::open(&path).unwrap();
        crate::migrate_schemas(&mut conn, &crate::MIGRATIONS[..25]).unwrap();
        let db = FactoryDb { conn };
        let project = record(1).project_id;
        db.memory_bind_project(&project).unwrap();
        for n in 1..=5 {
            db.memory_save(&record(n), None).unwrap();
        }
        db.conn.execute_batch("DELETE FROM project_memory_search;INSERT INTO project_memory_search(rowid,memory_id,revision,title,content) SELECT rowid+100,id,revision,title,content FROM project_memories;").unwrap();
        let before: Vec<String> = db
            .conn
            .prepare("SELECT payload FROM project_memory_revisions ORDER BY memory_id,revision")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        let integrity: Vec<String> = db
            .conn
            .prepare("PRAGMA quick_check")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(integrity, vec!["ok"]);
        drop(db);
        let db = FactoryDb::open(&path).unwrap();
        let after: Vec<String> = db
            .conn
            .prepare("SELECT payload FROM project_memory_revisions ORDER BY memory_id,revision")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(before, after);
        let page = db
            .memory_search(
                &project,
                &MemorySearch {
                    query: "adapter".into(),
                    limit: Some(2),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            page.items.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
            vec![record(5).id, record(4).id]
        );
        let next = db
            .memory_search(
                &project,
                &MemorySearch {
                    query: "adapter".into(),
                    limit: Some(2),
                    before: page.next_cursor,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(next.items[0].id, record(3).id);
        let mut updated = record(5);
        updated.revision = 2;
        updated.title = "Different fact".into();
        updated.content = "A separate verified convention".into();
        updated.status = MemoryStatus::Disabled;
        db.memory_save(&updated, Some(1)).unwrap();
        assert!(!db
            .memory_search(
                &project,
                &MemorySearch {
                    query: "adapter".into(),
                    ..Default::default()
                }
            )
            .unwrap()
            .items
            .iter()
            .any(|m| m.id == updated.id));
        db.memory_delete(&project, &record(4).id, 1).unwrap();
        db.memory_rebuild_index(&project).unwrap();
        assert!(!db
            .memory_search(
                &project,
                &MemorySearch {
                    query: "adapter".into(),
                    ..Default::default()
                }
            )
            .unwrap()
            .items
            .iter()
            .any(|m| m.id == record(4).id));
        assert!(db
            .memory_context_candidates(
                &project,
                "separate verified",
                &[MemoryCategory::GeneralFacts]
            )
            .unwrap()
            .items
            .is_empty());
        let exact=db.conn.query_row("SELECT COUNT(*) FROM project_memories m JOIN project_memory_search s ON m.rowid=s.rowid AND m.id=s.memory_id AND m.revision=s.revision",[],|r|r.get::<_,i64>(0)).unwrap();
        assert_eq!(exact, 4);
    }
}
