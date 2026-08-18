//! Pure plan-resolution logic shared by Factory Core and Factory Db.
//!
//! Validation and state resolution are pure functions over a [`PlanState`]
//! snapshot so the same invariants are enforced (a) by the editor's validate
//! pass and (b) inside the single SQLite transaction that applies a mutation.
//! Structural rules live here (graph integrity, immutability, state
//! recomputation); role/operation catalog checks that need the runtime config
//! live in Factory Core on top of this module.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::artifact::TaskOperation;
use crate::plans::{PlanDiagnostic, PlanMutation, PlanPatch, ReplanRequest, TaskRef};
use crate::task::{Task, TaskState};

/// Maximum number of tasks a plan may contain (mirrors the planner's guard).
pub const MAX_PLAN_TASKS: usize = 50;

/// Snapshot of a run's plan used for validation and resolution.
#[derive(Debug, Clone)]
pub struct PlanState {
    pub revision: i64,
    pub objective: String,
    pub tasks: Vec<Task>,
    /// Number of task attempts per task id (the immutability boundary).
    pub attempts: HashMap<i64, usize>,
}

/// A task in the fully resolved target state of the plan. New tasks carry
/// negative placeholder ids; the database maps them to real ids during apply.
#[derive(Debug, Clone)]
pub struct ResolvedTask {
    pub id: i64,
    pub insert: bool,
    pub title: String,
    pub objective: String,
    pub acceptance_criteria: Vec<String>,
    pub state: TaskState,
    pub position: i32,
    pub dependencies: Vec<i64>,
    pub role: Option<String>,
    pub operation: Option<TaskOperation>,
}

/// The concrete, fully resolved target state of the plan after an edit or a
/// replan.
#[derive(Debug, Clone)]
pub struct ResolvedPlan {
    pub revision: i64,
    pub objective: String,
    pub tasks: Vec<ResolvedTask>,
    /// Existing task ids removed by the edit.
    pub removed: Vec<i64>,
    /// Existing task ids that must be set to `superseded`.
    pub superseded: Vec<i64>,
}

/// Outcome of applying a plan change in the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanApplyOutcome {
    Applied {
        run_id: i64,
        revision: i64,
        tasks: Vec<Task>,
    },
    Invalid(Vec<PlanDiagnostic>),
}

/// A task is immutable once it has consumed run resources (any attempt) or is
/// in a state that cannot be rolled back.
pub fn is_immutable(task: &Task, attempts: usize) -> bool {
    attempts > 0
        || matches!(
            task.state,
            TaskState::Running
                | TaskState::AwaitingIntegration
                | TaskState::Integrating
                | TaskState::Completed
                | TaskState::Superseded
        )
}

fn is_working_immutable(task: &ResolvedTask, attempts: &HashMap<i64, usize>) -> bool {
    let count = attempts.get(&task.id).copied().unwrap_or(0);
    count > 0
        || matches!(
            task.state,
            TaskState::Running
                | TaskState::AwaitingIntegration
                | TaskState::Integrating
                | TaskState::Completed
                | TaskState::Superseded
        )
}

fn diag(task: Option<TaskRef>, field: Option<&str>, code: &str, message: String) -> PlanDiagnostic {
    PlanDiagnostic {
        task,
        field: field.map(str::to_string),
        error_code: code.to_string(),
        message,
    }
}

fn resolved_from_task(task: &Task) -> ResolvedTask {
    ResolvedTask {
        id: task.id,
        insert: false,
        title: task.title.clone(),
        objective: task.objective.clone(),
        acceptance_criteria: task.acceptance_criteria.clone(),
        state: task.state,
        position: task.position,
        dependencies: task.dependencies.clone(),
        role: task.role.clone(),
        operation: task.operation,
    }
}

/// Resolves a `TaskRef` to a task id: real ids must exist in the working set,
/// client ids must map to a placeholder registered by an `AddTask` in the same
/// patch.
fn resolve_id(
    tref: &TaskRef,
    tasks: &[ResolvedTask],
    clients: &HashMap<String, i64>,
) -> Option<i64> {
    match tref {
        TaskRef::Id(id) => tasks.iter().any(|t| t.id == *id).then_some(*id),
        TaskRef::ClientId(client_id) => clients.get(client_id).copied(),
    }
}

