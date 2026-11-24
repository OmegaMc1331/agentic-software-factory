use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use factory_db::FactoryDb;
use factory_git::Repo;
use factory_runtime::{Runtime, RuntimeBudget};
use serde::Serialize;

use crate::identity::file_identity;
use crate::{
    validate_factory_paths, DirectoryIdentity, ProjectEntry, ProjectError, ProjectId,
    ProjectPathPolicy, ProjectRegistry, Result,
};

#[derive(Clone, Debug, Serialize)]
pub struct ProjectLimits {
    pub registered_projects: usize,
    pub open_projects: usize,
    pub concurrent_operations: usize,
    pub concurrent_tasks: usize,
    pub terminal_sessions: usize,
    pub management_operations: usize,
}

impl Default for ProjectLimits {
    fn default() -> Self {
        Self {
            registered_projects: 128,
            open_projects: 8,
            concurrent_operations: 8,
            concurrent_tasks: 8,
            terminal_sessions: 8,
            management_operations: 2,
        }
    }
}

pub struct ProjectContext {
    pub project_id: ProjectId,
    pub root: PathBuf,
    pub db: Arc<Mutex<FactoryDb>>,
    pub runtime: Runtime,
    pub files: Arc<factory_files::FileService>,
    pub ide: Arc<factory_ide::IdeManager>,
    pub conversations: Arc<factory_conversation::ConversationService>,
    identity: DirectoryIdentity,
    database_identity: String,
}

