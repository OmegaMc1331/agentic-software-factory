use crate::{DbError, FactoryDb, Result};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

pub(crate) const SCHEMA: &str = "
CREATE TABLE application_notifications(
 id INTEGER PRIMARY KEY AUTOINCREMENT, source_event_id TEXT NOT NULL UNIQUE,
 event_type TEXT NOT NULL, category TEXT NOT NULL, severity TEXT NOT NULL,
 title TEXT NOT NULL, message TEXT NOT NULL, run_id INTEGER REFERENCES runs(id),
 task_id INTEGER REFERENCES tasks(id), target TEXT NOT NULL,
 created_at TEXT NOT NULL, read_at TEXT, dismissed_at TEXT, revision INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX notification_unread ON application_notifications(dismissed_at,read_at,id);
CREATE TRIGGER notification_retention AFTER INSERT ON application_notifications BEGIN
 DELETE FROM application_notifications WHERE id <= (SELECT id FROM application_notifications ORDER BY id DESC LIMIT 1 OFFSET 1000);
END;
CREATE TRIGGER notification_run AFTER UPDATE OF status ON runs
WHEN new.status<>old.status AND new.status IN ('completed','failed','blocked') BEGIN
 INSERT OR IGNORE INTO application_notifications(source_event_id,event_type,category,severity,title,message,run_id,target,created_at)
 VALUES('run:'||new.id||':'||new.updated_at||':'||new.status,'mission.'||new.status,'mission',
 CASE new.status WHEN 'completed' THEN 'success' WHEN 'blocked' THEN 'warning' ELSE 'danger' END,
 CASE new.status WHEN 'completed' THEN 'Mission completed' WHEN 'blocked' THEN 'Mission needs attention' ELSE 'Mission failed' END,
 'Inspect the recorded task, test, review and integration evidence.',new.id,'mission',new.updated_at);
END;
CREATE TRIGGER notification_scheduler AFTER INSERT ON scheduler_events
WHEN new.event_type IN ('agent_handoff_finished','agent_handoff_rejected','integration_refused','recovery_exhausted','execution_interrupted') BEGIN
 INSERT OR IGNORE INTO application_notifications(source_event_id,event_type,category,severity,title,message,run_id,task_id,target,created_at)
 VALUES('scheduler:'||new.id,new.event_type,
 CASE WHEN new.event_type LIKE 'agent_handoff%' THEN 'handoff' WHEN new.event_type='integration_refused' THEN 'git' ELSE 'mission' END,
 CASE WHEN new.event_type='agent_handoff_finished' AND json_extract(new.payload,'$.state')='completed' THEN 'success' ELSE 'warning' END,
 CASE new.event_type WHEN 'agent_handoff_finished' THEN 'Agent handoff finished' WHEN 'agent_handoff_rejected' THEN 'Agent replacement rejected' WHEN 'integration_refused' THEN 'Integration needs attention' ELSE 'Mission recovery needs attention' END,
 'Open the authoritative record for its actual outcome and recovery options.',new.run_id,new.task_id,
 CASE WHEN new.event_type='integration_refused' THEN 'review' ELSE 'mission' END,new.created_at);
END;
CREATE TRIGGER notification_lifecycle AFTER UPDATE OF state ON mission_lifecycle
WHEN new.state<>old.state AND new.state IN ('suspended','recovery_required') BEGIN
 INSERT OR IGNORE INTO application_notifications(source_event_id,event_type,category,severity,title,message,run_id,target,created_at)
 VALUES('lifecycle:'||new.run_id||':'||new.generation||':'||new.state,'mission.'||new.state,'lifecycle',
 CASE new.state WHEN 'suspended' THEN 'info' ELSE 'warning' END,
 CASE new.state WHEN 'suspended' THEN 'Mission suspended; manual resume available' ELSE 'Mission recovery required' END,
 'Inspect the checkpoint and current permissions before explicitly resuming.',new.run_id,'mission',new.updated_at);
END;
CREATE TRIGGER notification_delivery AFTER UPDATE OF state ON github_deliveries
WHEN new.state<>old.state AND new.state IN ('published','failed','interrupted') BEGIN
 INSERT OR IGNORE INTO application_notifications(source_event_id,event_type,category,severity,title,message,run_id,target,created_at)
 VALUES('delivery:'||new.id||':'||new.updated_at||':'||new.state,'delivery.'||new.state,'github',
 CASE WHEN new.state IN ('published') THEN 'success' ELSE 'warning' END,
 'GitHub delivery state changed','Inspect the persisted remote identity and delivery result. Local integration and remote publication are distinct.',new.run_id,'review',new.updated_at);
END;
";

fn notification(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"id":row.get::<_,i64>(0)?,"source_event_id":row.get::<_,String>(1)?,"event_type":row.get::<_,String>(2)?,"category":row.get::<_,String>(3)?,"severity":row.get::<_,String>(4)?,"title":row.get::<_,String>(5)?,"message":row.get::<_,String>(6)?,"run_id":row.get::<_,Option<i64>>(7)?,"task_id":row.get::<_,Option<i64>>(8)?,"target":row.get::<_,String>(9)?,"created_at":row.get::<_,String>(10)?,"read_at":row.get::<_,Option<String>>(11)?,"dismissed_at":row.get::<_,Option<String>>(12)?,"revision":row.get::<_,i64>(13)?,"schema_version":1}),
    )
}

