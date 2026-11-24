use crate::{now, DbError, FactoryDb, Result};
use factory_types::{
    Conversation, ConversationCommand, ConversationEvent, ConversationMessage, RecipeSnapshot, Run,
    WorkflowTeam,
};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::{json, Value};

pub(crate) const SCHEMA: &str = "
CREATE TABLE conversations(id TEXT PRIMARY KEY, project_id TEXT NOT NULL, title TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'active', revision INTEGER NOT NULL DEFAULT 1, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE INDEX idx_conversations_updated ON conversations(project_id,created_at,id);
CREATE TABLE conversation_messages(id INTEGER PRIMARY KEY AUTOINCREMENT, conversation_id TEXT NOT NULL REFERENCES conversations(id), role TEXT NOT NULL, content TEXT NOT NULL, status TEXT NOT NULL, run_id INTEGER REFERENCES runs(id), parent_message_id INTEGER REFERENCES conversation_messages(id), provenance TEXT NOT NULL, created_at TEXT NOT NULL);
CREATE INDEX idx_conversation_messages ON conversation_messages(conversation_id,id);
CREATE TABLE conversation_commands(id INTEGER PRIMARY KEY AUTOINCREMENT, conversation_id TEXT NOT NULL REFERENCES conversations(id), message_id INTEGER NOT NULL REFERENCES conversation_messages(id), response_id INTEGER NOT NULL REFERENCES conversation_messages(id), command_key TEXT NOT NULL, fingerprint TEXT NOT NULL, kind TEXT NOT NULL, state TEXT NOT NULL, payload TEXT NOT NULL, run_id INTEGER REFERENCES runs(id), error TEXT, UNIQUE(conversation_id,command_key));
CREATE INDEX idx_conversation_commands ON conversation_commands(conversation_id,state,id);
CREATE TABLE conversation_events(id INTEGER PRIMARY KEY AUTOINCREMENT, conversation_id TEXT NOT NULL REFERENCES conversations(id), event_type TEXT NOT NULL, payload TEXT NOT NULL, created_at TEXT NOT NULL);
CREATE INDEX idx_conversation_events ON conversation_events(conversation_id,id);
CREATE TABLE conversation_missions(run_id INTEGER PRIMARY KEY REFERENCES runs(id), conversation_id TEXT NOT NULL REFERENCES conversations(id), command_id INTEGER NOT NULL UNIQUE REFERENCES conversation_commands(id), snapshot TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL);
CREATE INDEX idx_conversation_missions ON conversation_missions(conversation_id,run_id);
";

fn conversation(row: &rusqlite::Row<'_>) -> rusqlite::Result<Conversation> {
    Ok(Conversation {
        id: row.get(0)?,
        project_id: row.get(1)?,
        title: row.get(2)?,
        status: row.get(3)?,
        revision: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn message(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConversationMessage> {
    Ok(ConversationMessage {
        id: row.get(0)?,
        conversation_id: row.get(1)?,
        role: row.get(2)?,
        content: row.get(3)?,
        status: row.get(4)?,
        run_id: row.get(5)?,
        parent_message_id: row.get(6)?,
        provenance: serde_json::from_str(&row.get::<_, String>(7)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(e))
        })?,
        created_at: row.get(8)?,
    })
}
fn command(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConversationCommand> {
    Ok(ConversationCommand {
        id: row.get(0)?,
        conversation_id: row.get(1)?,
        message_id: row.get(2)?,
        response_id: row.get(3)?,
        key: row.get(4)?,
        fingerprint: row.get(5)?,
        kind: row.get(6)?,
        state: row.get(7)?,
        payload: serde_json::from_str(&row.get::<_, String>(8)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(e))
        })?,
        run_id: row.get(9)?,
        error: row.get(10)?,
    })
}
fn storage_budget(conn: &rusqlite::Connection) -> Result<()> {
    let bytes: i64 = conn.query_row(
        "SELECT COALESCE(SUM(length(CAST(content AS BLOB))+length(CAST(provenance AS BLOB))),0) FROM conversation_messages",
        [],
        |r| r.get(0),
    )?;
    let events: i64 = conn.query_row(
        "SELECT COALESCE(SUM(length(CAST(payload AS BLOB))),0) FROM conversation_events",
        [],
        |r| r.get(0),
    )?;
    if bytes + events >= 256 * 1024 * 1024 {
        return Err(DbError::Corrupt(
            "conversation storage limit reached; export or explicitly remove retained data offline"
                .into(),
        ));
    }
    Ok(())
}
pub(crate) fn event(
    conn: &rusqlite::Connection,
    id: &str,
    kind: &str,
    payload: &Value,
) -> Result<()> {
    storage_budget(conn)?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM conversation_events WHERE conversation_id=?1",
        [id],
        |r| r.get(0),
    )?;
    if count >= 50000 {
        return Err(DbError::Corrupt(
            "conversation event limit reached; history retained".into(),
        ));
    }
    conn.execute("INSERT INTO conversation_events(conversation_id,event_type,payload,created_at) VALUES(?1,?2,?3,?4)",params![id,kind,serde_json::to_string(payload)?,now()])?;
    Ok(())
}
impl FactoryDb {
    pub fn conversation_fail_unplanned_run(&self, run: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE runs SET status='failed',updated_at=?2 WHERE id=?1 AND status='planning'",
            params![run, now()],
        )?;
        Ok(())
    }
    pub fn conversation_create(
        &self,
        id: &str,
        project: &str,
        title: &str,
    ) -> Result<Conversation> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM conversations", [], |r| r.get(0))?;
        if count >= 10000 {
            return Err(DbError::Corrupt(
                "conversation limit reached; archive/export existing history".into(),
            ));
        }
        tx.execute("INSERT INTO conversations(id,project_id,title,created_at,updated_at) VALUES(?1,?2,?3,?4,?4)",params![id,project,title,now()])?;
        tx.commit()?;
        self.conversation_get(id)?
            .ok_or(DbError::NotFound("conversation"))
    }
    pub fn conversation_get(&self, id: &str) -> Result<Option<Conversation>> {
        Ok(self.conn.query_row("SELECT id,project_id,title,status,revision,created_at,updated_at FROM conversations WHERE id=?1",[id],conversation).optional()?)
    }
    pub fn conversation_list(
        &self,
        project: &str,
        before: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Conversation>> {
        Ok(self.conn.prepare("SELECT id,project_id,title,status,revision,created_at,updated_at FROM conversations WHERE project_id=?1 AND (?2 IS NULL OR (created_at,id)<(SELECT created_at,id FROM conversations WHERE id=?2)) ORDER BY created_at DESC,id DESC LIMIT ?3")?.query_map(params![project,before,limit.min(100)],conversation)?.collect::<std::result::Result<_,_>>()?)
    }
    pub fn conversation_update(
        &self,
        id: &str,
        title: &str,
        status: &str,
        revision: i64,
    ) -> Result<Conversation> {
        let changed = self.conn.execute("UPDATE conversations SET title=?2,status=?3,revision=revision+1,updated_at=?4 WHERE id=?1 AND revision=?5",params![id,title,status,now(),revision])?;
        if changed == 0 {
            return Err(DbError::Conflict {
                expected: revision,
                current: self
                    .conversation_get(id)?
                    .ok_or(DbError::NotFound("conversation"))?
                    .revision,
            });
        }
        self.conversation_get(id)?
            .ok_or(DbError::NotFound("conversation"))
    }
    pub fn conversation_messages(
        &self,
        id: &str,
        before: Option<i64>,
        limit: usize,
    ) -> Result<Vec<ConversationMessage>> {
        Ok(self.conn.prepare("SELECT id,conversation_id,role,content,status,run_id,parent_message_id,provenance,created_at FROM conversation_messages WHERE conversation_id=?1 AND (?2 IS NULL OR id<?2) ORDER BY id DESC LIMIT ?3")?.query_map(params![id,before,limit.min(100)],message)?.collect::<std::result::Result<_,_>>()?)
    }
    pub fn conversation_command(&self, id: i64) -> Result<ConversationCommand> {
        self.conn.query_row("SELECT id,conversation_id,message_id,response_id,command_key,fingerprint,kind,state,payload,run_id,error FROM conversation_commands WHERE id=?1",[id],command).optional()?.ok_or(DbError::NotFound("conversation command"))
    }
    pub fn conversation_command_key(
        &self,
        id: &str,
        key: &str,
    ) -> Result<Option<ConversationCommand>> {
        Ok(self.conn.query_row("SELECT id,conversation_id,message_id,response_id,command_key,fingerprint,kind,state,payload,run_id,error FROM conversation_commands WHERE conversation_id=?1 AND command_key=?2",params![id,key],command).optional()?)
    }
    pub fn conversation_accept(
        &self,
        id: &str,
        key: &str,
        fingerprint: &str,
        kind: &str,
        content: &str,
        payload: &Value,
    ) -> Result<(ConversationCommand, bool)> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        storage_budget(&tx)?;
        if let Some(existing) = self.conversation_command_key(id, key)? {
            if existing.fingerprint != fingerprint {
                return Err(DbError::Corrupt(
                    "idempotency key reused with different content".into(),
                ));
            }
            return Ok((existing, false));
        }
        let busy:i64=tx.query_row("SELECT COUNT(*) FROM conversation_commands WHERE conversation_id=?1 AND state IN ('accepted','processing','planning','running')",[id],|r|r.get(0))?;
        if busy > 0 {
            return Err(DbError::Corrupt(
                "conversation is busy; wait or cancel the active command".into(),
            ));
        }
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM conversation_messages WHERE conversation_id=?1",
            [id],
            |r| r.get(0),
        )?;
        if count >= 10000 {
            return Err(DbError::Corrupt(
                "message limit reached; start another conversation".into(),
            ));
        }
        tx.execute("INSERT INTO conversation_messages(conversation_id,role,content,status,provenance,created_at) VALUES(?1,'user',?2,'accepted',?3,?4)",params![id,content,serde_json::to_string(payload)?,now()])?;
        let user = tx.last_insert_rowid();
        tx.execute("INSERT INTO conversation_messages(conversation_id,role,content,status,parent_message_id,provenance,created_at) VALUES(?1,'orchestrator','','processing',?2,'{}',?3)",params![id,user,now()])?;
        let response = tx.last_insert_rowid();
        tx.execute("INSERT INTO conversation_commands(conversation_id,message_id,response_id,command_key,fingerprint,kind,state,payload) VALUES(?1,?2,?3,?4,?5,?6,'accepted',?7)",params![id,user,response,key,fingerprint,kind,serde_json::to_string(payload)?])?;
        let command_id = tx.last_insert_rowid();
        tx.execute(
            "UPDATE conversation_messages SET provenance=?2 WHERE id=?1",
            params![
                response,
                serde_json::to_string(&json!({"command_id":command_id}))?
            ],
        )?;
        event(
            &tx,
            id,
            "conversation.message.accepted",
            &json!({"message_id":user,"response_id":response,"command_id":command_id}),
        )?;
        tx.execute(
            "UPDATE conversations SET updated_at=?2 WHERE id=?1",
            params![id, now()],
        )?;
        tx.commit()?;
        Ok((self.conversation_command(command_id)?, true))
    }
    pub fn conversation_state(
        &self,
        command_id: i64,
        state: &str,
        content: &str,
        provenance: &Value,
        error: Option<&str>,
    ) -> Result<()> {
        self.write_conversation_state(command_id, state, content, provenance, error, None)
    }

    pub fn conversation_finish_stopped_agent(
        &self,
        command_id: i64,
        state: &str,
        content: &str,
        error: &str,
        session: Option<i64>,
    ) -> Result<()> {
        if !matches!(state, "cancelled" | "timed_out" | "interrupted") {
            return Err(DbError::Corrupt("invalid conversation stop state".into()));
        }
        self.write_conversation_state(
            command_id,
            state,
            content,
            &json!({"source":"coordinator","agent_session_id":session}),
            Some(error),
            session,
        )
    }

    fn write_conversation_state(
        &self,
        command_id: i64,
        state: &str,
        content: &str,
        provenance: &Value,
        error: Option<&str>,
        stopped_session: Option<i64>,
    ) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let command = self.conversation_command(command_id)?;
        if matches!(
            command.state.as_str(),
            "finished" | "failed" | "interrupted" | "cancelled" | "timed_out" | "blocked"
        ) {
            return Ok(());
        }
        if let Some(session) = stopped_session {
            let changed = tx.execute(
                "UPDATE agent_sessions SET status=?2 WHERE id=?1 AND run_id IS NULL AND status IN ('cancelled','timed_out')",
                params![session, state],
            )?;
            if changed != 1 {
                return Err(DbError::NotFound("stopped conversation agent session"));
            }
        }
        let mut provenance = provenance.clone();
        if let Some(map) = provenance.as_object_mut() {
            map.insert("command_id".into(), json!(command_id));
        }
        tx.execute(
            "UPDATE conversation_commands SET state=?2,error=?3 WHERE id=?1",
            params![command_id, state, error],
        )?;
        tx.execute("UPDATE conversation_messages SET content=?2,status=?3,run_id=?4,provenance=?5 WHERE id=?1",params![command.response_id,content,state,command.run_id,serde_json::to_string(&provenance)?])?;
        tx.execute(
            "UPDATE conversation_messages SET status=?2,run_id=?3 WHERE id=?1",
            params![command.message_id, state, command.run_id],
        )?;
        event(
            &tx,
            &command.conversation_id,
            match state {
                "processing" => "conversation.response.started",
                "finished" => "conversation.response.completed",
                "failed" | "interrupted" => "conversation.response.failed",
                "cancelled" => "conversation.response.cancelled",
                "timed_out" => "conversation.response.timed_out",
                "blocked" => "conversation.response.blocked",
                _ => "conversation.response.updated",
            },
            &json!({"command_id":command_id,"message_id":command.response_id,"state":state,"error":error}),
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn conversation_begin_run(
        &self,
        command_id: i64,
        objective: &str,
        team: &WorkflowTeam,
        recipe: Option<&RecipeSnapshot>,
    ) -> Result<Run> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let command = self.conversation_command(command_id)?;
        if let Some(id) = command.run_id {
            return self.get_run(id)?.ok_or(DbError::NotFound("run"));
        }
        if command.state != "processing" {
            return Err(DbError::Corrupt(
                "command cannot create a workflow in this state".into(),
            ));
        }
        tx.execute("INSERT INTO runs(objective,status,planner_agent,team,recipe,created_at,updated_at) VALUES(?1,'planning',?2,?3,?4,?5,?5)",params![objective,team.planner,serde_json::to_string(team)?,recipe.map(serde_json::to_string).transpose()?,now()])?;
        let run_id = tx.last_insert_rowid();
        self.attach_workspace_mission(run_id, &command)?;
        tx.execute(
            "UPDATE conversation_commands SET run_id=?2,state='planning' WHERE id=?1",
            params![command_id, run_id],
        )?;
        tx.execute(
            "UPDATE conversation_messages SET run_id=?2 WHERE id IN (?1,?3)",
            params![command.message_id, run_id, command.response_id],
        )?;
        tx.execute("INSERT INTO conversation_missions(run_id,conversation_id,command_id,created_at) VALUES(?1,?2,?3,?4)",params![run_id,command.conversation_id,command_id,now()])?;
        event(
            &tx,
            &command.conversation_id,
            "mission.created",
            &json!({"run_id":run_id,"command_id":command_id}),
        )?;
        tx.commit()?;
        self.get_run(run_id)?.ok_or(DbError::NotFound("run"))
    }
    pub fn conversation_missions(&self, id: &str) -> Result<Vec<i64>> {
        Ok(self.conn.prepare("SELECT run_id FROM conversation_missions WHERE conversation_id=?1 ORDER BY run_id DESC LIMIT 100")?.query_map([id],|r|r.get(0))?.collect::<std::result::Result<_,_>>()?)
    }
    pub fn conversation_observation_ids(&self, id: &str) -> Result<Vec<i64>> {
        Ok(self.conn.prepare("SELECT cm.run_id FROM conversation_missions cm JOIN runs r ON r.id=cm.run_id WHERE cm.conversation_id=?1 AND (r.status IN ('planning','active') OR COALESCE(json_extract(NULLIF(cm.snapshot,''),'$.status'),'')<>r.status OR EXISTS(SELECT 1 FROM mission_lifecycle l WHERE l.run_id=r.id AND l.state='settled' AND l.checkpoint_id IS NULL AND l.generation>1 AND EXISTS(SELECT 1 FROM scheduler_events e WHERE e.run_id=l.run_id AND e.event_type='mission.retry_requested' AND json_extract(e.payload,'$.generation')=l.generation) AND NOT EXISTS(SELECT 1 FROM scheduler_events e WHERE e.run_id=l.run_id AND e.event_type='mission.execution_settled' AND json_extract(e.payload,'$.generation')=l.generation))) ORDER BY cm.run_id DESC LIMIT 100")?.query_map([id],|r|r.get(0))?.collect::<std::result::Result<_,_>>()?)
    }
    pub fn conversation_observe(&self, run: i64, snapshot: &Value) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let association: Option<(String, String)> = tx
            .query_row(
                "SELECT conversation_id,snapshot FROM conversation_missions WHERE run_id=?1",
                [run],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((id, previous)) = association {
            let current_status: String =
                tx.query_row("SELECT status FROM runs WHERE id=?1", [run], |row| {
                    row.get(0)
                })?;
            if snapshot["status"].as_str() != Some(current_status.as_str()) {
                tx.commit()?;
                return Ok(());
            }
            let next = serde_json::to_string(snapshot)?;
            if let Some(life) = self.mission_lifecycle(run)? {
                if snapshot["lifecycle"]["generation"]
                    .as_i64()
                    .is_some_and(|generation| generation < life.generation)
                    || snapshot["lifecycle"]["state"]
                        .as_str()
                        .is_some_and(|state| state != life.state.as_str())
                {
                    tx.commit()?;
                    return Ok(());
                }
            }
            let previous_value: Value = serde_json::from_str(&previous).unwrap_or(Value::Null);
            let old_revision = previous_value["plan_revision"].as_i64().unwrap_or(1);
            let new_revision = snapshot["plan_revision"].as_i64().unwrap_or(1);
            if new_revision < old_revision {
                tx.commit()?;
                return Ok(());
            }
            if previous != next {
                tx.execute(
                    "UPDATE conversation_missions SET snapshot=?2 WHERE run_id=?1",
                    params![run, next],
                )?;
                if new_revision > old_revision {
                    let content=format!("Mission #{run} continued with plan revision {new_revision}. Active work was reconciled before the remaining DAG changed. Completed task history is preserved. Inspect Plan history in Progress for the reason and task changes.");
                    tx.execute("INSERT INTO conversation_messages(conversation_id,role,content,status,run_id,provenance,created_at) VALUES(?1,'orchestrator',?2,'finished',?3,?4,?5)",params![id,content,run,json!({"source":"runtime_replan","revision":new_revision}).to_string(),now()])?;
                    event(
                        &tx,
                        &id,
                        "mission.plan_revised",
                        &json!({"run_id":run,"revision":new_revision}),
                    )?;
                }
                event(&tx, &id, "mission.updated", snapshot)?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    pub fn conversation_events(
        &self,
        id: &str,
        after: i64,
        limit: usize,
    ) -> Result<Vec<ConversationEvent>> {
        let rows=self.conn.prepare("SELECT id,conversation_id,event_type,payload,created_at FROM conversation_events WHERE conversation_id=?1 AND id>?2 ORDER BY id LIMIT ?3")?.query_map(params![id,after,limit.min(100)],|r| Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        rows.into_iter()
            .map(|(id, conversation_id, event_type, payload, timestamp)| {
                Ok(ConversationEvent {
                    schema_version: 1,
                    id,
                    conversation_id,
                    event_type,
                    payload: serde_json::from_str(&payload)?,
                    timestamp,
                })
            })
            .collect()
    }
    pub fn conversation_reconcile(&self) -> Result<()> {
        let ids:Vec<i64>=self.conn.prepare("SELECT id FROM conversation_commands WHERE state IN ('accepted','processing','planning','running')")?.query_map([],|r|r.get(0))?.collect::<std::result::Result<_,_>>()?;
        for id in ids {
            self.conversation_state(id,"interrupted","The server stopped before this interaction finished. History is retained. Inspect the linked workflow before submitting a new command; no action was automatically repeated.",&json!({"source":"recovery"}),Some("server_restart"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn late_observer_cannot_regress_a_revision_or_duplicate_its_notice() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("db.sqlite3");
        let db = FactoryDb::open(&path).unwrap();
        db.conversation_create("c_revision", "p_revision", "Revision")
            .unwrap();
        let (command, _) = db
            .conversation_accept(
                "c_revision",
                "key",
                "fingerprint",
                "chat",
                "Objective",
                &json!({}),
            )
            .unwrap();
        db.conversation_state(command.id, "processing", "Processing", &json!({}), None)
            .unwrap();
        let run = db
            .conversation_begin_run(
                command.id,
                "Objective",
                &WorkflowTeam {
                    planner: "planner".into(),
                    workers: vec!["worker".into()],
                    reviewers: vec!["reviewer".into()],
                    additional: Default::default(),
                },
                None,
            )
            .unwrap();
        let delayed = FactoryDb::open(&path).unwrap();
        db.set_run_status(run.id, factory_types::RunStatus::Active)
            .unwrap();
        let fresh = json!({"run_id":run.id,"status":"active","plan_revision":2});
        db.conversation_observe(run.id, &fresh).unwrap();
        delayed
            .conversation_observe(
                run.id,
                &json!({"run_id":run.id,"status":"active","plan_revision":1}),
            )
            .unwrap();
        db.conversation_observe(run.id, &fresh).unwrap();
        db.set_run_status(run.id, factory_types::RunStatus::Completed)
            .unwrap();
        let completed = json!({"run_id":run.id,"status":"completed","plan_revision":2});
        db.conversation_observe(run.id, &completed).unwrap();
        delayed.conversation_observe(run.id, &fresh).unwrap();
        let stored: String = db
            .conn
            .query_row(
                "SELECT snapshot FROM conversation_missions WHERE run_id=?1",
                [run.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&stored).unwrap(), completed);
        let messages = db.conversation_messages("c_revision", None, 50).unwrap();
        assert_eq!(
            messages
                .iter()
                .filter(|message| message.provenance["source"] == "runtime_replan")
                .count(),
            1
        );
        assert_eq!(
            db.conversation_events("c_revision", 0, 100)
                .unwrap()
                .iter()
                .filter(|event| event.event_type == "mission.plan_revised")
                .count(),
            1
        );
    }

    #[test]
    fn conversation_writer_waits_before_reading_a_snapshot() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("db.sqlite3");
        let blocker = FactoryDb::open(&path).unwrap();
        blocker
            .conversation_create("c_test", "p_test", "Test")
            .unwrap();
        let (command, _) = blocker
            .conversation_accept(
                "c_test",
                "key",
                "fingerprint",
                "chat",
                "Question",
                &json!({}),
            )
            .unwrap();
        let writer = FactoryDb::open(&path).unwrap();
        let lock =
            Transaction::new_unchecked(&blocker.conn, TransactionBehavior::Immediate).unwrap();
        let (entered, entry) = mpsc::channel();
        let (completed, completion) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            entered.send(()).unwrap();
            let result =
                writer.conversation_state(command.id, "finished", "Answer", &json!({}), None);
            completed.send(result).unwrap();
        });
        entry.recv().unwrap();
        assert!(matches!(
            completion.recv_timeout(Duration::from_millis(150)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        lock.commit().unwrap();
        completion
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        thread.join().unwrap();
        assert_eq!(
            blocker.conversation_command(command.id).unwrap().state,
            "finished"
        );
    }
}
