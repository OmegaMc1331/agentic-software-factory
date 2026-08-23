//! Workflow recipes: catalog-backed workflow creation, immutable snapshots,
//! planner guidance, routing/concurrency/policy application, and Save as
//! Recipe. Fake shell-command agents only — no real agent credentials.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

use factory_core::recipes::{create_recipe, update_recipe, RecipeCatalog, RECIPE_FILE};
use factory_core::{Factory, FactoryError, RoleAssignment};
use factory_types::{Recipe, RecipeRouting, RunStatus};
use tempfile::TempDir;

fn init_git(dir: &Path) {
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.name", "Factory Test"],
        vec!["add", "."],
    ] {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(&args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?} failed");
    }
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["commit", "-q", "-m", "init"])
        .status()
        .unwrap()
        .success();
    assert!(ok, "git commit failed");
}

/// A fake planner that (best-effort) captures the mission it received, then
/// prints the plan file. Cross-platform via powershell / sh.
fn capture_planner_config(plan_path: &Path, capture_path: &Path) -> String {
    let (command, args) = if cfg!(windows) {
        (
            "powershell",
            vec![
                "-NoProfile".to_string(),
                "-Command".to_string(),
                format!(
                    "[Console]::In.ReadToEnd() | Set-Content -NoNewline -Encoding utf8 '{}'; \
                     Get-Content -Raw '{}'",
                    capture_path.display(),
                    plan_path.display()
                ),
            ],
        )
    } else {
        (
            "sh",
            vec![
                "-c".to_string(),
                format!(
                    "cat > '{}'; cat '{}'",
                    capture_path.display(),
                    plan_path.display()
                ),
            ],
        )
    };
    agent_config_text(command, &args)
}

fn agent_config_text(command: &str, args: &[String]) -> String {
    format!(
        r#"[agents.fake]
command = "{command}"
args = {args:?}

[[role_assignments]]
role = "planner"
agent = "fake"
preferred = true

[[role_assignments]]
role = "worker"
agent = "fake"
preferred = true

[[role_assignments]]
role = "reviewer"
agent = "fake"
preferred = true

[[role_assignments]]
role = "architect"
agent = "fake"
preferred = true

[[role_assignments]]
role = "test_engineer"
agent = "fake"
preferred = true
"#
    )
}

fn write_config(dir: &Path, text: &str) {
    let factory_dir = dir.join(".factory");
    std::fs::create_dir_all(&factory_dir).unwrap();
    std::fs::write(factory_dir.join("config.toml"), text).unwrap();
}

fn with_factory_and_plan(plan: &str) -> (TempDir, Factory, PathBuf) {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("README.md"), "repo").unwrap();
    init_git(dir.path());
    let plan_path = dir.path().join("plan.json");
    std::fs::write(&plan_path, plan).unwrap();
    let capture_path = dir.path().join("mission.txt");
    write_config(
        dir.path(),
        &capture_planner_config(&plan_path, &capture_path),
    );
    let factory = Factory::init(dir.path()).unwrap();
    (dir, factory, capture_path)
}

const WORKER_ONLY_PLAN: &str = r#"{"objective":"small css fix","tasks":[
{"id":"T1","title":"Fix the button style","objective":"fix the css","dependencies":[],"acceptanceCriteria":["style fixed"],"role":"worker"},
{"id":"T2","title":"Review","objective":"review the fix","dependencies":["T1"],"acceptanceCriteria":["approved"]}
]}"#;

#[test]
fn recipes_load_without_a_recipes_file_and_runs_stay_recipe_less() {
    // Backward compatibility: no `factory init` recipe file, no recipe on the
    // run, no behavior change.
    let (dir, factory, _) = with_factory_and_plan(WORKER_ONLY_PLAN);
    let catalog = factory.recipes().unwrap();
    assert_eq!(catalog.list().len(), 6, "built-ins only");
    assert!(!dir.path().join(".factory").join(RECIPE_FILE).exists());

    let run = factory.begin_run("small css fix", None).unwrap();
    assert!(run.recipe.is_none());
    assert_eq!(
        factory.run_max_parallel_tasks(run.id).unwrap(),
        4,
        "project [runtime] default applies without a recipe"
    );
}

