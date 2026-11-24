use crate::{AgentEntry, Agents, Config, Factory, FactoryError, RoleAssignment};
use factory_agent::{AgentKind, CommandAgent};
use factory_db::FactoryDb;
use factory_types::{AgentHandoff, AgentProfile, AgentProfileInput, HandoffRequest};
use serde_json::{json, Value};
use std::path::Path;

fn invalid(reason: impl Into<String>) -> FactoryError {
    FactoryError::AgentValidationFailed(format!("PROFILE_INVALID: {}", reason.into()))
}

pub fn model_identifier_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_./:@-".contains(&c))
}

pub(crate) fn observed_model(stderr: &str) -> Option<String> {
    stderr.lines().take(40).find_map(|line| {
        line.strip_prefix("model: ")
            .map(str::trim)
            .filter(|value| model_identifier_valid(value))
            .map(str::to_string)
    })
}

pub fn replace_argument(args: &mut Vec<String>, flags: &[&str], value: &str) {
    let mut skip = false;
    args.retain(|arg| {
        if skip {
            skip = false;
            return false;
        }
        if flags.contains(&arg.as_str()) {
            skip = true;
            return false;
        }
        !flags
            .iter()
            .any(|flag| arg.starts_with(&format!("{flag}=")))
    });
    args.extend([flags[0].into(), value.into()]);
}

fn configured_entry(
    base: &AgentEntry,
    input: &AgentProfileInput,
) -> Result<AgentEntry, FactoryError> {
    let mut entry = base.clone();
    let kind = base.effective_kind();
    entry.args = base.effective_args(kind);
    if entry.args.iter().any(|a| {
        a == "--"
            || a.contains("bypass")
            || a.contains("danger-full-access")
            || a == "--ignore-rules"
    }) {
        return Err(invalid(
            "Installation uses unsupported or unsafe automation arguments",
        ));
    }
    if let Some(model) = &input.model_id {
        if !model_identifier_valid(model) {
            return Err(invalid("Unsafe model identifier"));
        }
        if kind == AgentKind::Custom {
            return Err(invalid(
                "This adapter supports provider-default models only",
            ));
        }
        replace_argument(&mut entry.args, &["--model", "-m"], model);
    }
    if let Some(effort) = &input.reasoning_effort {
        if kind != AgentKind::Codex
            || !["low", "medium", "high", "xhigh", "max", "ultra"].contains(&effort.as_str())
        {
            return Err(invalid("Unsupported reasoning configuration"));
        }
        entry
            .args
            .extend(["-c".into(), format!("model_reasoning_effort={effort}")]);
    }
    entry.capabilities = input.roles.clone();
    entry.max_concurrency = Some(base.concurrency().min(input.max_concurrency));
    Ok(entry)
}

pub fn validate_profile(config: &Config, input: &AgentProfileInput) -> Result<(), FactoryError> {
    if input.name.trim().is_empty()
        || input.name.len() > 100
        || input.name.chars().any(char::is_control)
        || input.description.len() > 2000
        || input.instructions.len() > 16384
        || input.instructions.contains('\0')
        || !(1..=32).contains(&input.max_concurrency)
        || !(10..=3600).contains(&input.timeout_secs)
        || input.roles.is_empty()
        || input.roles.len() > 16
    {
        return Err(invalid("Bounded name, instructions, roles, concurrency (1-32) and deadline (10-3600 seconds) are required"));
    }
    input
        .permissions
        .validate("agent profile")
        .map_err(invalid)?;
    let entry = config
        .agents
        .get(&input.provider_adapter_id)
        .ok_or_else(|| invalid("Unknown trusted provider installation"))?;
    if config
        .profile_bindings
        .contains_key(&input.provider_adapter_id)
    {
        return Err(invalid(
            "A provider must be a trusted installation, not another profile",
        ));
    }
    let catalog = config.catalog();
    let mut seen = std::collections::BTreeSet::new();
    for role in &input.roles {
        if catalog.get(role).is_none()
            || !seen.insert(role)
            || (!entry.capabilities.is_empty() && !entry.capabilities.contains(role))
        {
            return Err(invalid(
                "Unsupported or duplicate role for this installation",
            ));
        }
    }
    configured_entry(entry, input)?;
    Ok(())
}

pub(crate) fn load_agents(root: &Path, db: &FactoryDb) -> Result<Agents, FactoryError> {
    let mut config = Config::load_and_migrate(root)?;
    let profiles = db.agent_profiles(false)?;
    for profile in profiles {
        let input = &profile.configuration;
        if !input.enabled {
            continue;
        }
        if validate_profile(&config, input).is_err() {
            continue;
        }
        let base = config
            .agents
            .get(&input.provider_adapter_id)
            .expect("validated installation");
        let entry = configured_entry(base, input)?;
        if let Some(policy) = config
            .policies
            .agents
            .get(&input.provider_adapter_id)
            .cloned()
        {
            config.policies.agents.insert(profile.id.clone(), policy);
        }
        config.agents.insert(profile.id.clone(), entry);
        for role in &input.roles {
            if input.preferred && input.automatic_delegation {
                for assignment in &mut config.role_assignments {
                    if &assignment.role == role {
                        assignment.preferred = false;
                    }
                }
            }
            config.role_assignments.push(RoleAssignment {
                role: role.clone(),
                agent: profile.id.clone(),
                preferred: input.preferred && input.automatic_delegation,
            });
        }
        config.profile_bindings.insert(profile.id.clone(), profile);
    }
    Ok(Agents::from_config(config))
}