/// Recomputes the state of every mutable task (attempt-free tasks in
/// pending/ready/blocked, plus drafts) from its dependency states, in
/// topological order. Mutable-failed tasks are left untouched.
fn recompute_states(tasks: &mut [ResolvedTask], attempts: &HashMap<i64, usize>) {
    let state_of: HashMap<i64, TaskState> = tasks.iter().map(|t| (t.id, t.state)).collect();
    let mutable: Vec<i64> = tasks
        .iter()
        .filter(|t| {
            t.insert
                || (attempts.get(&t.id).copied().unwrap_or(0) == 0
                    && matches!(
                        t.state,
                        TaskState::Pending | TaskState::Ready | TaskState::Blocked
                    ))
        })
        .map(|t| t.id)
        .collect();
    let mutable_set: HashSet<i64> = mutable.iter().copied().collect();
    let mut indegree: HashMap<i64, usize> = mutable
        .iter()
        .map(|id| {
            let task = tasks.iter().find(|t| t.id == *id).expect("mutable known");
            (
                *id,
                task.dependencies
                    .iter()
                    .filter(|d| mutable_set.contains(d))
                    .count(),
            )
        })
        .collect();
    let mut queue: VecDeque<i64> = mutable
        .iter()
        .copied()
        .filter(|id| *indegree.get(id).expect("known") == 0)
        .collect();
    let mut order = Vec::with_capacity(mutable.len());
    while let Some(id) = queue.pop_front() {
        order.push(id);
        for other in &mutable {
            if *indegree.get(other).expect("known") == 0 {
                continue;
            }
            let other_task = tasks
                .iter()
                .find(|t| t.id == *other)
                .expect("mutable known");
            if other_task.dependencies.contains(&id) {
                let degree = indegree.get_mut(other).expect("known");
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(*other);
                }
            }
        }
    }
    if order.len() != mutable.len() {
        // A cycle among mutable tasks; a diagnostic is reported separately and
        // nothing is applied, so the states are left as-is.
        return;
    }
    let mut state_of = state_of;
    for id in order {
        let deps = tasks
            .iter()
            .find(|t| t.id == id)
            .expect("known")
            .dependencies
            .clone();
        let next = if deps.is_empty() {
            TaskState::Ready
        } else {
            let mut blocked = false;
            let mut all_completed = true;
            for dep in &deps {
                let dep_state = state_of.get(dep).copied().unwrap_or(TaskState::Pending);
                match dep_state {
                    TaskState::Failed | TaskState::Blocked | TaskState::Superseded => {
                        blocked = true;
                        break;
                    }
                    _ => {}
                }
                if dep_state != TaskState::Completed {
                    all_completed = false;
                }
            }
            if blocked {
                TaskState::Blocked
            } else if all_completed {
                TaskState::Ready
            } else {
                TaskState::Pending
            }
        };
        if let Some(task) = tasks.iter_mut().find(|t| t.id == id) {
            task.state = next;
        }
        state_of.insert(id, next);
    }
}

/// Detects a cycle in the dependency graph and reports it as a plan-level
/// diagnostic. Returns true when a cycle exists.
fn detect_cycle(tasks: &[ResolvedTask], diags: &mut Vec<PlanDiagnostic>) -> bool {
    let by_id: HashMap<i64, &ResolvedTask> = tasks.iter().map(|t| (t.id, t)).collect();
    // 0 unvisited, 1 in current DFS stack, 2 fully processed.
    let mut state_map: HashMap<i64, u8> = HashMap::new();
    let mut cyclic = false;
    for start in tasks.iter() {
        if *state_map.get(&start.id).unwrap_or(&0) != 0 {
            continue;
        }
        let mut stack = vec![(start.id, false)];
        while let Some((id, exiting)) = stack.pop() {
            if exiting {
                state_map.insert(id, 2);
                continue;
            }
            let mark = state_map.entry(id).or_insert(0);
            if *mark == 1 {
                cyclic = true;
                continue;
            }
            *mark = 1;
            stack.push((id, true));
            if let Some(task) = by_id.get(&id) {
                for dep in &task.dependencies {
                    if state_map.get(dep).copied().unwrap_or(0) == 2 {
                        continue;
                    }
                    stack.push((*dep, false));
                }
            }
        }
    }
    if cyclic {
        diags.push(diag(
            None,
            Some("dependencies"),
            "dependency_cycle",
            "The task dependency graph must stay acyclic.".to_string(),
        ));
    }
    cyclic
}