#[test]
fn creating_a_run_from_a_recipe_snapshots_it_immutably() {
    let (dir, factory, _) = with_factory_and_plan(WORKER_ONLY_PLAN);

    let mut recipe = Recipe {
        id: "backend_feature".into(),
        name: "Backend Feature".into(),
        description: "Backend implementation.".into(),
        roles: vec!["planner".into(), "worker".into(), "reviewer".into()],
        routing: RecipeRouting::ProjectDefault,
        max_parallel_tasks: Some(3),
        policy_presets: BTreeMap::new(),
        planner_guidance: "Prefer a compact dependency-aware plan.".into(),
        github: None,
        builtin: false,
    };
    let config = factory.agents().config().clone();
    create_recipe(dir.path(), &config, recipe.clone()).unwrap();

    let run = factory
        .begin_run_with_recipe(
            "small css fix",
            None,
            Some(&factory.recipe_snapshot("backend_feature").unwrap()),
        )
        .unwrap();
    let snapshot = run.recipe.as_ref().unwrap();
    assert_eq!(snapshot.id, "backend_feature");
    assert_eq!(snapshot.name, "Backend Feature");
    assert_eq!(snapshot.max_parallel_tasks, Some(3));
    assert!(snapshot.planner_guidance.contains("compact"));

    // Editing the recipe afterwards must not mutate the existing workflow.
    recipe.name = "Backend Feature v2".into();
    recipe.planner_guidance = "Different guidance.".into();
    recipe.max_parallel_tasks = Some(8);
    update_recipe(dir.path(), &config, "backend_feature", recipe).unwrap();

    let reloaded = factory.get_run(run.id).unwrap().unwrap();
    let snapshot = reloaded.recipe.as_ref().unwrap();
    assert_eq!(snapshot.name, "Backend Feature", "snapshot is immutable");
    assert!(snapshot.planner_guidance.contains("compact"));
    assert_eq!(snapshot.max_parallel_tasks, Some(3));
}

#[test]
fn a_recipe_role_without_an_assignment_is_a_clear_error() {
    let (dir, factory, _) = with_factory_and_plan(WORKER_ONLY_PLAN);
    // security_auditor has no assignment in this project.
    let snapshot = factory.recipe_snapshot("security").unwrap_err();
    let message = snapshot.to_string();
    assert!(
        message.contains("security_auditor"),
        "unexpected: {message}"
    );
    assert!(
        message.contains("Security Sensitive"),
        "the error names the recipe: {message}"
    );
    // Creating the run fails too — never silently replaced by Worker.
    let catalog = RecipeCatalog::load(dir.path()).unwrap();
    let security = catalog.get("security").unwrap().snapshot();
    let error = factory
        .begin_run_with_recipe("add secure password reset", None, Some(&security))
        .unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("security_auditor"),
        "unexpected: {message}"
    );
    assert!(!message.contains("replaced"));
    assert!(matches!(error, FactoryError::InvalidTeam(_)));
}

#[test]
fn recipe_team_prefill_uses_preferred_assignments() {
    let (dir, factory, _) = with_factory_and_plan(WORKER_ONLY_PLAN);
    // Assign the reviewer agent to security_auditor as well.
    let mut config = factory.agents().config().clone();
    config.role_assignments.push(RoleAssignment {
        role: "security_auditor".into(),
        agent: "fake".into(),
        preferred: true,
    });
    config.write_atomic(dir.path()).unwrap();
    let factory = Factory::open(dir.path()).unwrap();

    let snapshot = factory.recipe_snapshot("security").unwrap();
    let run = factory
        .begin_run_with_recipe("add secure password reset", None, Some(&snapshot))
        .unwrap();
    let team = run.team.as_ref().unwrap();
    assert_eq!(team.planner, "fake");
    assert_eq!(team.workers, ["fake"]);
    assert_eq!(
        team.additional.get("security_auditor").map(Vec::as_slice),
        Some(&["fake".to_string()][..]),
        "the recipe's additional roles pre-fill the team"
    );
    // The snapshot itself keeps the full selected role list.
    assert!(run
        .recipe
        .as_ref()
        .unwrap()
        .roles
        .contains(&"security_auditor".to_string()));
}

#[test]
fn planner_receives_bounded_recipe_guidance() {
    let (_dir, factory, capture_path) = with_factory_and_plan(WORKER_ONLY_PLAN);
    let snapshot = factory.recipe_snapshot("bugfix").unwrap();
    let run = factory
        .begin_run_with_recipe("fix the login crash", None, Some(&snapshot))
        .unwrap();
    factory.plan_run(run.id, &AtomicBool::new(false)).unwrap();

    let mission = std::fs::read_to_string(&capture_path).unwrap();
    assert!(mission.contains("WORKFLOW OBJECTIVE") || mission.contains("Objective:"));
    assert!(mission.contains("Available roles"));
    assert!(
        mission.contains("RECIPE\nBug Fix"),
        "recipe is named: {mission}"
    );
    assert!(
        mission.contains("RECIPE GUIDANCE"),
        "guidance block present: {mission}"
    );
    assert!(mission.contains("regression test"));
    assert!(mission.contains("never overrides"));
}

