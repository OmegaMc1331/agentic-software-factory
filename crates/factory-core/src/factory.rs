use factory_db::FactoryDb;
use factory_git::{Repo, WorktreeInfo};
use factory_models::{ModelUsage, Run, Task, TaskState};
use thiserror::Error;

use crate::planner::{normalize_plan, PlanError, PlanOutcome, Planner};
use crate::provider::Provider;

#[derive(Debug, Error)]
pub enum FactoryError {
    #[error("factory not initialized here; run `factory init` first")]
    NotInitialized,
    #[error("a factory is already initialized at {0}")]
    AlreadyInitialized(std::path::PathBuf),
    #[error("task {0} not found")]
    TaskNotFound(i64),
    #[error("invalid state transition: {0} -> {1}")]
    InvalidTransition(TaskState, TaskState),
    #[error("planning failed: {0}")]
    Plan(#[from] PlanError),
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
    planner: Planner,
    root: std::path::PathBuf,
}

impl Factory {
    pub fn init(
        root: &std::path::Path,
        force: bool,
        provider: Box<dyn Provider>,
    ) -> Result<Factory, FactoryError> {
        let factory_dir = root.join(FACTORY_DIR);
        std::fs::create_dir_all(&factory_dir).map_err(FactoryError::Io)?;
        let db_path = factory_dir.join("db.sqlite3");
        if db_path.exists() && !force {
            return Err(FactoryError::AlreadyInitialized(db_path));
        }
        let db = FactoryDb::open(&db_path)?;
        Ok(Factory {
            db,
            planner: Planner::new(provider),
            root: root.to_path_buf(),
        })
    }

    pub fn open(
        root: &std::path::Path,
        provider: Box<dyn Provider>,
    ) -> Result<Factory, FactoryError> {
        let db_path = root.join(FACTORY_DIR).join("db.sqlite3");
        if !db_path.exists() {
            return Err(FactoryError::NotInitialized);
        }
        let db = FactoryDb::open(&db_path)?;
        Ok(Factory {
            db,
            planner: Planner::new(provider),
            root: root.to_path_buf(),
        })
    }

    pub fn provider(&self) -> &str {
        self.planner.provider()
    }

    pub fn create_run(&self, objective: &str) -> Result<RunOutcome, FactoryError> {
        if objective.trim().is_empty() {
            return Err(FactoryError::EmptyObjective);
        }
        let PlanOutcome { plan, model, usage } = self.planner.plan(objective)?;
        let plan = normalize_plan(plan);
        let run = self.db.create_run(&plan.objective, Some(&model), &usage)?;
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

    pub fn remove_worktree(&self, task_id: i64) -> Result<(), FactoryError> {
        let task = self
            .db
            .get_task(task_id)?
            .ok_or(FactoryError::TaskNotFound(task_id))?;
        let repo = Repo::detect(&self.root)?;
        let dir = self.worktree_dir(task_id);
        if repo.find_worktree(&dir)?.is_some() || dir.exists() {
            repo.remove_worktree(&dir)?;
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

    pub fn usage_for_run(&self, run_id: i64) -> Result<ModelUsage, FactoryError> {
        let run = self
            .db
            .get_run(run_id)?
            .ok_or(FactoryError::TaskNotFound(run_id))?;
        Ok(ModelUsage {
            prompt_tokens: run.prompt_tokens,
            completion_tokens: run.completion_tokens,
            total_tokens: run.total_tokens,
        })
    }
}