/// Content checks that mirror the planner's `validate_plan`: non-empty title,
/// objective, and acceptance criteria.
fn check_content(
    tref: Option<TaskRef>,
    title: &str,
    objective: &str,
    acceptance_criteria: &[String],
    diags: &mut Vec<PlanDiagnostic>,
) {
    if title.trim().is_empty() {
        diags.push(diag(
            tref.clone(),
            Some("title"),
            "empty_title",
            "Every task needs a title.".to_string(),
        ));
    }
    if objective.trim().is_empty() {
        diags.push(diag(
            tref.clone(),
            Some("objective"),
            "empty_objective",
            "Every task needs an objective.".to_string(),
        ));
    }
    if acceptance_criteria.is_empty() {
        diags.push(diag(
            tref.clone(),
            Some("acceptanceCriteria"),
            "empty_acceptance_criteria",
            "Every task needs at least one acceptance criterion.".to_string(),
        ));
    } else if acceptance_criteria.iter().any(|c| c.trim().is_empty()) {
        diags.push(diag(
            tref,
            Some("acceptanceCriteria"),
            "empty_acceptance_criterion",
            "Acceptance criteria must not be empty strings.".to_string(),
        ));
    }
}

fn check_planning_role(
    tref: &TaskRef,
    role: &Option<String>,
    operation: Option<TaskOperation>,
    diags: &mut Vec<PlanDiagnostic>,
) {
    if role.as_deref() == Some("planner") {
        diags.push(diag(
            Some(tref.clone()),
            Some("role"),
            "planner_role",
            "The planner role cannot appear as a planned task.".to_string(),
        ));
    }
    if operation == Some(TaskOperation::Planning) {
        diags.push(diag(
            Some(tref.clone()),
            Some("operation"),
            "planning_operation",
            "Planned tasks cannot use the planning operation.".to_string(),
        ));
    }
}

