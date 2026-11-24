use crate::{now, DbError, FactoryDb, Result};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

pub(crate) const SCHEMA: &str = "
CREATE TABLE source_control_operations(
 id INTEGER PRIMARY KEY, command_key TEXT NOT NULL UNIQUE, request_hash TEXT NOT NULL,
 kind TEXT NOT NULL, root_id TEXT NOT NULL, state TEXT NOT NULL,
 result TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE TABLE human_review_comments(
 id INTEGER PRIMARY KEY, command_key TEXT NOT NULL UNIQUE, run_id INTEGER NOT NULL REFERENCES runs(id),
 attempt_id INTEGER NOT NULL REFERENCES task_attempts(id), base_sha TEXT NOT NULL, head_sha TEXT NOT NULL,
 path TEXT, line INTEGER, side TEXT, decision TEXT NOT NULL, body TEXT NOT NULL,
 actor TEXT NOT NULL, created_at TEXT NOT NULL
);
CREATE INDEX human_review_attempt ON human_review_comments(attempt_id,id);
CREATE TABLE review_context_usage(session_id INTEGER PRIMARY KEY REFERENCES agent_sessions(id), evidence TEXT NOT NULL, created_at TEXT NOT NULL);
";

impl FactoryDb {
    pub fn source_integration_base(&self, run: i64) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT before_sha FROM integration_records WHERE run_id=?1 ORDER BY id LIMIT 1",
                [run],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn source_review_list(&self, run: Option<i64>, before: i64) -> Result<Vec<Value>> {
        let mut query=self.conn.prepare("SELECT a.id,a.task_id,a.attempt_number,a.agent,a.status,t.run_id,substr(r.objective,1,240),a.role,a.operation FROM task_attempts a JOIN tasks t ON t.id=a.task_id JOIN runs r ON r.id=t.run_id WHERE (?1 IS NULL OR t.run_id=?1) AND a.id<?2 ORDER BY a.id DESC LIMIT 100")?;
        let values=query.query_map(params![run,before],|row|Ok(json!({"runId":row.get::<_,i64>(5)?,"objective":row.get::<_,String>(6)?,"attempt":{"id":row.get::<_,i64>(0)?,"taskId":row.get::<_,i64>(1)?,"attemptNumber":row.get::<_,i64>(2)?,"agent":row.get::<_,String>(3)?,"status":row.get::<_,String>(4)?,"role":row.get::<_,Option<String>>(7)?,"operation":row.get::<_,Option<String>>(8)?}})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        Ok(values)
    }

    pub fn source_review_sessions(&self, attempt: i64) -> Result<Vec<Value>> {
        let mut query=self.conn.prepare("SELECT s.id,s.agent,s.role,s.status,s.policy_audit,s.output_meta,u.evidence FROM agent_sessions s LEFT JOIN review_context_usage u ON u.session_id=s.id WHERE s.attempt_id=?1 ORDER BY s.id LIMIT 200")?;
        let values=query.query_map([attempt],|row|{
            let parsed=|index|->rusqlite::Result<Option<Value>> {let text:Option<String>=row.get(index)?;text.map(|text|serde_json::from_str(&text)).transpose().map_err(|error|rusqlite::Error::FromSqlConversionFailure(index,rusqlite::types::Type::Text,Box::new(error)))};
            Ok(json!({"id":row.get::<_,i64>(0)?,"agent":row.get::<_,String>(1)?,"role":row.get::<_,String>(2)?,"status":row.get::<_,String>(3)?,"policy":parsed(4)?,"outputMetadata":parsed(5)?,"reviewContext":parsed(6)?}))
        })?.collect::<std::result::Result<Vec<_>,_>>()?;
        Ok(values)
    }

    pub fn source_integrations(&self, run: i64, before: i64) -> Result<Vec<Value>> {
        let mut query=self.conn.prepare("SELECT id,task_id,attempt_id,before_sha,source_sha,candidate_sha,status,details,worktree_path,created_at FROM integration_records WHERE run_id=?1 AND id<?2 ORDER BY id DESC LIMIT 100")?;
        let records=query.query_map(params![run,before],|row|{
            let raw:String=row.get(7)?;
            let details=if raw.len()<=64*1024 {serde_json::from_str(&raw).map_err(|error|rusqlite::Error::FromSqlConversionFailure(7,rusqlite::types::Type::Text,Box::new(error)))?} else {json!({"limited":true,"bytes":raw.len(),"notice":"Detailed evidence exceeds the Source Control page limit; inspect the owning mission"})};
            Ok(json!({"id":row.get::<_,i64>(0)?,"run_id":run,"task_id":row.get::<_,i64>(1)?,"attempt_id":row.get::<_,i64>(2)?,"before_sha":row.get::<_,String>(3)?,"source_sha":row.get::<_,String>(4)?,"candidate_sha":row.get::<_,Option<String>>(5)?,"status":row.get::<_,String>(6)?,"details":details,"worktree_path":row.get::<_,String>(8)?,"created_at":row.get::<_,String>(9)?}))
        })?.collect::<std::result::Result<Vec<_>,_>>()?;
        Ok(records)
    }
    pub fn record_review_context(&self, session: i64, evidence: &Value) -> Result<()> {
        self.conn.execute(
            "INSERT INTO review_context_usage(session_id,evidence,created_at) VALUES(?1,?2,?3)",
            params![session, serde_json::to_string(evidence)?, now()],
        )?;
        Ok(())
    }

    pub fn review_context_usage(&self, session: i64) -> Result<Option<Value>> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT evidence FROM review_context_usage WHERE session_id=?1",
                [session],
                |r| r.get(0),
            )
            .optional()?;
        value
            .map(|v| serde_json::from_str(&v).map_err(Into::into))
            .transpose()
    }
    pub fn source_operation(&self, key: &str) -> Result<Option<Value>> {
        self.conn.query_row("SELECT id,request_hash,kind,root_id,state,result,created_at,updated_at FROM source_control_operations WHERE command_key=?1",[key],|row| {
            let result:Option<String>=row.get(5)?;
            let result:Option<Value>=result.map(|value|serde_json::from_str(&value)).transpose().map_err(|error|rusqlite::Error::FromSqlConversionFailure(5,rusqlite::types::Type::Text,Box::new(error)))?;
            Ok(json!({"id":row.get::<_,i64>(0)?,"requestHash":row.get::<_,String>(1)?,"kind":row.get::<_,String>(2)?,"rootId":row.get::<_,String>(3)?,"state":row.get::<_,String>(4)?,"result":result,"createdAt":row.get::<_,String>(6)?,"updatedAt":row.get::<_,String>(7)?}))
        }).optional().map_err(Into::into)
    }

    pub fn claim_source_operation(
        &self,
        key: &str,
        request_hash: &str,
        kind: &str,
        root: &str,
    ) -> Result<bool> {
        let count: i64 =
            self.conn
                .query_row("SELECT count(*) FROM source_control_operations", [], |r| {
                    r.get(0)
                })?;
        if count >= 100_000 {
            return Err(DbError::Busy("Source-control audit retention limit".into()));
        }
        Ok(self.conn.execute("INSERT OR IGNORE INTO source_control_operations(command_key,request_hash,kind,root_id,state,created_at,updated_at) VALUES(?1,?2,?3,?4,'executing',?5,?5)",params![key,request_hash,kind,root,now()])?==1)
    }

    pub fn finish_source_operation(&self, key: &str, state: &str, result: &Value) -> Result<()> {
        self.conn.execute("UPDATE source_control_operations SET state=?2,result=?3,updated_at=?4 WHERE command_key=?1 AND state='executing'",params![key,state,serde_json::to_string(result)?,now()])?;
        Ok(())
    }

    pub fn source_operations(&self, before: i64) -> Result<Vec<Value>> {
        let mut query=self.conn.prepare("SELECT command_key FROM source_control_operations WHERE id<?1 ORDER BY id DESC LIMIT 100")?;
        let keys = query
            .query_map([before], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        keys.iter()
            .map(|key| {
                self.source_operation(key)?
                    .ok_or(DbError::NotFound("source operation"))
            })
            .collect()
    }

    pub fn add_human_review(
        &self,
        key: &str,
        run: i64,
        attempt: i64,
        anchor: &Value,
        decision: &str,
        body: &str,
    ) -> Result<Value> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let original = self
            .get_task_attempt(attempt)?
            .ok_or(DbError::NotFound("attempt"))?;
        let task = self
            .get_task(original.task_id)?
            .ok_or(DbError::NotFound("task"))?;
        if task.run_id != run {
            return Err(DbError::NotFound("attempt"));
        }
        let count: i64 = self.conn.query_row(
            "SELECT count(*) FROM human_review_comments WHERE run_id=?1",
            [run],
            |r| r.get(0),
        )?;
        if count >= 2000 {
            return Err(DbError::Busy("Review comment retention limit".into()));
        }
        let base = anchor["base"]
            .as_str()
            .ok_or(DbError::NotFound("review base"))?;
        let head = anchor["head"]
            .as_str()
            .ok_or(DbError::NotFound("review head"))?;
        if decision == "request_changes" {
            let publishing:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM integration_records WHERE attempt_id=?1 AND status='publishing')",[attempt],|row|row.get(0))?;
            let delivering:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM github_deliveries WHERE run_id=?1 AND state IN ('pushing','creating_pr'))",[run],|row|row.get(0))?;
            if publishing || delivering {
                return Err(DbError::Busy("Integration publication already owns this diff; retry feedback after reconciliation".into()));
            }
        }
        self.conn.execute("INSERT OR IGNORE INTO human_review_comments(command_key,run_id,attempt_id,base_sha,head_sha,path,line,side,decision,body,actor,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'authenticated_local_user',?11)",params![key,run,attempt,base,head,anchor["path"].as_str(),anchor["line"].as_i64(),anchor["side"].as_str(),decision,body,now()])?;
        let record = self
            .human_reviews(attempt)?
            .into_iter()
            .find(|v| v["commandKey"] == key)
            .ok_or(DbError::NotFound("comment"))?;
        if record["body"] != body
            || record["base"] != base
            || record["head"] != head
            || record["decision"] != decision
            || record["path"] != anchor["path"]
            || record["line"] != anchor["line"]
            || record["side"] != anchor["side"]
        {
            return Err(DbError::Busy(
                "Review command key already has a different payload".into(),
            ));
        }
        transaction.commit()?;
        Ok(record)
    }

    pub fn human_review_blocks(&self, attempt: i64) -> Result<bool> {
        let decision:Option<String>=self.conn.query_row("SELECT decision FROM human_review_comments WHERE attempt_id=?1 AND decision IN ('request_changes','approve_annotation') ORDER BY id DESC LIMIT 1",[attempt],|row|row.get(0)).optional()?;
        Ok(decision.as_deref() == Some("request_changes"))
    }

    pub fn human_reviews(&self, attempt: i64) -> Result<Vec<Value>> {
        let mut query=self.conn.prepare("SELECT id,command_key,run_id,base_sha,head_sha,path,line,side,decision,body,actor,created_at FROM human_review_comments WHERE attempt_id=?1 ORDER BY id LIMIT 2000")?;
        let records = query.query_map([attempt],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"commandKey":r.get::<_,String>(1)?,"runId":r.get::<_,i64>(2)?,"attemptId":attempt,"base":r.get::<_,String>(3)?,"head":r.get::<_,String>(4)?,"path":r.get::<_,Option<String>>(5)?,"line":r.get::<_,Option<i64>>(6)?,"side":r.get::<_,Option<String>>(7)?,"decision":r.get::<_,String>(8)?,"body":r.get::<_,String>(9)?,"actor":r.get::<_,String>(10)?,"createdAt":r.get::<_,String>(11)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        Ok(records)
    }
}
