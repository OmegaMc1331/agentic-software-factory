use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::capacity::AgentCapacity;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationCommand {
    pub program: String,
    pub args: Vec<String>,
    pub timeout_secs: u64,
}

impl ValidationCommand {
    pub fn validate(&self) -> Result<(), String> {
        if self.program.trim().is_empty()
            || self.program.len() > 4096
            || self.program.contains('\0')
            || self.args.len() > 64
            || self
                .args
                .iter()
                .any(|arg| arg.len() > 8192 || arg.contains('\0'))
            || !(1..=300).contains(&self.timeout_secs)
        {
            return Err("Integration check requires a bounded executable, argv and a 1-300 second deadline.".into());
        }
        Ok(())
    }
}

pub struct TaskBudget {
    maximum: usize,
    state: Mutex<BudgetState>,
}

#[derive(Default)]
struct BudgetState {
    used: usize,
    projects: HashMap<String, usize>,
    runs: HashMap<(String, i64), usize>,
    queue: VecDeque<WaitingRun>,
    project_limits: HashMap<String, BTreeMap<usize, usize>>,
}

struct WaitingRun {
    key: (String, i64),
    project_limit: usize,
    run_limit: usize,
    seen: Instant,
}

pub struct TaskPermit {
    project_limit: usize,
    budget: Arc<TaskBudget>,
    key: (String, i64),
}

type RootCheck = dyn Fn() -> Result<(), String> + Send + Sync;

pub struct ProjectExecution {
    project_id: OnceLock<String>,
    root_check: OnceLock<Arc<RootCheck>>,
    pub(crate) agents: Arc<AgentCapacity>,
    pub(crate) budget: Arc<TaskBudget>,
    pub(crate) identity: String,
}

impl ProjectExecution {
    pub fn new(identity: String, budget: Arc<TaskBudget>) -> Self {
        Self {
            project_id: OnceLock::new(),
            root_check: OnceLock::new(),
            agents: Arc::new(AgentCapacity::new()),
            budget,
            identity,
        }
    }
}

impl ProjectExecution {
    pub fn bind_project_id(&self, project: &str) -> Result<(), String> {
        if !project.strip_prefix("p_").is_some_and(|id| {
            id.len() == 32
                && id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }) {
            return Err("Invalid authoritative project identity".into());
        }
        self.project_id
            .set(project.to_owned())
            .map_err(|_| "Project identity is already bound".into())
    }

    pub fn project_id(&self) -> &str {
        self.project_id
            .get()
            .map(String::as_str)
            .unwrap_or(&self.identity)
    }

    pub fn bind_root(&self, check: Arc<RootCheck>) -> Result<(), String> {
        self.root_check
            .set(check)
            .map_err(|_| "Project validation is already bound".into())
    }
    pub(crate) fn validate_root(&self) -> Result<(), crate::FactoryError> {
        if let Some(check) = self.root_check.get() {
            check().map_err(crate::FactoryError::PolicyBlocked)?;
        }
        Ok(())
    }
}

impl TaskBudget {
    pub fn new(maximum: usize) -> Self {
        Self {
            maximum,
            state: Mutex::new(BudgetState::default()),
        }
    }

    pub fn maximum(&self) -> usize {
        self.maximum
    }

    pub fn usage(&self) -> usize {
        self.state
            .lock()
            .map(|state| state.used)
            .unwrap_or(self.maximum)
    }

    pub fn unregister(&self, project: &str, run_id: i64) {
        if let Ok(mut state) = self.state.lock() {
            state
                .queue
                .retain(|entry| entry.key.0 != project || entry.key.1 != run_id);
        }
    }

    pub fn try_acquire(
        self: &Arc<Self>,
        project: &str,
        run_id: i64,
        project_limit: usize,
        run_limit: usize,
    ) -> Option<TaskPermit> {
        let mut state = self.state.lock().ok()?;
        let now = Instant::now();
        state
            .queue
            .retain(|entry| now.duration_since(entry.seen) < Duration::from_secs(2));
        let key = (project.to_owned(), run_id);
        match state.queue.iter_mut().find(|entry| entry.key == key) {
            Some(entry) => {
                entry.seen = now;
                entry.project_limit = project_limit;
                entry.run_limit = run_limit;
            }
            None => state.queue.push_back(WaitingRun {
                key: key.clone(),
                project_limit,
                run_limit,
                seen: now,
            }),
        }
        if state.used >= self.maximum {
            return None;
        }
        let index = state.queue.iter().position(|entry| {
            let active_limit = state
                .project_limits
                .get(&entry.key.0)
                .and_then(|limits| limits.keys().next())
                .copied()
                .unwrap_or(entry.project_limit)
                .min(entry.project_limit);
            state.projects.get(&entry.key.0).copied().unwrap_or(0) < active_limit
                && state.runs.get(&entry.key).copied().unwrap_or(0) < entry.run_limit
        })?;
        if state.queue[index].key != key {
            return None;
        }
        state.queue.remove(index)?;
        state.used += 1;
        *state
            .project_limits
            .entry(project.to_owned())
            .or_default()
            .entry(project_limit)
            .or_default() += 1;
        *state.projects.entry(project.to_owned()).or_default() += 1;
        *state.runs.entry(key.clone()).or_default() += 1;
        Some(TaskPermit {
            project_limit,
            budget: self.clone(),
            key,
        })
    }
}

impl Drop for TaskPermit {
    fn drop(&mut self) {
        if let Ok(mut state) = self.budget.state.lock() {
            state.used = state.used.saturating_sub(1);
            if let Some(limits) = state.project_limits.get_mut(&self.key.0) {
                if let Some(count) = limits.get_mut(&self.project_limit) {
                    *count -= 1;
                    if *count == 0 {
                        limits.remove(&self.project_limit);
                    }
                }
                if limits.is_empty() {
                    state.project_limits.remove(&self.key.0);
                }
            }
            if let Some(count) = state.projects.get_mut(&self.key.0) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    state.projects.remove(&self.key.0);
                }
            }
            if let Some(count) = state.runs.get_mut(&self.key) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    state.runs.remove(&self.key);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_and_application_caps_release_on_drop() {
        let budget = Arc::new(TaskBudget::new(2));
        let a = budget.try_acquire("a", 1, 1, 2).unwrap();
        assert!(budget.try_acquire("a", 2, 1, 2).is_none());
        let b = budget.try_acquire("b", 1, 1, 2).unwrap();
        assert!(budget.try_acquire("c", 1, 1, 2).is_none());
        drop(a);
        budget.unregister("a", 2);
        let c = budget.try_acquire("c", 1, 1, 2).unwrap();
        assert_eq!(budget.usage(), 2);
        drop((b, c));
        assert_eq!(budget.usage(), 0);
    }

    #[test]
    fn waiting_runs_take_turns_without_a_saturated_project_blocking_others() {
        let budget = Arc::new(TaskBudget::new(1));
        let a = budget.try_acquire("a", 1, 1, 1).unwrap();
        assert!(budget.try_acquire("b", 1, 1, 1).is_none());
        drop(a);
        assert!(budget.try_acquire("a", 1, 1, 1).is_none());
        let b = budget.try_acquire("b", 1, 1, 1).unwrap();
        drop(b);
        assert!(budget.try_acquire("a", 1, 1, 1).is_some());
    }
}
