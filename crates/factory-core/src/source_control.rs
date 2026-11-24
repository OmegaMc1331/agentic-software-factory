use factory_git::source_control::{digest, validate_path, validate_sha};
use factory_git::{GitError, Repo};
use serde_json::{json, Value};

use crate::{Factory, FactoryError};

impl Factory {
    pub fn source_delivery_changes(&self, run: i64) -> Result<Value, FactoryError> {
        self.get_run(run)?.ok_or(FactoryError::RunNotFound(run))?;
        let recorded = self.db.get_run_base(run)?;
        let base = match recorded.as_ref() {
            Some(base) => Some(base.clone()),
            None => self.db.source_integration_base(run)?,
        };
        let head = self.db.get_run_integration(run)?;
        let (Some(base), Some(head)) = (base, head) else {
            return Ok(
                json!({"changedFiles":[],"changedFilesComplete":false,"comparisonBase":null,"baseSource":"unavailable"}),
            );
        };
        let paths =
            Repo::detect_bounded(&self.root, &self.root)?.source_changed_paths(&base, &head)?;
        Ok(
            json!({"changedFiles":paths,"changedFilesComplete":true,"comparisonBase":base,"baseSource":if recorded.is_some(){"recorded_run_base"}else{"first_integration_journal"}}),
        )
    }

    pub fn authorize_manual_delivery(&self, run: i64) -> Result<(), FactoryError> {
        let completed = self.db.mission_lifecycle(run)?.is_some_and(|life| {
            life.state == factory_types::MissionLifecycleState::Settled
                && life.project_id == self.execution.project_id()
        }) && self
            .get_run(run)?
            .is_some_and(|run| run.status == factory_types::RunStatus::Completed);
        if !completed {
            self.require_lifecycle_active(run)?;
        }
        let records = self.db.integration_records(run)?;
        if !records.is_empty() {
            let published: Vec<_> = records
                .iter()
                .filter(|record| record.status == "published")
                .collect();
            let head = self.db.get_run_integration(run)?;
            if published
                .last()
                .and_then(|record| record.candidate_sha.as_ref())
                != head.as_ref()
                || head.is_none()
                || self.db.list_tasks(run)?.iter().any(|task| {
                    !matches!(
                        task.state,
                        factory_types::TaskState::Completed | factory_types::TaskState::Superseded
                    )
                })
            {
                return Err(FactoryError::NotDeliverable(
                    "The integration journal and completed task state must match the delivery head"
                        .into(),
                ));
            }
            for record in &published {
                let attempt = self
                    .db
                    .get_task_attempt(record.attempt_id)?
                    .ok_or(FactoryError::TaskNotFound(record.task_id))?;
                if attempt.status != factory_types::AttemptStatus::Approved
                    || attempt.review.as_ref().is_none_or(|review| {
                        review.decision != factory_types::ReviewDecision::Approve
                    })
                    || self.db.human_review_blocks(attempt.id)?
                {
                    return Err(FactoryError::NotDeliverable("Every published task requires its recorded automatic approval and resolved human findings".into()));
                }
            }
            let checks = published
                .last()
                .and_then(|record| record.details["checks"].as_array());
            if checks.is_some_and(|checks| {
                checks
                    .iter()
                    .any(|check| check["passed"] != true || check["exitCode"] != 0)
            }) || self
                .agents()
                .config()
                .integration_checks
                .iter()
                .any(|required| {
                    checks.is_none_or(|checks| {
                        !checks.iter().any(|check| {
                            check["program"] == required.program
                                && check["args"] == json!(required.args)
                                && check["passed"] == true
                                && check["exitCode"] == 0
                        })
                    })
                })
            {
                return Err(FactoryError::NotDeliverable("Current required combined checks must have passing recorded evidence at the delivery head".into()));
            }
        }
        if let Some(ceiling) = self.agents().config().policies.ceiling.as_ref() {
            if let Some(commands) = &ceiling.commands {
                let matches = |pattern: &String| {
                    factory_policy::path::matches_glob(pattern, "git push", !cfg!(windows))
                };
                if commands.mode == factory_policy::CommandsMode::Denied
                    || commands.deny.iter().any(matches)
                    || commands.mode == factory_policy::CommandsMode::Restricted
                        && !commands.allow.iter().any(matches)
                {
                    return Err(FactoryError::PolicyBlocked(
                        "Project command ceiling denies delivery".into(),
                    ));
                }
            }
            if ceiling
                .network
                .as_ref()
                .is_some_and(|network| network.mode == factory_policy::NetworkMode::Deny)
            {
                return Err(FactoryError::PolicyBlocked(
                    "Project network ceiling denies GitHub delivery".into(),
                ));
            }
            if ceiling.git.as_ref().is_some_and(|git| {
                !git.allow.is_empty() && !git.allow.iter().any(|op| op == "push")
            }) {
                return Err(FactoryError::PolicyBlocked(
                    "Project Git ceiling denies publication".into(),
                ));
            }
        }
        Ok(())
    }
    pub fn review_evidence(&self, attempt_id: i64) -> Result<Value, FactoryError> {
        let attempt = self
            .db
            .get_task_attempt(attempt_id)?
            .ok_or(FactoryError::Db(factory_db::DbError::NotFound("attempt")))?;
        let task = self
            .get_task(attempt.task_id)?
            .ok_or(FactoryError::TaskNotFound(attempt.task_id))?;
        let run = self
            .get_run(task.run_id)?
            .ok_or(FactoryError::RunNotFound(task.run_id))?;
        let evidence = attempt.evidence.as_ref();
        let snapshot = evidence.and_then(|e| e.git_snapshot.as_ref());
        let mut verified = false;
        let mut actual_paths = Vec::new();
        if let Some(snapshot) = snapshot {
            validate_sha(&snapshot.base_sha)?;
            validate_sha(&snapshot.head_sha)?;
            let repo = Repo::detect_bounded(&self.root, &self.root)?;
            actual_paths = repo.source_changed_paths(&snapshot.base_sha, &snapshot.head_sha)?;
            verified = evidence
                .and_then(|e| e.diff_patch.as_deref())
                .is_some_and(|patch| digest(patch) == snapshot.patch_sha256);
        }
        let sessions = self.db.source_review_sessions(attempt_id)?;
        let latest = self.db.latest_task_attempt(task.id)?;
        let latest_head = latest
            .as_ref()
            .and_then(|a| a.evidence.as_ref())
            .and_then(|e| e.git_snapshot.as_ref())
            .map(|s| s.head_sha.as_str());
        let comments: Vec<_> = self
            .db
            .human_reviews(attempt_id)?
            .into_iter()
            .map(|mut comment| {
                comment["outdated"] = json!(latest_head != comment["head"].as_str());
                comment
            })
            .collect();
        Ok(
            json!({"schema":1,"run":run,"task":task,"attempt":attempt,"actualPaths":actual_paths,"patchHashVerified":verified,"sessions":sessions,"humanComments":comments,"integration":self.db.source_integrations(task.run_id,i64::MAX)?,"integrationPageLimit":100,"handoffs":self.db.handoffs(task.run_id)?,"authority":"Human change requests fence pending publication. Human annotations cannot replace configured automated review, checks or integration."}),
        )
    }