impl Factory {
    pub(crate) fn integration_factory(&self, task: i64) -> Result<Self, FactoryError> {
        let mut current = self.task_factory()?;
        let Some(attempt) = self.db.latest_task_attempt(task)? else {
            return Ok(current);
        };
        let Some(snapshot) = self.db.attempt_execution_snapshot(
            attempt.id,
            &attempt.agent,
            attempt.role.as_deref().unwrap_or("worker"),
        )?
        else {
            return Ok(current);
        };
        let mut config = current.agents.config().clone();
        config
            .profile_policy_snapshots
            .insert(snapshot.profile.id.clone(), snapshot.policy_configuration);
        config
            .profile_bindings
            .insert(snapshot.profile.id.clone(), snapshot.profile);
        current.agents = Agents::from_config(config);
        Ok(current)
    }

    pub fn agent_profiles(&self, archived: bool) -> Result<Vec<AgentProfile>, FactoryError> {
        Ok(self.db.agent_profiles(archived)?)
    }

    pub fn save_agent_profile(
        &self,
        id: Option<&str>,
        expected: Option<i64>,
        input: AgentProfileInput,
    ) -> Result<AgentProfile, FactoryError> {
        self.execution.validate_root()?;
        let base = Config::load_and_migrate(&self.root)?;
        validate_profile(&base, &input)?;
        let ts = chrono::Utc::now().to_rfc3339();
        let mut profile = match id {
            Some(id) => self.db.agent_profile(id)?,
            None => AgentProfile {
                id: format!("ap_{}", uuid::Uuid::new_v4().simple()),
                project_id: self.execution.project_id().to_owned(),
                revision: 0,
                created_at: ts.clone(),
                updated_at: ts.clone(),
                archived: false,
                configuration: input.clone(),
            },
        };
        if profile.archived {
            return Err(invalid("Archived profiles cannot be edited"));
        }
        if input.preferred
            && input.enabled
            && input.automatic_delegation
            && self.db.agent_profiles(false)?.iter().any(|p| {
                p.id != profile.id
                    && p.configuration.enabled
                    && p.configuration.preferred
                    && p.configuration.automatic_delegation
                    && p.configuration
                        .roles
                        .iter()
                        .any(|role| input.roles.contains(role))
            })
        {
            return Err(FactoryError::RoutingOverride(
                "Another custom profile is already preferred for this role".into(),
            ));
        }
        profile.configuration = input;
        profile.revision += 1;
        profile.updated_at = ts;
        self.db.save_agent_profile(&profile, expected)?;
        Ok(profile)
    }

    pub fn archive_agent_profile(
        &self,
        id: &str,
        expected: i64,
    ) -> Result<AgentProfile, FactoryError> {
        self.execution.validate_root()?;
        let mut profile = self.db.agent_profile(id)?;
        profile.revision += 1;
        profile.archived = true;
        profile.configuration.enabled = false;
        profile.updated_at = chrono::Utc::now().to_rfc3339();
        self.db.save_agent_profile(&profile, Some(expected))?;
        Ok(profile)
    }

    pub fn agent_profile_details(&self, id: &str) -> Result<Value, FactoryError> {
        let profile = self.db.agent_profile(id)?;
        let agents = load_agents(&self.root, &self.db)?;
        let eligible = profile.configuration.enabled
            && !profile.archived
            && agents.config().profile_bindings.contains_key(id);
        let mut config = agents.config().clone();
        if let Some(scope) = config
            .policies
            .agents
            .get(&profile.configuration.provider_adapter_id)
            .cloned()
        {
            config.policies.agents.insert(id.into(), scope);
        }
        config.profile_bindings.insert(id.into(), profile.clone());
        let policies: Vec<Value> = profile
            .configuration
            .roles
            .iter()
            .map(|role| json!({"role":role,"policy":config.effective_policy(role,id).view()}))
            .collect();
        Ok(
            json!({"profile":profile,"effectivePermissions":policies,"eligible":eligible,"modelValidation":"configured; execution unverified until a successful session","hostEnforcement":"CLI/OS restrictions apply independently; application policy is not an OS sandbox"}),
        )
    }

