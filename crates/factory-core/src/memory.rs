use super::*;
use factory_types::memory::*;
use std::time::Duration;

impl Factory {
    pub fn project_memory(&self) -> Result<factory_memory::MemoryService<'_>, FactoryError> {
        self.execution.validate_root()?;
        let mut secrets = factory_policy::Secrets::new();
        for (key, value) in std::env::vars() {
            if ["TOKEN", "SECRET", "PASSWORD", "API_KEY"]
                .iter()
                .any(|part| key.to_ascii_uppercase().contains(part))
            {
                secrets.add(&value, 4);
            }
        }
        for agent in self.agents.config().agents.values() {
            for value in agent.env.values() {
                secrets.add(value, 4);
            }
        }
        Ok(factory_memory::MemoryService::new(
            &self.db,
            self.execution.project_id(),
            &self.root,
            secrets,
        )?)
    }

    pub(super) fn invocation_memory_context(
        &self,
        session: i64,
        scope: &InvocationScope<'_>,
        policy: &factory_policy::EffectivePolicy,
    ) -> Result<String, FactoryError> {
        if !self.execution.project_id().starts_with("p_") {
            return Ok(String::new());
        }
        let service = self.project_memory()?;
        let settings = service.settings()?;
        if let Some(id) = scope.memory_extraction_id {
            let extraction = self.db.memory_extraction(self.execution.project_id(), id)?;
            if extraction.state != "running"
                || extraction.agent_session_id != Some(session)
                || extraction.policy_snapshot.revision != settings.revision
                || !settings.enabled
                || settings.extraction_mode == ExtractionMode::Disabled
            {
                return Err(factory_memory::Error::Policy.into());
            }
            for source in extraction.source_references {
                service.resolve_source(&source.source, Some(policy))?;
            }
        }
        if !settings.enabled {
            return Ok(String::new());
        }
        let query = if let Some(query) = scope.memory_query {
            query.chars().take(3500).collect::<String>()
        } else if let Some(task) = scope.task_id {
            let task = self
                .db
                .get_task(task)?
                .ok_or(FactoryError::TaskNotFound(task))?;
            format!("{} {}", task.title, task.objective)
                .chars()
                .take(3500)
                .collect()
        } else if let Some(run) = scope.run_id {
            self.db
                .get_run(run)?
                .ok_or(FactoryError::RunNotFound(run))?
                .objective
                .chars()
                .take(3500)
                .collect()
        } else {
            String::new()
        };
        for _ in 0..3 {
            let context = match service.retrieve(&query, scope.role, Some(policy)) {
                Ok(context) => context,
                Err(factory_memory::Error::Db(
                    factory_db::DbError::Conflict { .. }
                    | factory_db::DbError::NotFound("project memory"),
                )) => continue,
                Err(error) => return Err(error.into()),
            };
            match service.record_usage(session, scope.role, &context) {
                Ok(rendered) => return Ok(rendered),
                Err(factory_memory::Error::Db(factory_db::DbError::Conflict { .. })) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(FactoryError::AgentValidationFailed(
            "Memory changed repeatedly during context selection; no process launched".into(),
        ))
    }

    pub fn extract_project_memory(
        &self,
        sources: &[MemorySource],
        cancel: &AtomicBool,
    ) -> Result<serde_json::Value, FactoryError> {
        self.extract_project_memory_inner(sources, cancel, None)
    }

    fn extract_project_memory_inner(
        &self,
        sources: &[MemorySource],
        cancel: &AtomicBool,
        managed_run: Option<i64>,
    ) -> Result<serde_json::Value, FactoryError> {
        let service = self.project_memory()?;
        let settings = service.settings()?;
        let resolved = service.extraction_sources(sources)?;
        for run in service.extraction_owner_runs(sources)? {
            let record = self
                .db
                .get_run(run)?
                .ok_or(FactoryError::RunNotFound(run))?;
            if managed_run != Some(run)
                && !matches!(
                    record.status,
                    RunStatus::Completed | RunStatus::Failed | RunStatus::Cancelled
                )
            {
                return Err(factory_memory::Error::Policy.into());
            }
            self.require_lifecycle_active(run)?;
        }
        let mut extraction = MemoryExtraction {
            id: format!("me_{}", uuid::Uuid::new_v4().simple()),
            project_id: self.execution.project_id().into(),
            source_references: resolved.iter().map(|s| s.provenance.clone()).collect(),
            policy_snapshot: settings.clone(),
            source_key: factory_db::memory_hash(
                &serde_json::to_string(
                    &resolved
                        .iter()
                        .map(|s| (&s.provenance.source, &s.provenance.fingerprint))
                        .collect::<Vec<_>>(),
                )
                .map_err(factory_db::DbError::from)?,
            ),
            state: "running".into(),
            agent_session_id: None,
            error: None,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        };
        if !self.db.memory_claim_extraction(&extraction)? {
            return Ok(
                serde_json::json!({"state":"already_recorded","notice":"This source revision was already attempted. No model request or retry occurred."}),
            );
        }
        let run = managed_run;
        let result = (|| -> Result<Vec<MemoryCandidate>, FactoryError> {
            let instruction = service.extraction_prompt(&resolved)?;
            let agent = self
                .agents
                .command_agent(roles::PLANNER)?
                .with_output_limit(32768)
                .with_timeout(Duration::from_secs(180));
            let _capacity = self.wait_agent_capacity(agent.name(), cancel)?;
            let invocation = self.invoke_with_agent(
                agent,
                InvocationScope {
                    memory_query: Some(""),
                    memory_extraction_id: Some(&extraction.id),
                    review_evidence: None,
                    run_id: run,
                    task_id: None,
                    attempt_id: None,
                    role: roles::PLANNER,
                    operation: Some(TaskOperation::Planning),
                    working_dir: &self.root,
                },
                &instruction,
                cancel,
            )?;
            extraction.agent_session_id = Some(invocation.session_id);
            if invocation.result.cancelled {
                return Err(FactoryError::Cancelled);
            }
            if invocation.result.exit_code != Some(0) {
                return Err(FactoryError::AgentProcess(AgentError::ExecutionFailed {
                    command: format!("memory extraction session #{}", invocation.session_id),
                    exit_code: invocation.result.exit_code.unwrap_or(-1),
                }));
            }
            if let Some(error) = agent_output_limit_error("memory extractor", &invocation.result) {
                return Err(error);
            }
            let session = self
                .db
                .get_agent_session(invocation.session_id)?
                .ok_or(factory_db::DbError::NotFound("memory extraction session"))?;
            Ok(service.accept_extracted(
                &extraction,
                &resolved,
                &session.stdout.unwrap_or_default(),
                settings.revision,
            )?)
        })();
        extraction.updated_at = Utc::now().to_rfc3339();
        extraction.state = if result.is_ok() {
            "completed"
        } else {
            "failed"
        }
        .into();
        extraction.error=result.as_ref().err().map(|error|match error {
            FactoryError::AgentResourceExhausted(_)=>"PROVIDER_USAGE_LIMIT: external availability blocker; no automatic retry",
            FactoryError::PolicyBlocked(_)|FactoryError::Suspended|FactoryError::Memory(factory_memory::Error::Policy)=>"POLICY_DENIED_OR_SUSPENDED: no automatic retry or policy change",
            FactoryError::Cancelled=>"INTERRUPTED: source and partial evidence retained; no automatic replay",
            FactoryError::Memory(_)=>"VALIDATION_FAILED: invalid claims, policy change or unresolved sources; no automatic retry",
            _=>"AGENT_OR_STORAGE_FAILED: inspect the linked session and retained evidence; no automatic retry",
        }.to_string());
        self.db.memory_finish_extraction(&extraction)?;
        result
            .map(|candidates| serde_json::json!({"extraction":extraction,"candidates":candidates}))
    }

    pub(super) fn automatic_project_memory(&self, run: i64, cancel: &AtomicBool) {
        if !self.execution.project_id().starts_with("p_") || cancel.load(Ordering::Acquire) {
            return;
        }
        let result = (|| -> Result<(), FactoryError> {
            let settings = self.project_memory()?.settings()?;
            if !settings.enabled || settings.extraction_mode == ExtractionMode::Disabled {
                return Ok(());
            }
            self.require_lifecycle_active(run)?;
            let Some(commit) = self.db.get_run_integration(run)? else {
                return Ok(());
            };
            let source = MemorySource {
                project_id: self.execution.project_id().into(),
                reference: MemorySourceRef::IntegrationCommit {
                    run_id: run,
                    commit_sha: commit,
                },
            };
            self.db.scheduler_event(run,None,"memory_extraction_started",&serde_json::json!({"mode":settings.extraction_mode,"scope":"verified integration evidence; not mission outcome"}))?;
            self.extract_project_memory_inner(&[source], cancel, Some(run))?;
            self.db.scheduler_event(run,None,"memory_extraction_finished",&serde_json::json!({"notice":"Inspect project Memory proposals. No policy was modified."}))?;
            Ok(())
        })();
        if result.is_err() {
            let _=self.db.scheduler_event(run,None,"memory_extraction_failed",&serde_json::json!({"notice":"Memory extraction failed or was interrupted; the underlying implementation result is unchanged. Inspect extraction history; no automatic retry."}));
        }
    }
}