    pub fn comment_on_review(
        &self,
        attempt_id: i64,
        key: &str,
        anchor: &Value,
        decision: &str,
        body: &str,
    ) -> Result<Value, FactoryError> {
        if key.is_empty()
            || key.len() > 128
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_:".contains(&b))
            || body.trim().is_empty()
            || body.len() > 8192
            || !["comment", "request_changes", "approve_annotation"].contains(&decision)
        {
            return Err(GitError::Invalid("Invalid bounded human feedback".into()).into());
        }
        if anchor.as_object().is_none_or(|object| {
            object
                .keys()
                .any(|key| !["base", "head", "path", "line", "side"].contains(&key.as_str()))
        }) {
            return Err(GitError::Invalid("Invalid structured review anchor".into()).into());
        }
        let attempt = self
            .db
            .get_task_attempt(attempt_id)?
            .ok_or(FactoryError::Db(factory_db::DbError::NotFound("attempt")))?;
        let task = self
            .get_task(attempt.task_id)?
            .ok_or(FactoryError::TaskNotFound(attempt.task_id))?;
        let snapshot = attempt
            .evidence
            .as_ref()
            .and_then(|e| e.git_snapshot.as_ref())
            .ok_or_else(|| {
                GitError::Invalid("No recorded diff version exists for this attempt".into())
            })?;
        if anchor["base"] != snapshot.base_sha || anchor["head"] != snapshot.head_sha {
            return Err(GitError::Stale.into());
        }
        if let Some(path) = anchor["path"].as_str() {
            validate_path(path)?;
            let repo = Repo::detect_bounded(&self.root, &self.root)?;
            if !repo
                .source_changed_paths(&snapshot.base_sha, &snapshot.head_sha)?
                .iter()
                .any(|p| p == path)
            {
                return Err(
                    GitError::Invalid("Comment path is not in the reviewed diff".into()).into(),
                );
            }
            if let Some(line) = anchor["line"].as_i64() {
                if !(1..=100_000).contains(&line)
                    || !matches!(anchor["side"].as_str(), Some("base" | "head"))
                {
                    return Err(GitError::Invalid("Invalid comment anchor".into()).into());
                }
                let d = repo.source_diff(&factory_git::source_control::DiffRequest {
                    path: path.into(),
                    comparison: factory_git::source_control::Comparison::Commits,
                    base: Some(snapshot.base_sha.clone()),
                    target: Some(snapshot.head_sha.clone()),
                })?;
                let source = if anchor["side"] == "base" {
                    d.before
                } else {
                    d.after
                };
                if source.is_none_or(|s| line as usize > s.lines().count()) {
                    return Err(GitError::Invalid(
                        "Comment line is unavailable in the selected revision".into(),
                    )
                    .into());
                }
            }
        } else if !anchor["line"].is_null() {
            return Err(GitError::Invalid("Line requires a file".into()).into());
        }
        Ok(self
            .db
            .add_human_review(key, task.run_id, attempt_id, anchor, decision, body)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(status: factory_types::RunStatus) -> (tempfile::TempDir, Factory, i64) {
        let dir = tempfile::tempdir().unwrap();
        assert!(std::process::Command::new("git")
            .arg("init")
            .arg("-q")
            .arg(dir.path())
            .output()
            .unwrap()
            .status
            .success());
        Factory::init(dir.path()).unwrap();
        let factory = Factory::open(dir.path()).unwrap();
        let run = factory
            .db
            .create_run("Controlled delivery authorization", None)
            .unwrap()
            .id;
        let project = factory.execution.project_id().to_string();
        let session = format!("s_{}", "1".repeat(32));
        factory
            .db
            .register_workspace_session(&project, &session)
            .unwrap();
        factory
            .db
            .bind_workspace_run(run, &project, &session)
            .unwrap();
        factory.db.set_run_status(run, status).unwrap();
        factory.db.settle_mission_lifecycle(run).unwrap();
        (dir, factory, run)
    }
    #[test]
    fn completed_settlement_is_not_a_suspension_or_publication_authorization() {
        let (_dir, factory, run) = fixture(factory_types::RunStatus::Completed);
        assert!(factory.db.lifecycle_fenced(run).unwrap());
        factory.authorize_manual_delivery(run).unwrap();
        assert!(factory.begin_delivery(run, None, None, false).is_err());
    }
    #[test]
    fn failed_or_suspending_missions_cannot_deliver() {
        let (_dir, factory, run) = fixture(factory_types::RunStatus::Failed);
        assert!(factory.authorize_manual_delivery(run).is_err());
        let (_dir, factory, run) = fixture(factory_types::RunStatus::Active);
        factory
            .db
            .request_mission_suspend(run, 1, "explicit-suspension", "user_requested_suspend")
            .unwrap();
        assert!(factory.authorize_manual_delivery(run).is_err());
    }
    #[test]
    fn completed_mission_still_obeys_the_project_publication_ceiling() {
        let (dir, _factory, run) = fixture(factory_types::RunStatus::Completed);
        let mut config = crate::Config::default();
        config.policies.ceiling = Some(factory_policy::PolicyScope {
            network: Some(factory_policy::NetworkPolicy {
                mode: factory_policy::NetworkMode::Deny,
            }),
            ..Default::default()
        });
        config.write_atomic(dir.path()).unwrap();
        assert!(matches!(
            Factory::open(dir.path())
                .unwrap()
                .authorize_manual_delivery(run),
            Err(FactoryError::PolicyBlocked(_))
        ));
    }

    #[test]
    fn delivery_comparison_uses_verified_first_journal_when_run_base_is_missing() {
        let (dir, factory, run) = fixture(factory_types::RunStatus::Completed);
        assert_eq!(
            factory.source_delivery_changes(run).unwrap()["changedFilesComplete"],
            false
        );
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };
        git(&["config", "user.name", "Delivery fixture"]);
        git(&["config", "user.email", "fixture@example.invalid"]);
        git(&["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.path().join("source.py"), "VALUE = 0\n").unwrap();
        git(&["add", "source.py"]);
        git(&["commit", "-qm", "baseline"]);
        let base = git(&["rev-parse", "HEAD"]);
        std::fs::write(dir.path().join("source.py"), "VALUE = 1\n").unwrap();
        git(&["add", "source.py"]);
        git(&["commit", "-qm", "source correction"]);
        let head = git(&["rev-parse", "HEAD"]);
        let task = factory
            .db
            .create_task(
                run,
                "Correction",
                "Correct source",
                &[],
                factory_types::TaskState::Completed,
                0,
                Some("worker"),
                Some(factory_types::TaskOperation::Implement),
            )
            .unwrap();
        let attempt = factory
            .db
            .create_task_attempt(
                task,
                "worker",
                Some(factory_types::TaskOperation::Implement),
                "controlled",
                dir.path().to_str().unwrap(),
                Some(&base),
            )
            .unwrap();
        let journal = factory
            .db
            .begin_integration(run, task, attempt.id, &base, &head, "fixture-candidate")
            .unwrap();
        factory.db.set_run_integration(run, Some(&head)).unwrap();
        assert!(factory.db.get_run_base(run).unwrap().is_none());
        let result = factory.source_delivery_changes(run).unwrap();
        assert_eq!(result["changedFilesComplete"], true);
        assert_eq!(result["changedFiles"], json!(["source.py"]));
        assert_eq!(result["comparisonBase"], base);
        assert_eq!(result["baseSource"], "first_integration_journal");
        factory.db.set_run_base(run, Some(&base)).unwrap();
        assert_eq!(
            factory.source_delivery_changes(run).unwrap()["baseSource"],
            "recorded_run_base"
        );
        factory
            .db
            .update_integration_record(journal, "published", Some(&head), &json!({"checks":[]}))
            .unwrap();
        factory
            .db
            .finish_task_attempt(
                attempt.id,
                factory_types::AttemptStatus::Approved,
                Some(0),
                Some(&head),
                None,
                None,
                None,
            )
            .unwrap();
        assert!(factory.authorize_manual_delivery(run).is_err());
        let approval = factory_types::ReviewResult {
            decision: factory_types::ReviewDecision::Approve,
            reason: "Controlled review fixture".into(),
            feedback: vec![],
        };
        factory
            .db
            .finish_task_attempt(
                attempt.id,
                factory_types::AttemptStatus::Approved,
                Some(0),
                Some(&head),
                None,
                None,
                Some(&approval),
            )
            .unwrap();
        factory.authorize_manual_delivery(run).unwrap();
        factory
            .db
            .update_integration_record(
                journal,
                "published",
                Some(&head),
                &json!({"checks":[{"program":"python","args":[],"passed":false,"exitCode":1}]}),
            )
            .unwrap();
        assert!(factory.authorize_manual_delivery(run).is_err());
        factory
            .db
            .update_integration_record(journal, "published", Some(&head), &json!({"checks":[]}))
            .unwrap();
        let anchor = json!({"base":base,"head":head});
        factory
            .db
            .add_human_review(
                "changes",
                run,
                attempt.id,
                &anchor,
                "request_changes",
                "Unresolved controlled finding",
            )
            .unwrap();
        assert!(factory.authorize_manual_delivery(run).is_err());
        factory
            .db
            .add_human_review(
                "resolved",
                run,
                attempt.id,
                &anchor,
                "approve_annotation",
                "Resolved human finding; automatic approval remains intact",
            )
            .unwrap();
        factory.authorize_manual_delivery(run).unwrap();
        factory.db.get_or_create_delivery(run).unwrap();
        factory
            .db
            .claim_delivery(run, "owned-delivery", "owner/project", "origin", "main")
            .unwrap()
            .unwrap();
        assert!(factory
            .db
            .add_human_review(
                "late",
                run,
                attempt.id,
                &anchor,
                "request_changes",
                "Delivery already owns publication"
            )
            .is_err());
    }
}
