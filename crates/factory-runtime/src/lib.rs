use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use factory_core::{ExecutionRoles, Factory, FactoryError};
use factory_types::Run;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Planning,
    Executing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveOperation {
    pub run_id: i64,
    pub kind: OperationKind,
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("workflow #{0} already has an active operation")]
    AlreadyActive(i64),
    #[error("workflow #{0} is not active in this Factory process")]
    NotActive(i64),
    #[error(transparent)]
    Factory(#[from] FactoryError),
}

#[derive(Clone)]
pub struct Runtime {
    root: Arc<PathBuf>,
    active: Arc<Mutex<HashMap<i64, Active>>>,
}

struct Active {
    kind: OperationKind,
    cancel: Arc<AtomicBool>,
}

impl Runtime {
    pub fn new(root: &Path) -> Result<Self, RuntimeError> {
        Factory::open(root)?.reconcile_interrupted()?;
        Ok(Self {
            root: Arc::new(root.to_path_buf()),
            active: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn create_workflow(&self, objective: &str) -> Result<Run, RuntimeError> {
        let factory = Factory::open(&self.root)?;
        let run = factory.begin_run(objective)?;
        let cancel = self.reserve(run.id, OperationKind::Planning)?;
        let runtime = self.clone();
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            if let Ok(factory) = Factory::open(&root) {
                let _ = factory.plan_run(run.id, &cancel);
            }
            runtime.release(run.id);
        });
        Ok(run)
    }

    pub fn start_workflow(&self, run_id: i64) -> Result<ExecutionRoles, RuntimeError> {
        let factory = Factory::open(&self.root)?;
        let cancel = self.reserve(run_id, OperationKind::Executing)?;
        let roles = match factory.prepare_start(run_id) {
            Ok(roles) => roles,
            Err(error) => {
                self.release(run_id);
                return Err(error.into());
            }
        };
        self.spawn_execution(run_id, cancel);
        Ok(roles)
    }

    pub fn retry_task(&self, task_id: i64) -> Result<i64, RuntimeError> {
        let factory = Factory::open(&self.root)?;
        let task = factory
            .get_task(task_id)?
            .ok_or(FactoryError::TaskNotFound(task_id))?;
        let cancel = self.reserve(task.run_id, OperationKind::Executing)?;
        let run_id = match factory.prepare_retry(task_id) {
            Ok(run_id) => run_id,
            Err(error) => {
                self.release(task.run_id);
                return Err(error.into());
            }
        };
        self.spawn_execution(run_id, cancel);
        Ok(run_id)
    }

    pub fn cancel_workflow(&self, run_id: i64) -> Result<(), RuntimeError> {
        let cancel = {
            let active = self.active.lock().expect("runtime mutex poisoned");
            active
                .get(&run_id)
                .map(|operation| operation.cancel.clone())
                .ok_or(RuntimeError::NotActive(run_id))?
        };
        cancel.store(true, Ordering::Relaxed);
        Factory::open(&self.root)?.cancel_run(run_id)?;
        Ok(())
    }

    pub fn active_operations(&self) -> Vec<ActiveOperation> {
        let mut operations: Vec<_> = self
            .active
            .lock()
            .expect("runtime mutex poisoned")
            .iter()
            .map(|(run_id, active)| ActiveOperation {
                run_id: *run_id,
                kind: active.kind,
            })
            .collect();
        operations.sort_by_key(|operation| operation.run_id);
        operations
    }

    fn spawn_execution(&self, run_id: i64, cancel: Arc<AtomicBool>) {
        let runtime = self.clone();
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            if let Ok(factory) = Factory::open(&root) {
                let _ = factory.execute_active_run(run_id, &cancel);
            }
            runtime.release(run_id);
        });
    }

    fn reserve(&self, run_id: i64, kind: OperationKind) -> Result<Arc<AtomicBool>, RuntimeError> {
        let mut active = self.active.lock().expect("runtime mutex poisoned");
        if active.contains_key(&run_id) {
            return Err(RuntimeError::AlreadyActive(run_id));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        active.insert(
            run_id,
            Active {
                kind,
                cancel: cancel.clone(),
            },
        );
        Ok(cancel)
    }

    fn release(&self, run_id: i64) {
        self.active
            .lock()
            .expect("runtime mutex poisoned")
            .remove(&run_id);
    }
}