const COLUMNS: &str = "id,source_event_id,event_type,category,severity,title,message,run_id,task_id,target,created_at,read_at,dismissed_at,revision";

#[derive(Clone, Copy)]
pub enum ServiceNotification {
    SaveConflict,
    IdeFailure,
    ExtensionFailure,
    ApprovalRequired,
}

impl FactoryDb {
    pub fn notification_service(&self, kind: ServiceNotification, source_hash: &str) -> Result<()> {
        if source_hash.len() != 64 || !source_hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(DbError::Corrupt("Invalid notification provenance".into()));
        }
        let (event, category, title, message) = match kind {
            ServiceNotification::SaveConflict => ("file.save_conflict", "editor", "File save conflict", "The save was refused because its disk revision changed. Return to the editor to compare and preserve both versions."),
            ServiceNotification::IdeFailure => ("ide.operation_failed", "ide", "IDE service needs attention", "A requested IDE operation failed. Inspect the service state and approved local tool configuration before retrying."),
            ServiceNotification::ExtensionFailure => ("extension.operation_failed", "extension", "Extension operation failed", "A requested extension operation failed. Inspect the extension state and its approved configuration."),
            ServiceNotification::ApprovalRequired => ("ide.approval_required", "permission", "IDE permission or approval required", "An IDE operation was denied. Review the actual project policy and local tool approval; this notification grants no permissions."),
        };
        self.conn.execute("INSERT OR IGNORE INTO application_notifications(source_event_id,event_type,category,severity,title,message,target,created_at) VALUES(?1,?2,?3,'warning',?4,?5,'settings',?6)",params![format!("service:{event}:{source_hash}"),event,category,title,message,crate::now()])?;
        Ok(())
    }
    pub fn notification_page(
        &self,
        before: Option<i64>,
        limit: usize,
        archived: bool,
    ) -> Result<Value> {
        let limit = limit.clamp(1, 100);
        let mut q = self.conn.prepare(&format!("SELECT {COLUMNS} FROM application_notifications WHERE (?1 IS NULL OR id<?1) AND (?2 OR dismissed_at IS NULL) ORDER BY id DESC LIMIT ?3"))?;
        let mut items = q
            .query_map(params![before, archived, limit + 1], notification)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let next = if items.len() > limit {
            items.truncate(limit);
            items.last().and_then(|r| r["id"].as_i64())
        } else {
            None
        };
        let unread: i64 = self.conn.query_row("SELECT COUNT(*) FROM application_notifications WHERE dismissed_at IS NULL AND read_at IS NULL", [], |r|r.get(0))?;
        Ok(json!({"items":items,"next_cursor":next,"unread":unread,"retention_limit":1000}))
    }

    pub fn notification_update(
        &self,
        id: i64,
        revision: i64,
        read: bool,
        dismissed: bool,
    ) -> Result<Value> {
        let changed = self.conn.execute("UPDATE application_notifications SET read_at=CASE WHEN ?3 THEN COALESCE(read_at,?5) ELSE NULL END,dismissed_at=CASE WHEN ?4 THEN COALESCE(dismissed_at,?5) ELSE NULL END,revision=revision+1 WHERE id=?1 AND revision=?2",params![id,revision,read,dismissed,crate::now()])?;
        let result = self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM application_notifications WHERE id=?1"),
                [id],
                notification,
            )
            .optional()?
            .ok_or(DbError::NotFound("notification"))?;
        if changed == 0
            && (result["read_at"].is_null() == read
                || result["dismissed_at"].is_null() == dismissed)
        {
            return Err(DbError::Conflict {
                expected: revision,
                current: result["revision"].as_i64().unwrap_or(0),
            });
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use factory_types::RunStatus;

    #[test]
    fn typed_service_failures_are_private_bounded_and_deduplicated() {
        let dir = tempfile::tempdir().unwrap();
        let db = FactoryDb::open(&dir.path().join("db")).unwrap();
        let digest = crate::memory_hash("owned failed operation");
        for _ in 0..2 {
            db.notification_service(ServiceNotification::SaveConflict, &digest)
                .unwrap();
        }
        assert!(db
            .notification_service(ServiceNotification::IdeFailure, "untrusted raw output")
            .is_err());
        let page = db.notification_page(None, 50, false).unwrap();
        assert_eq!(page["items"].as_array().unwrap().len(), 1);
        assert_eq!(page["items"][0]["event_type"], "file.save_conflict");
        assert!(!page.to_string().contains("owned failed operation"));
    }

    #[test]
    fn actual_transitions_are_durable_deduplicated_private_and_revision_checked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite3");
        let db = FactoryDb::open(&path).unwrap();
        let run = db
            .create_run("sensitive objective sk-example", None)
            .unwrap()
            .id;
        db.set_run_status(run, RunStatus::Completed).unwrap();
        db.set_run_status(run, RunStatus::Completed).unwrap();
        let page = db.notification_page(None, 50, false).unwrap();
        assert_eq!(page["items"].as_array().unwrap().len(), 1);
        assert!(!page.to_string().contains("sensitive objective"));
        let id = page["items"][0]["id"].as_i64().unwrap();
        db.notification_update(id, 1, true, false).unwrap();
        db.notification_update(id, 1, true, false).unwrap();
        assert!(matches!(
            db.notification_update(id, 1, false, false),
            Err(DbError::Conflict { .. })
        ));
        drop(db);
        let db = FactoryDb::open(&path).unwrap();
        assert_eq!(db.notification_page(None, 50, false).unwrap()["unread"], 0);
        db.notification_update(id, 2, true, true).unwrap();
        assert!(db.notification_page(None, 50, false).unwrap()["items"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(
            db.notification_page(None, 50, true).unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        db.scheduler_event(
            run,
            None,
            "agent_handoff_rejected",
            &json!({"error":"private raw text"}),
        )
        .unwrap();
        assert!(!db
            .notification_page(None, 50, true)
            .unwrap()
            .to_string()
            .contains("private raw text"));
    }

    #[test]
    fn retention_and_paging_do_not_delete_runtime_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let db = FactoryDb::open(&dir.path().join("db")).unwrap();
        let run = db.create_run("owned", None).unwrap().id;
        for _ in 0..1010 {
            db.scheduler_event(run, None, "execution_interrupted", &json!({}))
                .unwrap();
        }
        let page = db.notification_page(None, 100, false).unwrap();
        assert_eq!(page["unread"], 1000);
        assert_eq!(page["items"].as_array().unwrap().len(), 100);
        let cursor = page["next_cursor"].as_i64();
        let next = db.notification_page(cursor, 100, false).unwrap();
        assert!(next["items"][0]["id"].as_i64() < cursor);
        assert_eq!(
            db.conn
                .query_row("SELECT COUNT(*) FROM scheduler_events", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1010
        );
    }
}