/// Resolves a patch of mutations against the current plan. Returns the fully
/// resolved target plan on success, or every collected diagnostic on failure.
pub fn resolve_patch(
    state: &PlanState,
    patch: &PlanPatch,
) -> std::result::Result<ResolvedPlan, Vec<PlanDiagnostic>> {
    let mut diags: Vec<PlanDiagnostic> = Vec::new();
    let mut tasks: Vec<ResolvedTask> = state.tasks.iter().map(resolved_from_task).collect();
    let mut clients: HashMap<String, i64> = HashMap::new();
    // Pre-assign placeholders so references may legally precede their AddTask.
    for mutation in &patch.mutations {
        if let PlanMutation::AddTask { client_id, .. } = mutation {
            let placeholder = -(clients.len() as i64) - 1;
            clients.insert(client_id.clone(), placeholder);
        }
    }

    let state_of_existing: HashMap<i64, TaskState> =
        state.tasks.iter().map(|t| (t.id, t.state)).collect();
    let attempts = &state.attempts;

    // Process AddTask operations first so every client id referenced by later
    // operations is already a task in the working set.
    for mutation in &patch.mutations {
        let PlanMutation::AddTask {
            client_id,
            title,
            objective,
            acceptance_criteria,
            dependencies,
            role,
            operation,
        } = mutation
        else {
            continue;
        };
        let placeholder = *clients.get(client_id).expect("pre-registered");
        let tref = TaskRef::ClientId(client_id.clone());
        check_content(
            Some(tref.clone()),
            title,
            objective,
            acceptance_criteria,
            &mut diags,
        );
        check_planning_role(&tref, role, *operation, &mut diags);
        let mut deps = Vec::new();
        for dep in dependencies {
            match resolve_id(dep, &tasks, &clients) {
                None => diags.push(diag(
                    Some(tref.clone()),
                    Some("dependencies"),
                    "unknown_dependency",
                    format!("Dependency {} does not exist.", dep.label()),
                )),
                Some(dep_id) if dep_id == placeholder => diags.push(diag(
                    Some(tref.clone()),
                    Some("dependencies"),
                    "self_dependency",
                    "A task cannot depend on itself.".to_string(),
                )),
                Some(dep_id)
                    if state_of_existing.get(&dep_id).copied() == Some(TaskState::Superseded) =>
                {
                    diags.push(diag(
                        Some(tref.clone()),
                        Some("dependencies"),
                        "superseded_dependency",
                        "A task cannot depend on a superseded task.".to_string(),
                    ));
                }
                Some(dep_id) if deps.contains(&dep_id) => diags.push(diag(
                    Some(tref.clone()),
                    Some("dependencies"),
                    "duplicate_dependency",
                    "Each dependency can only be listed once.".to_string(),
                )),
                Some(dep_id) => deps.push(dep_id),
            }
        }
        tasks.push(ResolvedTask {
            id: placeholder,
            insert: true,
            title: title.clone(),
            objective: objective.clone(),
            acceptance_criteria: acceptance_criteria.clone(),
            state: TaskState::Pending,
            position: i32::MAX,
            dependencies: deps,
            role: role.clone(),
            operation: *operation,
        });
    }

    let mut removed: Vec<i64> = Vec::new();
    let mut reorders: Vec<(i64, u32)> = Vec::new();

    for mutation in &patch.mutations {
        match mutation {
            PlanMutation::AddTask { .. } => continue,
            PlanMutation::UpdateTask {
                task,
                title,
                objective,
                acceptance_criteria,
                role,
                operation,
            } => {
                let id = match resolve_id(task, &tasks, &clients) {
                    Some(id) => id,
                    None => {
                        diags.push(diag(
                            Some(task.clone()),
                            Some("task"),
                            "unknown_task",
                            "The referenced task does not exist.".to_string(),
                        ));
                        continue;
                    }
                };
                let index = tasks
                    .iter()
                    .position(|t| t.id == id)
                    .expect("resolved id known");
                if is_working_immutable(&tasks[index], attempts) {
                    diags.push(diag(
                        Some(task.clone()),
                        Some("task"),
                        "immutable_task",
                        "A task that has started or finished can no longer be edited.".to_string(),
                    ));
                    continue;
                }
                let mut updated = tasks.remove(index);
                if let Some(value) = title {
                    if value.trim().is_empty() {
                        diags.push(diag(
                            Some(task.clone()),
                            Some("title"),
                            "empty_title",
                            "Every task needs a title.".to_string(),
                        ));
                    } else {
                        updated.title = value.clone();
                    }
                }
                if let Some(value) = objective {
                    if value.trim().is_empty() {
                        diags.push(diag(
                            Some(task.clone()),
                            Some("objective"),
                            "empty_objective",
                            "Every task needs an objective.".to_string(),
                        ));
                    } else {
                        updated.objective = value.clone();
                    }
                }
                if let Some(criteria) = acceptance_criteria {
                    match criteria {
                        Some(list) => {
                            check_content(
                                Some(task.clone()),
                                &updated.title,
                                &updated.objective,
                                list,
                                &mut diags,
                            );
                            updated.acceptance_criteria = list.clone();
                        }
                        None => updated.acceptance_criteria = Vec::new(),
                    }
                }
                if let Some(value) = role {
                    if value.as_deref() == Some("planner") {
                        diags.push(diag(
                            Some(task.clone()),
                            Some("role"),
                            "planner_role",
                            "The planner role cannot appear as a planned task.".to_string(),
                        ));
                    }
                    updated.role = value.clone();
                }
                if let Some(value) = operation {
                    if *value == Some(TaskOperation::Planning) {
                        diags.push(diag(
                            Some(task.clone()),
                            Some("operation"),
                            "planning_operation",
                            "Planned tasks cannot use the planning operation.".to_string(),
                        ));
                    }
                    updated.operation = *value;
                }
                tasks.insert(index, updated);
            }
            PlanMutation::RemoveTask { task } => {
                let id = match resolve_id(task, &tasks, &clients) {
                    Some(id) => id,
                    None => {
                        diags.push(diag(
                            Some(task.clone()),
                            Some("task"),
                            "unknown_task",
                            "The referenced task does not exist.".to_string(),
                        ));
                        continue;
                    }
                };
                let index = tasks
                    .iter()
                    .position(|t| t.id == id)
                    .expect("resolved id known");
                if is_working_immutable(&tasks[index], attempts) {
                    diags.push(diag(
                        Some(task.clone()),
                        Some("task"),
                        "immutable_task",
                        "A task that has started or finished cannot be removed.".to_string(),
                    ));
                    continue;
                }
                let removed_existing = !tasks[index].insert;
                // A removed task cannot leave an immutable dependent dangling.
                for other in tasks.iter() {
                    if other.id == id || !other.dependencies.contains(&id) {
                        continue;
                    }
                    if is_working_immutable(other, attempts) {
                        diags.push(diag(
                            Some(task.clone()),
                            Some("task"),
                            "immutable_dependent",
                            "The task depends on completed work and cannot be removed.".to_string(),
                        ));
                        break;
                    }
                }
                tasks.retain(|t| t.id != id);
                if removed_existing {
                    removed.push(id);
                }
            }
            PlanMutation::AddDependency { task, depends_on } => {
                handle_dependency(
                    &mut tasks,
                    &clients,
                    task,
                    depends_on,
                    true,
                    &state_of_existing,
                    attempts,
                    &mut diags,
                );
            }
            PlanMutation::RemoveDependency { task, depends_on } => {
                handle_dependency(
                    &mut tasks,
                    &clients,
                    task,
                    depends_on,
                    false,
                    &state_of_existing,
                    attempts,
                    &mut diags,
                );
            }
            PlanMutation::ReorderTask { task, position } => {
                let id = match resolve_id(task, &tasks, &clients) {
                    Some(id) => id,
                    None => {
                        diags.push(diag(
                            Some(task.clone()),
                            Some("task"),
                            "unknown_task",
                            "The referenced task does not exist.".to_string(),
                        ));
                        continue;
                    }
                };
                let index = tasks
                    .iter()
                    .position(|t| t.id == id)
                    .expect("resolved id known");
                if is_working_immutable(&tasks[index], attempts) {
                    diags.push(diag(
                        Some(task.clone()),
                        Some("position"),
                        "immutable_task",
                        "A task that has started or finished can no longer be reordered."
                            .to_string(),
                    ));
                    continue;
                }
                reorders.push((id, *position));
            }
        }
    }

    // Prune dangling dependencies (pointing at removed tasks or drafts).
    let live: HashSet<i64> = tasks.iter().map(|t| t.id).collect();
    for task in &mut tasks {
        task.dependencies.retain(|dep| live.contains(dep));
    }

    // Reorder after all additions/removals for deterministic targeting.
    for (id, position) in reorders {
        let index = tasks.iter().position(|t| t.id == id);
        let Some(index) = index else { continue };
        let task = tasks.remove(index);
        let insert_at = (position as usize).min(tasks.len());
        tasks.insert(insert_at, task);
    }

    for (index, task) in tasks.iter_mut().enumerate() {
        task.position = index as i32;
    }

    detect_cycle(&tasks, &mut diags);
    recompute_states(&mut tasks, attempts);

    if !diags.is_empty() {
        return Err(diags);
    }

    Ok(ResolvedPlan {
        revision: state.revision + 1,
        objective: state.objective.clone(),
        tasks,
        removed,
        superseded: Vec::new(),
    })
}