impl ProjectContext {
    pub fn validate(&self) -> Result<()> {
        self.identity.verify()?;
        validate_factory_paths(&self.root)?;
        if file_identity(&self.root.join(".factory/db.sqlite3"))? != self.database_identity {
            return Err(ProjectError::Unavailable(
                "database identity changed; close and repair this project".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectView {
    #[serde(flatten)]
    pub entry: ProjectEntry,
    pub lifecycle: String,
    pub availability: String,
    pub problem: Option<String>,
}

struct Slot {
    state: Mutex<SlotState>,
    changed: Condvar,
}
enum SlotState {
    Closed,
    Opening,
    Open(Arc<ProjectContext>),
    Closing,
    Unavailable(String),
}

pub struct ProjectManager {
    pub registry: ProjectRegistry,
    pub policy: ProjectPathPolicy,
    pub limits: ProjectLimits,
    pub budget: Arc<RuntimeBudget>,
    ide_budget: Arc<RuntimeBudget>,
    ide_store: Arc<factory_ide::IdeStore>,
    slots: Mutex<BTreeMap<ProjectId, Arc<Slot>>>,
    open_count: AtomicUsize,
}

impl ProjectManager {
    pub fn close_admission(&self) {
        self.budget.begin_shutdown();
        self.ide_budget.close();
    }

    pub fn shutdown(&self, timeout: std::time::Duration) -> Result<()> {
        let initial = self.begin_shutdown();
        let finalization = self.finish_shutdown(timeout);
        initial.and(finalization)
    }

    pub fn begin_shutdown(&self) -> Result<()> {
        self.close_admission();
        let mut failed = false;
        for context in self.open_contexts() {
            failed |= context.conversations.shutdown().is_err();
            failed |= context.runtime.shutdown().is_err();
            failed |= context.ide.shutdown().is_err();
        }
        if failed {
            return Err(ProjectError::Unavailable(
                "Shutdown cancellation reported an error; inspect retained evidence".into(),
            ));
        }
        Ok(())
    }

    pub fn finish_shutdown(&self, timeout: std::time::Duration) -> Result<()> {
        let mut failed = self.begin_shutdown().is_err();
        let contexts = self.open_contexts();
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if contexts.iter().all(|context| {
                context.runtime.is_idle()
                    && context.conversations.is_idle()
                    && context.ide.is_idle()
            }) {
                self.budget.allow_shutdown_checkpoints();
                for context in &contexts {
                    failed |= context.runtime.finish_shutdown_checkpoints().is_err();
                }
                return if failed {
                    Err(ProjectError::Unavailable(
                        "Shutdown cleanup reported an error; inspect retained evidence".into(),
                    ))
                } else {
                    Ok(())
                };
            }
            if std::time::Instant::now() >= deadline {
                return Err(ProjectError::Unavailable("Shutdown cleanup deadline exceeded; interrupted operations require reconciliation".into()));
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }

    pub fn open_contexts(&self) -> Vec<Arc<ProjectContext>> {
        self.slots
            .lock()
            .expect("project slots poisoned")
            .values()
            .filter_map(
                |slot| match &*slot.state.lock().expect("project state poisoned") {
                    SlotState::Open(context) => Some(context.clone()),
                    _ => None,
                },
            )
            .collect()
    }
    pub fn new(
        registry_path: &Path,
        allowed_roots: &[PathBuf],
        limits: ProjectLimits,
    ) -> Result<Self> {
        if registry_path
            .components()
            .any(|part| part.as_os_str() == ".factory")
        {
            return Err(ProjectError::Invalid(
                "application registry must be outside project .factory directories".into(),
            ));
        }
        if limits.open_projects == 0
            || limits.registered_projects == 0
            || limits.management_operations == 0
            || !(1..=32).contains(&limits.concurrent_tasks)
        {
            return Err(ProjectError::Invalid(
                "project limits must be positive".into(),
            ));
        }
        let registry = ProjectRegistry::open(registry_path)?;
        let ide_store = Arc::new(
            factory_ide::IdeStore::open(&registry_path.with_extension("ide.sqlite3"))
                .unwrap_or_else(|_| factory_ide::IdeStore::unavailable()),
        );
        Ok(Self {
            registry,
            ide_store,
            ide_budget: Arc::new(RuntimeBudget::new(16, 1)),
            policy: ProjectPathPolicy::new(allowed_roots)?,
            budget: Arc::new(RuntimeBudget::with_task_limit(
                limits.concurrent_operations,
                limits.terminal_sessions,
                limits.concurrent_tasks,
            )),
            limits,
            slots: Mutex::new(BTreeMap::new()),
            open_count: AtomicUsize::new(0),
        })
    }

    pub fn register(&self, path: &Path, display_name: Option<&str>) -> Result<ProjectEntry> {
        let root = self.policy.authorize(path)?;
        let (git_key, db_key) = inspect_repository(&root, &self.policy)?;
        root.verify()?;
        let name = display_name
            .or_else(|| root.path.file_name().and_then(|s| s.to_str()))
            .unwrap_or("Project");
        self.registry.register(
            &root.path,
            &root.key,
            &git_key,
            db_key.as_deref(),
            name,
            self.limits.registered_projects,
        )
    }

    pub fn list(&self) -> Result<Vec<ProjectView>> {
        self.registry
            .list()?
            .into_iter()
            .map(|entry| self.view(entry))
            .collect()
    }

    pub fn get(&self, id: &ProjectId) -> Result<ProjectView> {
        self.view(self.registry.get(id)?)
    }

    fn view(&self, entry: ProjectEntry) -> Result<ProjectView> {
        let slot = self.slot(&entry.project_id);
        let state = slot
            .state
            .lock()
            .map_err(|_| ProjectError::Unavailable("project state lock failed".into()))?;
        let (lifecycle, previous) = match &*state {
            SlotState::Closed => ("closed", None),
            SlotState::Opening => ("opening", None),
            SlotState::Open(_) => ("open", None),
            SlotState::Closing => ("closing", None),
            SlotState::Unavailable(reason) => ("unavailable", Some(reason.clone())),
        };
        let check = self.validate_entry(&entry);
        let (availability, problem) = match check {
            Err(error) => ("unavailable", Some(error.to_string())),
            Ok(_) if !entry.canonical_root.join(".factory/db.sqlite3").is_file() => {
                ("not_initialized", previous)
            }
            Ok(_) if previous.is_some() => ("unavailable", previous),
            Ok(_) => ("available", previous),
        };
        Ok(ProjectView {
            entry,
            lifecycle: lifecycle.into(),
            availability: availability.into(),
            problem,
        })
    }

    pub fn open(&self, id: &ProjectId) -> Result<Arc<ProjectContext>> {
        self.registry.get(id)?;
        let slot = self.slot(id);
        let mut state = slot
            .state
            .lock()
            .map_err(|_| ProjectError::Unavailable("project state lock failed".into()))?;
        loop {
            match &*state {
                SlotState::Open(context) => {
                    context.validate()?;
                    return Ok(context.clone());
                }
                SlotState::Opening | SlotState::Closing => {
                    let (next, waited) = slot
                        .changed
                        .wait_timeout(state, std::time::Duration::from_secs(30))
                        .map_err(|_| {
                            ProjectError::Unavailable("project state lock failed".into())
                        })?;
                    state = next;
                    if waited.timed_out() {
                        return Err(ProjectError::Conflict(
                            "project lifecycle transition is still in progress".into(),
                        ));
                    }
                }
                _ => break,
            }
        }
        self.open_count
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                (used < self.limits.open_projects).then_some(used + 1)
            })
            .map_err(|_| ProjectError::Limit("open projects"))?;
        *state = SlotState::Opening;
        drop(state);
        let outcome = self
            .registry
            .get(id)
            .and_then(|entry| self.open_context(entry));
        let mut state = slot
            .state
            .lock()
            .map_err(|_| ProjectError::Unavailable("project state lock failed".into()))?;
        match outcome {
            Ok(context) => {
                let context = Arc::new(context);
                *state = SlotState::Open(context.clone());
                slot.changed.notify_all();
                Ok(context)
            }
            Err(error) => {
                self.open_count.fetch_sub(1, Ordering::AcqRel);
                *state = SlotState::Unavailable(error.to_string());
                slot.changed.notify_all();
                Err(error)
            }
        }
    }

    fn open_context(&self, entry: ProjectEntry) -> Result<ProjectContext> {
        let identity = self.validate_entry(&entry)?;
        let (git, db_identity) = inspect_repository(&identity, &self.policy)?;
        if git != entry.git_identity {
            return Err(ProjectError::Unavailable(
                "Git repository identity changed; relink explicitly".into(),
            ));
        }
        if entry.database_identity.is_none() && db_identity.is_some() {
            self.registry.relink(
                &entry.project_id,
                &identity.path,
                &identity.key,
                &git,
                db_identity.as_deref(),
            )?;
        }
        let check_path = identity.path.clone();
        let check_root = identity.key.clone();
        let check_db = db_identity.clone();
        let validator = Arc::new(move || {
            let current = DirectoryIdentity::open(&check_path)
                .map_err(|_| "Project directory is unavailable".to_string())?;
            if current.key != check_root {
                return Err(
                    "Project directory identity changed; explicit relink is required".into(),
                );
            }
            validate_factory_paths(&check_path)
                .map_err(|_| "Protected Factory paths changed".to_string())?;
            let database = file_identity(&check_path.join(".factory/db.sqlite3"))
                .map_err(|_| "Project database is unavailable".to_string())?;
            if check_db.as_deref() != Some(database.as_str()) {
                return Err(
                    "Project database identity changed; explicit repair is required".into(),
                );
            }
            Ok(())
        });
        let runtime =
            Runtime::with_project_validation(&identity.path, self.budget.clone(), Some(validator))?;
        runtime.bind_project_identity(entry.project_id.as_str())?;
        let db = FactoryDb::open(&identity.path.join(".factory/db.sqlite3"))
            .map_err(factory_core::FactoryError::from)?;
        db.conversation_reconcile()
            .map_err(factory_core::FactoryError::from)?;
        identity.verify()?;
        let database_identity = file_identity(&identity.path.join(".factory/db.sqlite3"))?;
        let current = self.registry.get(&entry.project_id)?;
        if current.root_identity != identity.key
            || current.canonical_root != identity.path
            || current.database_identity.as_deref() != Some(database_identity.as_str())
        {
            return Err(ProjectError::Unavailable(
                "registry identity changed during open; retry explicitly".into(),
            ));
        }
        self.registry.opened(&entry.project_id)?;
        let files = Arc::new(
            factory_files::FileService::new(
                entry.project_id.as_str(),
                &identity.path,
                factory_files::Limits::from_environment()
                    .map_err(|_| ProjectError::Unavailable("invalid file-service limits".into()))?,
            )
            .map_err(|_| {
                ProjectError::Unavailable("file service could not verify the project root".into())
            })?,
        );
        Ok(ProjectContext {
            conversations: Arc::new(factory_conversation::ConversationService::new(
                entry.project_id.as_str().into(),
                identity.path.clone(),
                files.clone(),
                runtime.clone(),
            )),
            project_id: entry.project_id,
            root: identity.path.clone(),
            db: Arc::new(Mutex::new(db)),
            runtime,
            ide: Arc::new(factory_ide::IdeManager::new(
                files.clone(),
                self.ide_store.clone(),
                self.ide_budget.clone(),
                self.budget.clone(),
            )),
            files,
            identity,
            database_identity,
        })
    }

    pub fn context(&self, id: &ProjectId) -> Result<Arc<ProjectContext>> {
        let entry = self.registry.get(id)?;
        let slot = self.slot(id);
        let state = slot
            .state
            .lock()
            .map_err(|_| ProjectError::Unavailable("project state lock failed".into()))?;
        if let SlotState::Unavailable(reason) = &*state {
            return Err(ProjectError::Unavailable(reason.clone()));
        }
        let SlotState::Open(context) = &*state else {
            return Err(ProjectError::Closed);
        };
        if entry.root_identity != context.identity.key
            || entry.canonical_root != context.root
            || entry.database_identity.as_deref() != Some(context.database_identity.as_str())
        {
            return Err(ProjectError::Unavailable(
                "registry ownership changed; close and reopen the project explicitly".into(),
            ));
        }
        self.validate_entry(&entry)?;
        context.validate()?;
        Ok(context.clone())
    }

    pub fn close(&self, id: &ProjectId) -> Result<()> {
        self.registry.get(id)?;
        let slot = self.slot(id);
        let mut state = slot
            .state
            .lock()
            .map_err(|_| ProjectError::Unavailable("project state lock failed".into()))?;
        match &*state {
            SlotState::Opening | SlotState::Closing => {
                return Err(ProjectError::Conflict(
                    "project lifecycle transition in progress".into(),
                ))
            }
            SlotState::Open(context)
                if Arc::strong_count(context) > 1
                    || Arc::strong_count(&context.conversations) > 1
                    || !context.runtime.is_idle()
                    || !context.ide.is_idle()
                    || !context.conversations.is_idle() =>
            {
                return Err(ProjectError::Conflict(
                    "project owns active work or live requests; close those first".into(),
                ))
            }
            SlotState::Open(_) => {}
            _ => {
                *state = SlotState::Closed;
                return Ok(());
            }
        }
        let old = std::mem::replace(&mut *state, SlotState::Closing);
        drop(state);
        drop(old);
        self.open_count.fetch_sub(1, Ordering::AcqRel);
        *slot
            .state
            .lock()
            .map_err(|_| ProjectError::Unavailable("project state lock failed".into()))? =
            SlotState::Closed;
        slot.changed.notify_all();
        Ok(())
    }

    pub fn rename(&self, id: &ProjectId, name: &str) -> Result<ProjectView> {
        self.registry.rename(id, name)?;
        self.get(id)
    }

    pub fn remove(&self, id: &ProjectId) -> Result<()> {
        self.closed_mutation(id, || self.registry.remove(id))?;
        self.slots
            .lock()
            .expect("project slots poisoned")
            .remove(id);
        Ok(())
    }

    pub fn relink(
        &self,
        id: &ProjectId,
        path: &Path,
        accept_identity_change: bool,
    ) -> Result<ProjectView> {
        self.closed_mutation(id, || {
            let previous = self.registry.get(id)?;
            let identity = self.policy.authorize(path)?;
            if previous.root_identity != identity.key && !accept_identity_change {
                return Err(ProjectError::Conflict(
                    "new filesystem identity requires explicit acknowledgement".into(),
                ));
            }
            let (git, db) = inspect_repository(&identity, &self.policy)?;
            identity.verify()?;
            self.registry
                .relink(id, &identity.path, &identity.key, &git, db.as_deref())
        })?;
        self.get(id)
    }

    pub fn initialize(&self, id: &ProjectId) -> Result<ProjectView> {
        self.closed_mutation(id, || {
            let entry = self.registry.get(id)?;
            let identity = self.validate_entry(&entry)?;
            initialize_factory(&identity.path)?;
            let (git, db) = inspect_repository(&identity, &self.policy)?;
            self.registry
                .relink(id, &identity.path, &identity.key, &git, db.as_deref())
        })?;
        self.get(id)
    }

    pub fn create(&self, destination: &Path, name: &str) -> Result<ProjectEntry> {
        crate::registry::validate_name(name)?;
        let path = self.new_destination(destination)?;
        std::fs::create_dir(&path)?;
        Repo::initialize(&path).map_err(|_| {
            ProjectError::Unavailable(
                "Git initialization failed; new directory retained for recovery".into(),
            )
        })?;
        initialize_factory(&path)?;
        self.register(&path, Some(name))
    }

    pub fn clone_project(
        &self,
        remote: &str,
        destination: &Path,
        name: &str,
        initialize: bool,
        allow_network: bool,
    ) -> Result<ProjectEntry> {
        crate::registry::validate_name(name)?;
        let remote = self.validate_remote(remote, allow_network)?;
        let path = self.new_destination(destination)?;
        std::fs::create_dir(&path)?;
        Repo::clone_into(&remote, &path).map_err(|_| {
            ProjectError::Unavailable(
                "clone failed or timed out; destination retained, nothing registered".into(),
            )
        })?;
        self.policy.authorize(&path)?;
        validate_factory_paths(&path)?;
        if initialize {
            if !path.join(".factory").exists() {
                initialize_factory(&path)?;
            } else if !path.join(".factory/db.sqlite3").is_file()
                || !path.join(".factory/config.toml").is_file()
            {
                return Err(ProjectError::Conflict("cloned .factory data is incomplete; directory retained, register it explicitly for recovery".into()));
            }
        }
        self.register(&path, Some(name))
    }

    fn new_destination(&self, destination: &Path) -> Result<PathBuf> {
        let path = self.policy.destination(destination)?;
        if self.registry.list()?.iter().any(|entry| {
            path.starts_with(&entry.canonical_root) || entry.canonical_root.starts_with(&path)
        }) {
            return Err(ProjectError::Conflict(
                "new project destination overlaps a registered project".into(),
            ));
        }
        Ok(path)
    }

    fn validate_remote(&self, value: &str, network: bool) -> Result<String> {
        if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
            return Err(ProjectError::Invalid("invalid remote".into()));
        }
        let path = Path::new(value);
        if path.is_absolute() {
            return Ok(self
                .policy
                .authorize(path)?
                .path
                .to_string_lossy()
                .into_owned());
        }
        let url = url::Url::parse(value).map_err(|_| {
            ProjectError::Invalid(
                "use an authorized absolute local path, https:// or ssh:// remote".into(),
            )
        })?;
        if !network
            || !matches!(url.scheme(), "https" | "ssh")
            || url.host_str().is_none()
            || url.password().is_some()
            || (url.scheme() == "https" && !url.username().is_empty())
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ProjectError::Invalid("network cloning needs explicit authorization and a credential-free HTTPS or SSH URL".into()));
        }
        Ok(url.to_string())
    }

    fn validate_entry(&self, entry: &ProjectEntry) -> Result<DirectoryIdentity> {
        let identity = self.policy.authorize(&entry.canonical_root)?;
        if identity.key != entry.root_identity {
            return Err(ProjectError::Unavailable(
                "repository path now identifies another directory; relink explicitly".into(),
            ));
        }
        validate_factory_paths(&identity.path)?;
        if let Some(expected) = &entry.database_identity {
            if &file_identity(&identity.path.join(".factory/db.sqlite3"))? != expected {
                return Err(ProjectError::Unavailable(
                    "database identity changed; relink explicitly".into(),
                ));
            }
        }
        Ok(identity)
    }

    fn closed_mutation<T>(&self, id: &ProjectId, action: impl FnOnce() -> Result<T>) -> Result<T> {
        self.registry.get(id)?;
        let slot = self.slot(id);
        let mut state = slot
            .state
            .lock()
            .map_err(|_| ProjectError::Unavailable("project state lock failed".into()))?;
        if !matches!(&*state, SlotState::Closed | SlotState::Unavailable(_)) {
            return Err(ProjectError::Conflict(
                "close the project before this operation".into(),
            ));
        }
        *state = SlotState::Closing;
        drop(state);
        let result = (|| {
            let entry = self.registry.get(id)?;
            let _ownership = if entry.canonical_root.join(".factory").is_dir() {
                validate_factory_paths(&entry.canonical_root)?;
                Some(factory_runtime::ProjectOwnership::acquire(
                    &entry.canonical_root,
                )?)
            } else {
                None
            };
            action()
        })();
        let mut state = slot
            .state
            .lock()
            .map_err(|_| ProjectError::Unavailable("project state lock failed".into()))?;
        *state = match &result {
            Ok(_) => SlotState::Closed,
            Err(error) => SlotState::Unavailable(error.to_string()),
        };
        slot.changed.notify_all();
        result
    }

    fn slot(&self, id: &ProjectId) -> Arc<Slot> {
        self.slots
            .lock()
            .expect("project slots poisoned")
            .entry(id.clone())
            .or_insert_with(|| {
                Arc::new(Slot {
                    state: Mutex::new(SlotState::Closed),
                    changed: Condvar::new(),
                })
            })
            .clone()
    }
}

fn inspect_repository(
    root: &DirectoryIdentity,
    policy: &ProjectPathPolicy,
) -> Result<(String, Option<String>)> {
    let repo = match Repo::detect(&root.path) {
        Ok(repo) => repo,
        Err(_)
            if root.path.join(".factory/db.sqlite3").is_file()
                && !root.path.join(".git").exists() =>
        {
            validate_factory_paths(&root.path)?;
            return Ok((
                format!("legacy:{}", root.key),
                Some(file_identity(&root.path.join(".factory/db.sqlite3"))?),
            ));
        }
        Err(_) => {
            return Err(ProjectError::Invalid(
                "path must be a Git repository root or an initialized V1 project".into(),
            ))
        }
    };
    let actual = DirectoryIdentity::open(repo.root())?;
    if actual.key != root.key {
        return Err(ProjectError::Invalid(
            "select the repository root, not a subdirectory".into(),
        ));
    }
    validate_factory_paths(&root.path)?;
    let common = repo
        .common_directory()
        .map_err(|_| ProjectError::Invalid("Git common directory unavailable".into()))?;
    let git_key = policy.authorize(&common)?.key;
    let db_path = root.path.join(".factory/db.sqlite3");
    let db_key = if db_path.exists() {
        Some(file_identity(&db_path)?)
    } else {
        None
    };
    Ok((git_key, db_key))
}

fn initialize_factory(root: &Path) -> Result<()> {
    validate_factory_paths(root)?;
    std::fs::create_dir(root.join(".factory")).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            ProjectError::Conflict(
                "existing .factory directory is never overwritten; inspect or repair it explicitly"
                    .into(),
            )
        } else {
            error.into()
        }
    })?;
    factory_core::Factory::init(root)?;
    Ok(())
}
