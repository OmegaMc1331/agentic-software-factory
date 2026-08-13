use factory_db::FactoryDb;
use factory_git::{Repo, WorktreeInfo};
use factory_types::{AgentSession, Run, Task, TaskState};
use thiserror::Error;

use crate::config::{AgentResolutionError, Agents, ConfigError};
use crate::planner::{normalize_plan, PlanError, PlanOutcome, Planner};

#[derive(Debug, Error)]
pub enum FactoryError {
    #[error("factory not initialized here; run `factory init` first")]
    NotInitialized,
    #[error("task {0} not found")]
    TaskNotFound(i64),
    #[error("invalid state transition: {0} -> {1}")]
    InvalidTransition(TaskState, TaskState),
    #[error("planning failed: {0}")]
    Plan(#[from] PlanError),
    #[error("agent resolution: {0}")]
    Agent(#[from] AgentResolutionError),
    #[error("configuration error: {0}")]
    Config(#[from] ConfigError),
    #[error("database error: {0}")]
    Db(#[from] factory_db::DbError),
    #[error("git error: {0}")]
    Git(#[from] factory_git::GitError),
    #[error("task {0} is not ready to run")]
    NotReady(i64),
    #[error("objective must not be empty")]
    EmptyObjective,
    #[error("io error: {0}")]
    Io(std::io::Error),
}

pub const FACTORY_DIR: &str = ".factory";

#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub run: Run,
    pub tasks: Vec<Task>,
}

#[derive(Debug, Clone)]
pub struct MarkOutcome {
    pub task: Task,
    pub from: TaskState,
    pub updated: Vec<i64>,
}

pub struct Factory {
    db: FactoryDb,
    agents: Agents,
    root: std::path::PathBuf,
}

impl Factory {
    /// Create the state directory, default config, and database, or open the
    /// existing ones. Idempotent: never destroys existing state.
    pub fn init(root: &std::path::Path) -> Result<Factory, FactoryError> {
        let factory_dir = root.join(FACTORY_DIR);
        std::fs::create_dir_all(&factory_dir).map_err(FactoryError::Io)?;
        crate::config::Config::ensure_default(root)?;
        let db_path = factory_dir.join("db.sqlite3");
        let db = FactoryDb::open(&db_path)?;
        Ok(Factory {
            db,
            agents: Agents::load(root)?,
            root: root.to_path_buf(),
        })
    }

    pub fn open(root: &std::path::Path) -> Result<Factory, FactoryError> {
        let db_path = root.join(FACTORY_DIR).join("db.sqlite3");
        if !db_path.exists() {
            return Err(FactoryError::NotInitialized);
        }
        let db = FactoryDb::open(&db_path)?;
        Ok(Factory {
            db,
            agents: Agents::load(root)?,
            root: root.to_path_buf(),
        })
    }

    pub fn planner_agent(&self) -> Result<String, FactoryError> {
        Ok(self.agents.command_agent("planner")?.name().to_string())
    }

    pub fn create_run(&self, objective: &str) -> Result<RunOutcome, FactoryError> {
        if objective.trim().is_empty() {
            return Err(FactoryError::EmptyObjective);
        }
        let planner_agent = self.agents.command_agent("planner")?;
        let planner = Planner::new(planner_agent);
        let outcome = planner.plan(objective, &self.root)?;
        let PlanOutcome {
            plan,
            agent,
            command,
            result,
        } = outcome;
        let plan = normalize_plan(plan);
        let run = self.db.create_run(&plan.objective, Some(&agent))?;
        self.persist_planner_session(&run, &agent, &command, &result)?;
        let mut id_by_label = std::collections::HashMap::new();
        for (index, task) in plan.tasks.iter().enumerate() {
            let id = self.db.create_task(
                run.id,
                &task.title,
                &task.objective,
                &task.acceptance_criteria,
                TaskState::Pending,
                index as i32,
            )?;
            id_by_label.insert(task.id.clone(), id);
        }
        for task in &plan.tasks {
            let task_id = id_by_label[&task.id];
            for dep in &task.dependencies {
                let dep_id = id_by_label[dep];
                self.db.add_dependency(task_id, dep_id)?;
            }
        }
        for task in &plan.tasks {
            let task_id = id_by_label[&task.id];
            let state = crate::workflow::Workflow::initial_state(!task.dependencies.is_empty());
            self.db.set_task_state(task_id, state)?;
        }
        let tasks = self.db.list_tasks(run.id)?;
        Ok(RunOutcome { run, tasks })
    }

    fn persist_planner_session(
        &self,
        run: &Run,
        agent: &str,
        command: &str,
        result: &factory_agent::AgentResult,
    ) -> Result<(), FactoryError> {
        let now = chrono::Utc::now().to_rfc3339();
        let status = if result.exit_code == Some(0) {
            "success"
        } else {
            "failed"
        };
        let session = AgentSession {
            id: 0,
            run_id: Some(run.id),
            task_id: None,
            role: "planner".to_string(),
            agent: agent.to_string(),
            command: command.to_string(),
            status: status.to_string(),
            started_at: now.clone(),
            finished_at: Some(now),
            exit_code: result.exit_code,
            duration_ms: Some(result.duration.as_millis() as u64),
            stdout: Some(result.stdout.clone()),
            stderr: Some(result.stderr.clone()),
        };
        self.db.insert_agent_session(&session)?;
        Ok(())
    }

    pub fn mark_task(&self, task_id: i64, target: TaskState) -> Result<MarkOutcome, FactoryError> {
        let task = self
            .db
            .get_task(task_id)?
            .ok_or(FactoryError::TaskNotFound(task_id))?;
        let from = task.state;
        if !crate::workflow::Workflow::can_transition(from, target) {
            return Err(FactoryError::InvalidTransition(from, target));
        }
        self.db.set_task_state(task_id, target)?;
        let mut updated = vec![task_id];
        let run_tasks = self.db.list_tasks(task.run_id)?;
        let mut state_of: std::collections::HashMap<i64, TaskState> =
            run_tasks.iter().map(|t| (t.id, t.state)).collect();
        let mut visited = std::collections::HashSet::new();
        visited.insert(task_id);
        let mut frontier = vec![task_id];
        while let Some(changed_id) = frontier.pop() {
            for dependent in run_tasks
                .iter()
                .filter(|t| t.dependencies.contains(&changed_id))
            {
                if visited.contains(&dependent.id) {
                    continue;
                }
                visited.insert(dependent.id);
                if matches!(dependent.state, TaskState::Completed | TaskState::Failed) {
                    continue;
                }
                let dep_states: Vec<TaskState> = dependent
                    .dependencies
                    .iter()
                    .map(|id| state_of[id])
                    .collect();
                let next = crate::workflow::Workflow::next_state_for_dependent(&dep_states);
                if next != dependent.state {
                    self.db.set_task_state(dependent.id, next)?;
                    state_of.insert(dependent.id, next);
                    updated.push(dependent.id);
                }
                frontier.push(dependent.id);
            }
        }
        let task = self
            .db
            .get_task(task_id)?
            .ok_or(FactoryError::TaskNotFound(task_id))?;
        Ok(MarkOutcome {
            task,
            from,
            updated,
        })
    }

    pub fn list_runs(&self) -> Result<Vec<Run>, FactoryError> {
        Ok(self.db.list_runs()?)
    }

    pub fn get_run(&self, id: i64) -> Result<Option<Run>, FactoryError> {
        Ok(self.db.get_run(id)?)
    }

    pub fn list_tasks(&self, run_id: i64) -> Result<Vec<Task>, FactoryError> {
        Ok(self.db.list_tasks(run_id)?)
    }

    pub fn get_task(&self, id: i64) -> Result<Option<Task>, FactoryError> {
        Ok(self.db.get_task(id)?)
    }

    pub fn list_agent_sessions(
        &self,
        run_id: Option<i64>,
    ) -> Result<Vec<AgentSession>, FactoryError> {
        Ok(self.db.list_agent_sessions(run_id)?)
    }

    pub fn worktree_dir(&self, task_id: i64) -> std::path::PathBuf {
        self.root
            .join(FACTORY_DIR)
            .join("worktrees")
            .join(format!("t{task_id}"))
    }

    pub fn create_worktree(&self, task_id: i64) -> Result<std::path::PathBuf, FactoryError> {
        let task = self
            .db
            .get_task(task_id)?
            .ok_or(FactoryError::TaskNotFound(task_id))?;
        if task.state != TaskState::Ready {
            return Err(FactoryError::NotReady(task_id));
        }
        let repo = Repo::detect(&self.root)?;
        let dir = self.worktree_dir(task_id);
        let branch = format!("factory/t{task_id}");
        repo.add_worktree(&dir, &branch)?;
        self.db.set_worktree_path(task_id, dir.to_str())?;
        Ok(dir)
    }

    pub fn remove_worktree(&self, task_id: i64, force: bool) -> Result<(), FactoryError> {
        let task = self
            .db
            .get_task(task_id)?
            .ok_or(FactoryError::TaskNotFound(task_id))?;
        let repo = Repo::detect(&self.root)?;
        let dir = self.worktree_dir(task_id);
        if force {
            if repo.find_worktree(&dir)?.is_some() || dir.exists() {
                repo.remove_worktree_force(&dir)?;
            }
        } else {
            if repo.find_worktree(&dir)?.is_some() || dir.exists() {
                repo.remove_worktree(&dir)?;
            }
        }
        if task.worktree_path.is_some() {
            self.db.set_worktree_path(task_id, None)?;
        }
        Ok(())
    }

    pub fn list_worktrees(&self) -> Result<Vec<WorktreeInfo>, FactoryError> {
        let repo = Repo::detect(&self.root)?;
        Ok(repo.list_worktrees()?)
    }
}