#[allow(clippy::too_many_arguments)]
fn handle_dependency(
    tasks: &mut [ResolvedTask],
    clients: &HashMap<String, i64>,
    task: &TaskRef,
    depends_on: &TaskRef,
    add: bool,
    state_of_existing: &HashMap<i64, TaskState>,
    attempts: &HashMap<i64, usize>,
    diags: &mut Vec<PlanDiagnostic>,
) {
    let task_id = match resolve_id(task, tasks, clients) {
        Some(id) => id,
        None => {
            diags.push(diag(
                Some(task.clone()),
                Some("task"),
                "unknown_task",
                "The referenced task does not exist.".to_string(),
            ));
            return;
        }
    };
    let dep_id = match resolve_id(depends_on, tasks, clients) {
        Some(id) => id,
        None => {
            diags.push(diag(
                Some(task.clone()),
                Some("dependencies"),
                "unknown_dependency",
                format!("Dependency {} does not exist.", depends_on.label()),
            ));
            return;
        }
    };
    if task_id == dep_id {
        diags.push(diag(
            Some(task.clone()),
            Some("dependencies"),
            "self_dependency",
            "A task cannot depend on itself.".to_string(),
        ));
        return;
    }
    let index = tasks
        .iter()
        .position(|t| t.id == task_id)
        .expect("resolved id known");
    if is_working_immutable(&tasks[index], attempts) {
        diags.push(diag(
            Some(task.clone()),
            Some("dependencies"),
            "immutable_task",
            "A task that has started or finished can no longer be edited.".to_string(),
        ));
        return;
    }
    if state_of_existing.get(&dep_id).copied() == Some(TaskState::Superseded) {
        diags.push(diag(
            Some(task.clone()),
            Some("dependencies"),
            "superseded_dependency",
            "A task cannot depend on a superseded task.".to_string(),
        ));
        return;
    }
    let modified = &mut tasks[index];
    if add {
        if modified.dependencies.contains(&dep_id) {
            diags.push(diag(
                Some(task.clone()),
                Some("dependencies"),
                "duplicate_dependency",
                "Each dependency can only be listed once.".to_string(),
            ));
        } else {
            modified.dependencies.push(dep_id);
        }
    } else if !modified.dependencies.contains(&dep_id) {
        diags.push(diag(
            Some(task.clone()),
            Some("dependencies"),
            "missing_dependency",
            format!("Task does not depend on {}.", depends_on.label()),
        ));
    } else {
        modified.dependencies.retain(|d| *d != dep_id);
    }
}