#[test]
fn optional_roles_are_not_forced_into_plans() {
    let (dir, factory, _) = with_factory_and_plan(WORKER_ONLY_PLAN);
    // Thorough selects architect/test_engineer/documentation_writer; the
    // planner output uses only worker tasks for a trivial change.
    let mut config = factory.agents().config().clone();
    config.role_assignments.push(RoleAssignment {
        role: "documentation_writer".into(),
        agent: "fake".into(),
        preferred: true,
    });
    config.write_atomic(dir.path()).unwrap();
    let factory = Factory::open(dir.path()).unwrap();

    let snapshot = factory.recipe_snapshot("thorough").unwrap();
    let outcome = factory
        .begin_run_with_recipe("small css fix", None, Some(&snapshot))
        .and_then(|run| factory.plan_run(run.id, &AtomicBool::new(false)))
        .unwrap();
    assert_eq!(outcome.run.status.as_str(), "planned");
    let roles: Vec<&str> = outcome
        .tasks
        .iter()
        .map(|task| task.role.as_deref().unwrap_or("worker"))
        .collect();
    assert_eq!(
        roles,
        ["worker", "worker"],
        "the Planner stays adaptive: optional roles are never forced in"
    );
}

#[test]
fn recipe_routing_mode_overrides_the_project_default_for_the_run() {
    let (dir, factory, _) = with_factory_and_plan(WORKER_ONLY_PLAN);
    // Project config says round_robin (the default); the recipe pins
    // performance for this run.
    let recipe = Recipe {
        id: "performance_feature".into(),
        name: "Performance Feature".into(),
        description: String::new(),
        roles: vec!["planner".into(), "worker".into(), "reviewer".into()],
        routing: RecipeRouting::Performance,
        max_parallel_tasks: None,
        policy_presets: BTreeMap::new(),
        planner_guidance: String::new(),
        github: None,
        builtin: false,
    };
    let config = factory.agents().config().clone();
    create_recipe(dir.path(), &config, recipe).unwrap();
    let factory = Factory::open(dir.path()).unwrap();
    assert_eq!(
        factory.agents().config().routing.mode.as_str(),
        "round_robin"
    );

    let snapshot = factory.recipe_snapshot("performance_feature").unwrap();
    let outcome = factory
        .begin_run_with_recipe("small css fix", None, Some(&snapshot))
        .and_then(|run| factory.plan_run(run.id, &AtomicBool::new(false)))
        .unwrap();
    let task = outcome
        .tasks
        .iter()
        .find(|task| task.role.as_deref() == Some("worker"))
        .unwrap();
    let preview = factory.routing_preview(task.id).unwrap();
    assert_eq!(preview.mode, "performance", "recipe pins the run's mode");

    // A recipe-less run keeps the project mode.
    let plain = factory
        .begin_run("another objective", None)
        .and_then(|run| factory.plan_run(run.id, &AtomicBool::new(false)))
        .unwrap();
    let plain_task = plain
        .tasks
        .iter()
        .find(|task| task.role.as_deref() == Some("worker"))
        .unwrap();
    let plain_preview = factory.routing_preview(plain_task.id).unwrap();
    assert_eq!(plain_preview.mode, "round_robin");
}

#[test]
fn recipe_concurrency_preference_is_validated_and_applied() {
    let (dir, factory, _) = with_factory_and_plan(WORKER_ONLY_PLAN);
    let config = factory.agents().config().clone();
    let recipe = Recipe {
        id: "low_parallel".into(),
        name: "Low Parallel".into(),
        description: String::new(),
        roles: vec!["planner".into(), "worker".into(), "reviewer".into()],
        routing: RecipeRouting::ProjectDefault,
        max_parallel_tasks: Some(2),
        policy_presets: BTreeMap::new(),
        planner_guidance: String::new(),
        github: None,
        builtin: false,
    };
    create_recipe(dir.path(), &config, recipe.clone()).unwrap();
    let factory = Factory::open(dir.path()).unwrap();
    let snapshot = factory.recipe_snapshot("low_parallel").unwrap();
    let run = factory
        .begin_run_with_recipe("small css fix", None, Some(&snapshot))
        .unwrap();
    assert_eq!(factory.run_max_parallel_tasks(run.id).unwrap(), 2);

    let mut out_of_bounds = recipe.clone();
    out_of_bounds.id = "out_of_bounds".into();
    out_of_bounds.max_parallel_tasks = Some(99);
    assert!(create_recipe(dir.path(), &config, out_of_bounds).is_err());
}