    pub fn providers(&self) -> Result<Value, FactoryError> {
        let config = Config::load_and_migrate(&self.root)?;
        let providers: Vec<Value> = config.agents.iter().map(|(id,entry)| {
            let agent = CommandAgent::new(config.agent_config(id).expect("configured installation"));
            json!({"id":id,"kind":entry.effective_kind(),"available":agent.resolve_executable().is_ok(),"modelSelection":entry.effective_kind()!=AgentKind::Custom,"modelDiscovery":false,"reasoningSelection":entry.effective_kind()==AgentKind::Codex,"processCancellation":true,"sessionContinuation":false,"maxConcurrency":entry.concurrency(),"authentication":"existing CLI installation; credentials are not stored in profiles","roles":entry.capabilities})
        }).collect();
        Ok(
            json!({"providers":providers,"roles":config.role_infos(),"legacyAgents":Agents::from_config(config).list()}),
        )
    }

    pub fn request_agent_handoff(
        &self,
        run_id: i64,
        task_id: i64,
        request: HandoffRequest,
    ) -> Result<AgentHandoff, FactoryError> {
        self.execution.validate_root()?;
        if request.idempotency_key.is_empty()
            || request.idempotency_key.len() > 128
            || !request
                .idempotency_key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            || request.reason.trim().is_empty()
            || request.reason.len() > 2000
        {
            return Err(invalid("Bounded idempotency key and reason required"));
        }
        if let Some(existing) = self.db.handoff_by_key(run_id, &request.idempotency_key)? {
            if existing.task_id != task_id
                || existing.target.id != request.to_profile_id
                || existing.target.revision != request.expected_profile_revision
                || existing.reason != request.reason
            {
                return Err(FactoryError::RoutingOverride(
                    "Idempotency key payload conflict".into(),
                ));
            }
            if existing.state == "rejected" {
                return Err(FactoryError::PolicyBlocked(format!(
                    "Handoff {} was rejected: {}",
                    existing.id,
                    existing.evidence["reason"]
                        .as_str()
                        .unwrap_or("authorization denied")
                )));
            }
            return Ok(existing);
        }
        let task = self
            .db
            .get_task(task_id)?
            .ok_or(FactoryError::TaskNotFound(task_id))?;
        if task.run_id != run_id {
            return Err(FactoryError::PolicyBlocked(
                "PROJECT_MISMATCH: task does not belong to this mission".into(),
            ));
        }
        let run = self
            .db
            .get_run(run_id)?
            .ok_or(FactoryError::RunNotFound(run_id))?;
        let role = task.role.as_deref().unwrap_or("worker");
        let operation =
            crate::roles::resolve_task_operation(&task, &self.agents.config().catalog());
        if !matches!(
            operation,
            factory_types::TaskOperation::Implement
                | factory_types::TaskOperation::Verify
                | factory_types::TaskOperation::PostProcess
        ) && !matches!(
            task.state,
            factory_types::TaskState::Pending | factory_types::TaskState::Ready
        ) {
            return Err(FactoryError::PolicyBlocked("HANDOFF_UNSUPPORTED: active operation cannot be interrupted; running handoffs require an implementation-family task with captured Git evidence".into()));
        }
        let profile = self.db.agent_profile(&request.to_profile_id)?;
        if profile.revision != request.expected_profile_revision {
            return Err(factory_db::DbError::Conflict {
                expected: request.expected_profile_revision,
                current: profile.revision,
            }
            .into());
        }
        let ts = chrono::Utc::now().to_rfc3339();
        let handoff = AgentHandoff {
            id: format!("ho_{}", uuid::Uuid::new_v4().simple()),
            project_id: self.execution.project_id().to_owned(),
            run_id,
            task_id,
            idempotency_key: request.idempotency_key.clone(),
            from_agent: task.agent_override.clone(),
            from_session_id: None,
            from_attempt_id: None,
            target: profile.clone(),
            reason: request.reason.clone(),
            state: "waiting".into(),
            boundary: "pending_validation".into(),
            requested_at: ts.clone(),
            updated_at: ts,
            replacement_session_id: None,
            evidence: json!({}),
        };
        let reject = |reason: &str| -> Result<AgentHandoff, FactoryError> {
            self.db.record_rejected_handoff(&handoff, reason)?;
            Err(FactoryError::PolicyBlocked(format!(
                "Handoff {} rejected: {reason}",
                handoff.id
            )))
        };
        let agents = load_agents(&self.root, &self.db)?;
        if !profile.configuration.enabled || profile.archived {
            return reject("PROFILE_DISABLED: replacement profile is disabled or archived");
        }
        if !profile.configuration.roles.iter().any(|r| r == role) {
            return reject(
                "ROLE_UNSUPPORTED: replacement profile is not assigned to the task role",
            );
        }
        agents.command_agent_for(role, &profile.id)?;
        let mut policy = agents.config().effective_policy(role, &profile.id);
        if let Some(restriction) = run
            .recipe
            .as_ref()
            .and_then(|r| crate::recipes::snapshot_policy_restriction(r, role))
        {
            policy = policy.narrowed("recipe", &restriction);
        }
        if let Err(reason) = factory_policy::validate_executable(&policy, operation.as_str()) {
            return reject(&reason);
        }
        Ok(self.db.request_handoff(&handoff)?)
    }
}