/// Computes the mutable replan scope seeded by `seed`: every transitive
/// mutable descendant of the seed. Descendants beyond an immutable task are
/// not included. The seed itself stays as the upstream anchor.
pub fn mutable_scope(state: &PlanState, seed: i64) -> Vec<i64> {
    let mut dependents: HashMap<i64, Vec<i64>> = HashMap::new();
    for task in &state.tasks {
        for dep in &task.dependencies {
            dependents.entry(*dep).or_default().push(task.id);
        }
    }
    let mut scope = HashSet::new();
    let mut queue = VecDeque::from([seed]);
    while let Some(current) = queue.pop_front() {
        let Some(children) = dependents.get(&current) else {
            continue;
        };
        for child in children {
            let child_task = state
                .tasks
                .iter()
                .find(|t| t.id == *child)
                .expect("dependent exists");
            let count = state.attempts.get(child).copied().unwrap_or(0);
            if is_immutable(child_task, count) {
                continue;
            }
            if scope.insert(*child) {
                queue.push_back(*child);
            }
        }
    }
    let mut scope: Vec<i64> = scope.into_iter().collect();
    scope.sort();
    scope
}

/// Resolves a partial replan request. The scope is computed from the real DAG
/// of `state`; the request only provides the seed.
pub fn resolve_replan(
    state: &PlanState,
    request: &ReplanRequest,
) -> std::result::Result<ResolvedPlan, Vec<PlanDiagnostic>> {
    let mut diags: Vec<PlanDiagnostic> = Vec::new();
    let seed_id = match request.seed.id() {
        Some(id) if state.tasks.iter().any(|t| t.id == id) => id,
        _ => {
            return Err(vec![diag(
                Some(request.seed.clone()),
                Some("seed"),
                "unknown_seed",
                "The replan seed must reference an existing task.".to_string(),
            )]);
        }
    };
    let seed_task = state
        .tasks
        .iter()
        .find(|t| t.id == seed_id)
        .expect("seed exists");
    let seed_count = state.attempts.get(&seed_id).copied().unwrap_or(0);
    if !is_immutable(seed_task, seed_count) {
        diags.push(diag(
            Some(request.seed.clone()),
            Some("seed"),
            "mutable_seed",
            "A replan can only be seeded by a task that has already produced work.".to_string(),
        ));
    }

    let scope: HashSet<i64> = mutable_scope(state, seed_id).into_iter().collect();
    let allowed_external: HashSet<i64> = state
        .tasks
        .iter()
        .map(|t| t.id)
        .filter(|id| !scope.contains(id))
        .collect();

    // Validate the proposed replan.
    if request.plan.objective.trim().is_empty() {
        diags.push(diag(
            None,
            Some("objective"),
            "empty_objective",
            "The replan needs an objective.".to_string(),
        ));
    }
    if request.plan.tasks.is_empty() {
        diags.push(diag(
            None,
            None,
            "empty_plan",
            "A replan must contain at least one task.".to_string(),
        ));
    }
    if request.plan.tasks.len() > MAX_PLAN_TASKS {
        diags.push(diag(
            None,
            None,
            "too_many_tasks",
            format!("A plan cannot exceed {MAX_PLAN_TASKS} tasks."),
        ));
    }
    let mut seen = HashSet::new();
    let mut plan_placeholders: HashMap<String, i64> = HashMap::new();
    for task in &request.plan.tasks {
        let id = task.id.trim().to_string();
        let tref = TaskRef::ClientId(format!("replan:{id}"));
        if id.is_empty() {
            diags.push(diag(
                Some(tref.clone()),
                Some("id"),
                "empty_task_id",
                "Every task needs an id.".to_string(),
            ));
        }
        if !seen.insert(id.clone()) {
            diags.push(diag(
                Some(tref.clone()),
                Some("id"),
                "duplicate_task_id",
                format!("Duplicate task id '{id}'."),
            ));
        }
        let placeholder = -(plan_placeholders.len() as i64) - 1;
        plan_placeholders.insert(id, placeholder);
        check_content(
            Some(tref.clone()),
            &task.title,
            &task.objective,
            &task.acceptance_criteria,
            &mut diags,
        );
        check_planning_role(&tref, &task.role, task.operation, &mut diags);
    }
    for task in &request.plan.tasks {
        let tref = TaskRef::ClientId(format!("replan:{}", task.id.trim()));
        for dep in &task.dependencies {
            let dep_id = dep.trim();
            if dep_id == task.id.trim() {
                diags.push(diag(
                    Some(tref.clone()),
                    Some("dependencies"),
                    "self_dependency",
                    "A task cannot depend on itself.".to_string(),
                ));
            } else if plan_placeholders.contains_key(dep_id) {
                // In-plan reference: fine.
            } else if let Ok(external) = dep_id.parse::<i64>() {
                if allowed_external.contains(&external) {
                    // Existing immutable (or untouched) external task: fine.
                } else if scope.contains(&external) {
                    diags.push(diag(
                        Some(tref.clone()),
                        Some("dependencies"),
                        "scope_dependency",
                        "The replan cannot depend on a task it is replacing.".to_string(),
                    ));
                } else {
                    diags.push(diag(
                        Some(tref.clone()),
                        Some("dependencies"),
                        "unknown_dependency",
                        format!("Dependency {external} does not exist."),
                    ));
                }
            } else {
                diags.push(diag(
                    Some(tref.clone()),
                    Some("dependencies"),
                    "unknown_dependency",
                    format!("Dependency {dep_id} does not exist."),
                ));
            }
        }
    }

    // Cycle check over the new in-plan subgraph only (external anchors are
    // pre-existing and upstream, so they cannot participate in a new cycle).
    let new_tasks: Vec<ResolvedTask> = request
        .plan
        .tasks
        .iter()
        .map(|task| {
            let placeholder = *plan_placeholders.get(task.id.trim()).expect("registered");
            let deps = task
                .dependencies
                .iter()
                .filter_map(|dep| {
                    plan_placeholders
                        .get(dep.trim())
                        .copied()
                        .or_else(|| dep.trim().parse::<i64>().ok())
                })
                .collect();
            ResolvedTask {
                id: placeholder,
                insert: true,
                title: task.title.clone(),
                objective: task.objective.clone(),
                acceptance_criteria: task.acceptance_criteria.clone(),
                state: TaskState::Pending,
                position: i32::MAX,
                dependencies: deps,
                role: task.role.clone(),
                operation: task.operation,
            }
        })
        .collect();
    detect_cycle(&new_tasks, &mut diags);

    // Keep every unaffected task; splice the new tasks where the removed scope
    // roughly sat. The splice index counts *surviving* tasks only, so it is
    // computed against `remaining` (the scope is excluded from it).
    let mut remaining: Vec<ResolvedTask> = state
        .tasks
        .iter()
        .filter(|t| !scope.contains(&t.id))
        .map(resolved_from_task)
        .collect();
    let mut splice_position = remaining.len();
    if let Some(earliest) = state
        .tasks
        .iter()
        .filter(|t| scope.contains(&t.id))
        .map(|t| t.position)
        .min()
    {
        if let Some(after) = remaining.iter().position(|t| t.position > earliest) {
            splice_position = after;
        }
    }
    let mut resolved_tasks: Vec<ResolvedTask> =
        Vec::with_capacity(remaining.len() + new_tasks.len());
    resolved_tasks.extend(remaining.drain(..splice_position));
    resolved_tasks.extend(new_tasks);
    resolved_tasks.extend(remaining);
    for (index, task) in resolved_tasks.iter_mut().enumerate() {
        task.position = index as i32;
    }
    recompute_states(&mut resolved_tasks, &state.attempts);

    if !diags.is_empty() {
        return Err(diags);
    }

    let mut superseded: Vec<i64> = scope.into_iter().collect();
    superseded.sort();
    Ok(ResolvedPlan {
        revision: state.revision + 1,
        objective: request.plan.objective.trim().to_string(),
        tasks: resolved_tasks,
        removed: Vec::new(),
        superseded,
    })
}