#[test]
fn recipe_policy_narrows_but_never_elevates() {
    // Project config: worker policy implementation (writes allowed). The
    // recipe narrows worker to read_only for this run: an implement task is
    // blocked at the start gate with a policy reason.
    let (dir, factory, _) = with_factory_and_plan(WORKER_ONLY_PLAN);
    let mut config = factory.agents().config().clone();
    config.policies.role_mut("worker").preset = Some(factory_policy::PolicyPreset::Implementation);
    config.policies.role_mut("reviewer").preset = Some(factory_policy::PolicyPreset::ReadOnly);
    config.write_atomic(dir.path()).unwrap();
    let factory = Factory::open(dir.path()).unwrap();

    let mut narrowing = BTreeMap::new();
    narrowing.insert("worker".to_string(), "read_only".to_string());
    let recipe = Recipe {
        id: "locked".into(),
        name: "Locked".into(),
        description: String::new(),
        roles: vec!["planner".into(), "worker".into(), "reviewer".into()],
        routing: RecipeRouting::ProjectDefault,
        max_parallel_tasks: None,
        policy_presets: narrowing,
        planner_guidance: String::new(),
        github: None,
        builtin: false,
    };
    let config = factory.agents().config().clone();
    create_recipe(dir.path(), &config, recipe).unwrap();
    let factory = Factory::open(dir.path()).unwrap();

    let snapshot = factory.recipe_snapshot("locked").unwrap();
    let run = factory
        .begin_run_with_recipe("small css fix", None, Some(&snapshot))
        .unwrap();
    factory.plan_run(run.id, &AtomicBool::new(false)).unwrap();
    let error = factory.prepare_start(run.id).unwrap_err();
    assert!(
        matches!(error, FactoryError::PolicyBlocked(_)),
        "unexpected: {error}"
    );
    let blocked = factory.get_run(run.id).unwrap().unwrap();
    assert_eq!(blocked.status, RunStatus::Blocked);

    // Without the recipe, the same plan starts: the narrowing came from the
    // recipe, and a read_only reviewer with an implementation-preset recipe
    // can never gain writes (policy engine tests cover the elevation
    // directly); here we assert the baseline still works.
    let plain_run = factory
        .begin_run("small css fix", None)
        .and_then(|run| factory.plan_run(run.id, &AtomicBool::new(false)))
        .unwrap();
    factory.prepare_start(plain_run.run.id).unwrap();
    assert_eq!(
        factory.get_run(plain_run.run.id).unwrap().unwrap().status,
        RunStatus::Active
    );
}

#[test]
fn save_as_recipe_captures_configuration_without_history() {
    let (dir, factory, _) = with_factory_and_plan(WORKER_ONLY_PLAN);
    let outcome = factory
        .begin_run_with_recipe(
            "small css fix",
            None,
            Some(&factory.recipe_snapshot("bugfix").unwrap()),
        )
        .and_then(|run| factory.plan_run(run.id, &AtomicBool::new(false)))
        .unwrap();

    let derived = factory.recipe_from_run(outcome.run.id).unwrap();
    assert_eq!(
        derived.roles,
        vec!["planner", "worker", "reviewer", "test_engineer"]
    );
    assert_eq!(derived.routing, RecipeRouting::ProjectDefault);
    assert!(derived.planner_guidance.contains("regression"));
    assert!(
        derived.id.is_empty() && derived.name.is_empty(),
        "caller names it"
    );

    let mut saved = derived;
    saved.id = "saved_from_run".into();
    saved.name = "Saved From Run".into();
    let config = factory.agents().config().clone();
    create_recipe(dir.path(), &config, saved).unwrap();

    let file = std::fs::read_to_string(dir.path().join(".factory").join(RECIPE_FILE)).unwrap();
    assert!(file.contains("[recipes.saved_from_run]"));
    for task in &outcome.tasks {
        let id = format!("T{}", task.id);
        assert!(!file.contains(&id), "no task ids leak into recipes: {file}");
        assert!(
            !file.contains(&task.title),
            "no task titles leak into recipes: {file}"
        );
    }
    // No DAG edges, attempts, sessions, or commits either — the file only has
    // the configuration keys.
    for banned in ["dependencies", "attempt", "session", "commit"] {
        assert!(!file.contains(banned), "unexpected '{banned}' in: {file}");
    }
}

#[test]
fn unknown_recipe_ids_reject_run_creation() {
    let (_dir, factory, _) = with_factory_and_plan(WORKER_ONLY_PLAN);
    let error = factory.recipe_snapshot("does_not_exist").unwrap_err();
    assert!(error.to_string().contains("does_not_exist"));
}
