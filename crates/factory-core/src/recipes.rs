//! Workflow recipes: reusable workflow *configuration* — which roles a
//! workflow prefers, its routing/concurrency defaults, policy preset
//! references, and bounded Planner guidance.
//!
//! A recipe never stores the task DAG, task ids, attempts, sessions, or Git
//! state. The Planner stays responsible for producing the actual plan from
//! the objective; recipes only shape how Factory approaches the work:
//!
//! ```text
//! Recipe → workflow configuration → Planner → generated adaptive DAG
//! ```
//!
//! Built-in recipes ship read-only in the catalog; custom recipes persist in
//! `.factory/recipes.toml` (created lazily — existing projects never need a
//! re-init). When a workflow is created from a recipe, an immutable
//! [`RecipeSnapshot`](factory_types::RecipeSnapshot) is stored on the run, so
//! later recipe edits never mutate existing workflows.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use factory_policy::PolicyPreset;
use factory_types::{Recipe, RecipeRouting};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::{Config, MAX_PARALLEL_TASKS_LIMIT};
use crate::roles::{self, PLANNER};

pub const RECIPE_FILE: &str = "recipes.toml";

pub const MAX_RECIPE_NAME_CHARS: usize = 80;
pub const MAX_RECIPE_DESCRIPTION_CHARS: usize = 500;
/// Recipe guidance is planning *context*, bounded so a recipe can never
/// flood or steer the Planner beyond its rules.
pub const MAX_PLANNER_GUIDANCE_CHARS: usize = 4000;

pub const STANDARD_RECIPE: &str = "standard";
pub const THOROUGH_RECIPE: &str = "thorough";
pub const SECURITY_RECIPE: &str = "security";
pub const BUGFIX_RECIPE: &str = "bugfix";
pub const DOCS_RECIPE: &str = "docs";
pub const CUSTOM_RECIPE: &str = "custom";

#[derive(Debug, Error)]
pub enum RecipeError {
    #[error("failed to read {0}: {1}")]
    Read(PathBuf, std::io::Error),
    #[error("failed to parse {0}: {1}")]
    Parse(PathBuf, String),
    #[error("failed to write {0}: {1}")]
    Write(PathBuf, std::io::Error),
    #[error("failed to serialize {0}: {1}")]
    Serialize(PathBuf, String),
    #[error("invalid recipe: {0}")]
    Invalid(String),
    #[error("recipe '{0}' not found")]
    NotFound(String),
}

impl RecipeError {
    /// Whether this error is user-input shaped (bad recipe payload) rather
    /// than an infrastructure failure.
    pub fn is_invalid(&self) -> bool {
        matches!(self, RecipeError::Invalid(_) | RecipeError::NotFound(_))
    }
}

/// The small, fixed built-in catalog. Deliberately not dozens of presets:
/// recipes shape strategy, the Planner adapts to the actual objective.
pub fn builtin_recipes() -> Vec<Recipe> {
    vec![
        Recipe {
            id: STANDARD_RECIPE.into(),
            name: "Standard Development".into(),
            description: "Balanced default: plan, implement, review.".into(),
            roles: vec![PLANNER.into(), roles::WORKER.into(), roles::REVIEWER.into()],
            routing: RecipeRouting::ProjectDefault,
            max_parallel_tasks: Some(4),
            policy_presets: BTreeMap::new(),
            planner_guidance: String::new(),
            github: None,
            builtin: true,
        },
        Recipe {
            id: THOROUGH_RECIPE.into(),
            name: "Thorough Development".into(),
            description: "Architecture, implementation, verification, review, and \
                          documentation for substantial changes."
                .into(),
            roles: vec![
                PLANNER.into(),
                roles::ARCHITECT.into(),
                roles::WORKER.into(),
                roles::TEST_ENGINEER.into(),
                roles::REVIEWER.into(),
                roles::DOCUMENTATION_WRITER.into(),
            ],
            routing: RecipeRouting::ProjectDefault,
            max_parallel_tasks: None,
            policy_presets: BTreeMap::new(),
            planner_guidance: "Use Architect and Documentation Writer only when the change is \
                                substantial enough to need them; verify behavioral changes with \
                                the Test Engineer."
                .into(),
            github: None,
            builtin: true,
        },
        Recipe {
            id: SECURITY_RECIPE.into(),
            name: "Security Sensitive".into(),
            description: "Architecture, verification, an explicit security review, and \
                          narrower policy presets for security-relevant work."
                .into(),
            roles: vec![
                PLANNER.into(),
                roles::ARCHITECT.into(),
                roles::WORKER.into(),
                roles::TEST_ENGINEER.into(),
                roles::SECURITY_AUDITOR.into(),
                roles::REVIEWER.into(),
            ],
            routing: RecipeRouting::ProjectDefault,
            max_parallel_tasks: Some(2),
            policy_presets: BTreeMap::from([
                ("planner".to_string(), "read_only".to_string()),
                ("architect".to_string(), "read_only".to_string()),
                ("worker".to_string(), "implementation".to_string()),
                ("test_engineer".to_string(), "implementation".to_string()),
                ("security_auditor".to_string(), "read_only".to_string()),
                ("reviewer".to_string(), "review".to_string()),
            ]),
            planner_guidance: "Treat the objective as security-sensitive: put a Security \
                               Auditor review after the security-relevant implementation, cover \
                               the changed behavior with tests, and keep the plan proportional \
                               to the actual risk."
                .into(),
            github: None,
            builtin: true,
        },
        Recipe {
            id: BUGFIX_RECIPE.into(),
            name: "Bug Fix".into(),
            description: "Compact diagnosis, fix, and regression verification for a defect.".into(),
            roles: vec![
                PLANNER.into(),
                roles::WORKER.into(),
                roles::TEST_ENGINEER.into(),
                roles::REVIEWER.into(),
            ],
            routing: RecipeRouting::ProjectDefault,
            max_parallel_tasks: None,
            policy_presets: BTreeMap::new(),
            planner_guidance: "Prefer a compact workflow: one focused diagnosis/fix task, a \
                                regression test that fails without the fix, and a review. Avoid \
                                architecture or documentation tasks unless the bug exposes a \
                                structural problem."
                .into(),
            github: None,
            builtin: true,
        },
        Recipe {
            id: DOCS_RECIPE.into(),
            name: "Documentation".into(),
            description: "Documentation-first workflow with a review pass.".into(),
            roles: vec![
                PLANNER.into(),
                roles::DOCUMENTATION_WRITER.into(),
                roles::REVIEWER.into(),
            ],
            routing: RecipeRouting::ProjectDefault,
            max_parallel_tasks: Some(2),
            policy_presets: BTreeMap::from([(
                "documentation_writer".to_string(),
                "documentation".to_string(),
            )]),
            planner_guidance: "Keep every task inside documentation: no production code changes."
                .into(),
            github: None,
            builtin: true,
        },
        Recipe {
            id: CUSTOM_RECIPE.into(),
            name: "Custom".into(),
            description: "User-defined configuration; pre-fills nothing.".into(),
            roles: Vec::new(),
            routing: RecipeRouting::ProjectDefault,
            max_parallel_tasks: None,
            policy_presets: BTreeMap::new(),
            planner_guidance: String::new(),
            github: None,
            builtin: true,
        },
    ]
}

/// The read-only ids of the built-in catalog.
pub fn is_builtin_recipe(id: &str) -> bool {
    builtin_recipes().iter().any(|recipe| recipe.id == id)
}

/// `.factory/recipes.toml` — the project-local custom recipe store.
pub fn recipe_path(root: &Path) -> PathBuf {
    root.join(crate::factory::FACTORY_DIR).join(RECIPE_FILE)
}

/// The on-disk form (snake_case TOML, table key = recipe id), mirroring the
/// documented file format:
///
/// ```toml
/// [recipes.backend_feature]
/// name = "Backend Feature"
/// description = "..."
/// roles = ["planner", "worker", "reviewer"]
/// routing = "performance"
/// max_parallel_tasks = 3
/// planner_guidance = """..."""
/// ```
#[derive(Debug, Default, Serialize, Deserialize)]
struct RecipesFile {
    #[serde(default)]
    recipes: BTreeMap<String, RecipeEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RecipeEntry {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    roles: Vec<String>,
    #[serde(default)]
    routing: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_parallel_tasks: Option<usize>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    policy_presets: BTreeMap<String, String>,
    #[serde(default)]
    planner_guidance: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    github: Option<RecipeDeliveryEntry>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
struct RecipeDeliveryEntry {
    #[serde(default)]
    draft: bool,
}

fn recipe_from_entry(id: String, entry: RecipeEntry) -> Result<Recipe, RecipeError> {
    let routing = if entry.routing.is_empty() {
        RecipeRouting::ProjectDefault
    } else {
        entry
            .routing
            .parse()
            .map_err(|reason: String| RecipeError::Invalid(format!("recipe '{id}': {reason}")))?
    };
    Ok(Recipe {
        id,
        name: entry.name,
        description: entry.description,
        roles: entry.roles,
        routing,
        max_parallel_tasks: entry.max_parallel_tasks,
        policy_presets: entry.policy_presets,
        planner_guidance: entry.planner_guidance,
        github: entry.github.map(|delivery| factory_types::RecipeDelivery {
            draft: delivery.draft,
        }),
        builtin: false,
    })
}

fn recipe_to_entry(recipe: &Recipe) -> (String, RecipeEntry) {
    (
        recipe.id.clone(),
        RecipeEntry {
            name: recipe.name.clone(),
            description: recipe.description.clone(),
            roles: recipe.roles.clone(),
            routing: recipe.routing.as_str().to_string(),
            max_parallel_tasks: recipe.max_parallel_tasks,
            policy_presets: recipe.policy_presets.clone(),
            planner_guidance: recipe.planner_guidance.clone(),
            github: recipe.github.map(|delivery| RecipeDeliveryEntry {
                draft: delivery.draft,
            }),
        },
    )
}

/// Built-in plus project-local custom recipes.
#[derive(Debug, Clone)]
pub struct RecipeCatalog {
    builtin: Vec<Recipe>,
    custom: Vec<Recipe>,
}

impl RecipeCatalog {
    /// Loads the catalog. A missing `recipes.toml` simply yields the built-in
    /// catalog — existing projects stay valid without any migration.
    pub fn load(root: &Path) -> Result<RecipeCatalog, RecipeError> {
        let path = recipe_path(root);
        let custom = if path.exists() {
            let text = std::fs::read_to_string(&path)
                .map_err(|error| RecipeError::Read(path.clone(), error))?;
            let file: RecipesFile = toml::from_str(&text)
                .map_err(|error| RecipeError::Parse(path.clone(), error.to_string()))?;
            file.recipes
                .into_iter()
                .map(|(id, entry)| recipe_from_entry(id, entry))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        Ok(RecipeCatalog {
            builtin: builtin_recipes(),
            custom,
        })
    }

    /// Built-ins first (in catalog order), then custom recipes by id.
    pub fn list(&self) -> Vec<&Recipe> {
        let mut recipes: Vec<&Recipe> = self.builtin.iter().collect();
        recipes.extend(self.custom.iter());
        recipes
    }

    pub fn get(&self, id: &str) -> Option<&Recipe> {
        self.builtin
            .iter()
            .chain(self.custom.iter())
            .find(|recipe| recipe.id == id)
    }

    pub fn custom_recipes(&self) -> &[Recipe] {
        &self.custom
    }

    pub fn is_builtin(&self, id: &str) -> bool {
        self.builtin.iter().any(|recipe| recipe.id == id)
    }
}

fn valid_recipe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

/// Validates one recipe against the project configuration. `allow_empty_roles`
/// is set only for the built-in `custom` placeholder; user recipes must select
/// at least the planner.
pub fn validate_recipe(
    recipe: &Recipe,
    config: &Config,
    allow_empty_roles: bool,
) -> Result<(), String> {
    if !valid_recipe_id(&recipe.id) {
        return Err(format!(
            "recipe id '{}' is invalid; use lowercase letters, digits, '-' or '_'",
            recipe.id
        ));
    }
    let name = recipe.name.trim();
    if name.is_empty() {
        return Err(format!("recipe '{}' has no name", recipe.id));
    }
    if name.chars().count() > MAX_RECIPE_NAME_CHARS {
        return Err(format!(
            "recipe '{}' name exceeds {MAX_RECIPE_NAME_CHARS} characters",
            recipe.id
        ));
    }
    if recipe.description.chars().count() > MAX_RECIPE_DESCRIPTION_CHARS {
        return Err(format!(
            "recipe '{}' description exceeds {MAX_RECIPE_DESCRIPTION_CHARS} characters",
            recipe.id
        ));
    }
    let known_role = |role: &str| roles::is_core_role(role) || config.roles.contains_key(role);
    let mut seen = std::collections::HashSet::new();
    for role in &recipe.roles {
        if !known_role(role) {
            return Err(format!("recipe '{}' uses unknown role '{role}'", recipe.id));
        }
        if !seen.insert(role.clone()) {
            return Err(format!("recipe '{}' lists role '{role}' twice", recipe.id));
        }
    }
    if recipe.roles.is_empty() && !allow_empty_roles {
        return Err(format!(
            "recipe '{}' selects no roles; include at least the planner",
            recipe.id
        ));
    }
    if !recipe.roles.is_empty() && !recipe.roles.iter().any(|role| role == PLANNER) {
        return Err(format!(
            "recipe '{}' does not include the planner role",
            recipe.id
        ));
    }
    if let Some(max_parallel_tasks) = recipe.max_parallel_tasks {
        if !(1..=MAX_PARALLEL_TASKS_LIMIT).contains(&max_parallel_tasks) {
            return Err(format!(
                "recipe '{}' max_parallel_tasks must be between 1 and \
                 {MAX_PARALLEL_TASKS_LIMIT}",
                recipe.id
            ));
        }
    }
    for (role, preset) in &recipe.policy_presets {
        if !known_role(role) {
            return Err(format!(
                "recipe '{}' sets a policy preset for unknown role '{role}'",
                recipe.id
            ));
        }
        if PolicyPreset::parse(preset).is_none() {
            return Err(format!(
                "recipe '{}' references unknown policy preset '{preset}' (expected read_only, \
                 implementation, documentation, review, or custom)",
                recipe.id
            ));
        }
    }
    if recipe.planner_guidance.chars().count() > MAX_PLANNER_GUIDANCE_CHARS {
        return Err(format!(
            "recipe '{}' planner guidance exceeds {MAX_PLANNER_GUIDANCE_CHARS} characters",
            recipe.id
        ));
    }
    Ok(())
}

fn load_custom(root: &Path) -> Result<Vec<Recipe>, RecipeError> {
    Ok(RecipeCatalog::load(root)?.custom_recipes().to_vec())
}

fn write_custom(root: &Path, recipes: &[Recipe], config: &Config) -> Result<(), RecipeError> {
    // Validate the whole file before writing: a rejected recipe never lands
    // on disk (atomic rejection).
    for recipe in recipes {
        validate_recipe(recipe, config, false).map_err(RecipeError::Invalid)?;
    }
    let mut file = RecipesFile::default();
    for recipe in recipes {
        let (id, entry) = recipe_to_entry(recipe);
        file.recipes.insert(id, entry);
    }
    let path = recipe_path(root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| RecipeError::Write(path.clone(), error))?;
    }
    let text = toml::to_string_pretty(&file)
        .map_err(|error| RecipeError::Serialize(path.clone(), error.to_string()))?;
    let tmp = path.with_extension(format!("toml.tmp{}", std::process::id()));
    std::fs::write(&tmp, text).map_err(|error| RecipeError::Write(tmp.clone(), error))?;
    std::fs::rename(&tmp, &path).map_err(|error| RecipeError::Write(path.clone(), error))?;
    Ok(())
}

/// Creates a custom recipe. Rejects built-in id collisions and duplicates.
pub fn create_recipe(root: &Path, config: &Config, recipe: Recipe) -> Result<Recipe, RecipeError> {
    let mut custom = load_custom(root)?;
    if is_builtin_recipe(&recipe.id) {
        return Err(RecipeError::Invalid(format!(
            "recipe id '{}' collides with a built-in recipe; pick another id",
            recipe.id
        )));
    }
    if custom.iter().any(|existing| existing.id == recipe.id) {
        return Err(RecipeError::Invalid(format!(
            "recipe '{}' already exists",
            recipe.id
        )));
    }
    let mut recipe = recipe;
    recipe.builtin = false;
    validate_recipe(&recipe, config, false).map_err(RecipeError::Invalid)?;
    custom.push(recipe.clone());
    custom.sort_by(|a, b| a.id.cmp(&b.id));
    write_custom(root, &custom, config)?;
    Ok(recipe)
}

/// Replaces a custom recipe. The id is immutable; built-ins are read-only.
pub fn update_recipe(
    root: &Path,
    config: &Config,
    id: &str,
    recipe: Recipe,
) -> Result<Recipe, RecipeError> {
    if is_builtin_recipe(id) {
        return Err(RecipeError::Invalid(format!(
            "built-in recipe '{id}' is read-only; duplicate it into a custom recipe to customize"
        )));
    }
    let mut custom = load_custom(root)?;
    let Some(existing) = custom.iter_mut().find(|recipe| recipe.id == id) else {
        return Err(RecipeError::NotFound(id.to_string()));
    };
    if recipe.id != id {
        return Err(RecipeError::Invalid(format!(
            "recipe ids are stable; cannot rename '{id}' to '{}'",
            recipe.id
        )));
    }
    let mut recipe = recipe;
    recipe.builtin = false;
    validate_recipe(&recipe, config, false).map_err(RecipeError::Invalid)?;
    *existing = recipe.clone();
    write_custom(root, &custom, config)?;
    Ok(recipe)
}

/// Deletes a custom recipe. Built-ins cannot be deleted.
pub fn delete_recipe(root: &Path, config: &Config, id: &str) -> Result<(), RecipeError> {
    if is_builtin_recipe(id) {
        return Err(RecipeError::Invalid(format!(
            "built-in recipe '{id}' cannot be deleted"
        )));
    }
    let mut custom = load_custom(root)?;
    let before = custom.len();
    custom.retain(|recipe| recipe.id != id);
    if custom.len() == before {
        return Err(RecipeError::NotFound(id.to_string()));
    }
    write_custom(root, &custom, config)
}

/// The only-narrowing policy restriction a snapshot applies to a role, when
/// it references a preset for that role. The Policy Engine stays
/// authoritative; the preset scope can only intersect existing access.
pub fn snapshot_policy_restriction(
    snapshot: &factory_types::RecipeSnapshot,
    role: &str,
) -> Option<factory_policy::PolicyScope> {
    let preset = snapshot.policy_preset_for(role)?;
    let preset = PolicyPreset::parse(preset)?;
    Some(preset.scope())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgentEntry;
    use factory_types::RecipeDelivery;

    fn agent_entry(command: &str) -> AgentEntry {
        AgentEntry {
            kind: None,
            command: command.into(),
            args: vec!["exec".into()],
            env: BTreeMap::new(),
            prompt_transport: None,
            interactive_args: None,
            capabilities: Vec::new(),
            max_concurrency: None,
        }
    }

    fn test_config() -> Config {
        let mut config = Config::default();
        config.agents.insert("codex".into(), agent_entry("codex"));
        config
    }

    #[test]
    fn builtin_catalog_matches_the_documented_set() {
        let recipes = builtin_recipes();
        let ids: Vec<&str> = recipes.iter().map(|recipe| recipe.id.as_str()).collect();
        assert_eq!(
            ids,
            ["standard", "thorough", "security", "bugfix", "docs", "custom"]
        );
        assert!(recipes.iter().all(|recipe| recipe.builtin));
        let by_id = |id: &str| recipes.iter().find(|recipe| recipe.id == id).unwrap();

        assert_eq!(
            by_id(STANDARD_RECIPE).roles,
            ["planner", "worker", "reviewer"]
        );
        assert_eq!(
            by_id(THOROUGH_RECIPE).roles,
            [
                "planner",
                "architect",
                "worker",
                "test_engineer",
                "reviewer",
                "documentation_writer"
            ]
        );
        assert_eq!(
            by_id(SECURITY_RECIPE).roles,
            [
                "planner",
                "architect",
                "worker",
                "test_engineer",
                "security_auditor",
                "reviewer"
            ]
        );
        assert_eq!(
            by_id(BUGFIX_RECIPE).roles,
            ["planner", "worker", "test_engineer", "reviewer"]
        );
        assert_eq!(
            by_id(DOCS_RECIPE).roles,
            ["planner", "documentation_writer", "reviewer"]
        );
        assert!(by_id(CUSTOM_RECIPE).roles.is_empty());

        // Concurrency preferences from the design.
        assert_eq!(by_id(STANDARD_RECIPE).max_parallel_tasks, Some(4));
        assert_eq!(by_id(SECURITY_RECIPE).max_parallel_tasks, Some(2));
        assert_eq!(by_id(DOCS_RECIPE).max_parallel_tasks, Some(2));
        // Security Sensitive references stricter existing presets.
        assert_eq!(
            by_id(SECURITY_RECIPE)
                .policy_presets
                .get("security_auditor")
                .map(String::as_str),
            Some("read_only")
        );
        assert_eq!(
            by_id(SECURITY_RECIPE)
                .policy_presets
                .get("worker")
                .map(String::as_str),
            Some("implementation")
        );
        // Bug Fix guidance favors a compact diagnosis/fix/regression flow.
        let guidance = by_id(BUGFIX_RECIPE).planner_guidance.clone();
        assert!(guidance.to_lowercase().contains("regression"));
        // Every built-in validates (custom allows its empty role list).
        let config = test_config();
        for recipe in &recipes {
            assert!(validate_recipe(recipe, &config, recipe.id == CUSTOM_RECIPE).is_ok());
        }
    }

    #[test]
    fn custom_recipes_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".factory")).unwrap();
        let config = test_config();
        let recipe = Recipe {
            id: "backend_feature".into(),
            name: "Backend Feature".into(),
            description: "Backend implementation.".into(),
            roles: vec!["planner".into(), "worker".into(), "reviewer".into()],
            routing: RecipeRouting::Performance,
            max_parallel_tasks: Some(3),
            policy_presets: BTreeMap::from([("worker".to_string(), "implementation".to_string())]),
            planner_guidance: "Prefer a compact dependency-aware plan.".into(),
            github: Some(RecipeDelivery { draft: true }),
            builtin: false,
        };
        create_recipe(dir.path(), &config, recipe.clone()).unwrap();

        let text = std::fs::read_to_string(recipe_path(dir.path())).unwrap();
        assert!(text.contains("[recipes.backend_feature]"));
        assert!(text.contains("max_parallel_tasks = 3"));
        assert!(text.contains("routing = \"performance\""));

        let catalog = RecipeCatalog::load(dir.path()).unwrap();
        let loaded = catalog.get("backend_feature").unwrap();
        assert_eq!(loaded, &recipe);
        assert!(!catalog.is_builtin("backend_feature"));
        // The built-ins stay present next to the custom recipe.
        assert!(catalog.get(STANDARD_RECIPE).is_some());
        assert_eq!(catalog.list().len(), 7);
    }

    #[test]
    fn catalog_loads_without_a_recipes_file() {
        let dir = tempfile::tempdir().unwrap();
        // No .factory at all — existing projects must keep working.
        let catalog = RecipeCatalog::load(dir.path()).unwrap();
        assert_eq!(catalog.list().len(), 6);
        assert!(catalog.custom_recipes().is_empty());
    }

    #[test]
    fn invalid_recipes_are_rejected() {
        let config = test_config();
        let base = || Recipe {
            id: "valid_id".into(),
            name: "Valid".into(),
            description: String::new(),
            roles: vec!["planner".into(), "worker".into()],
            routing: RecipeRouting::ProjectDefault,
            max_parallel_tasks: None,
            policy_presets: BTreeMap::new(),
            planner_guidance: String::new(),
            github: None,
            builtin: false,
        };

        let mut recipe = base();
        recipe.roles = vec!["planner".into(), "ghost_role".into()];
        assert!(validate_recipe(&recipe, &config, false)
            .unwrap_err()
            .contains("unknown role"));

        let mut recipe = base();
        recipe.roles = vec!["planner".into(), "planner".into()];
        assert!(validate_recipe(&recipe, &config, false)
            .unwrap_err()
            .contains("twice"));

        let mut recipe = base();
        recipe.roles = vec!["worker".into()];
        assert!(validate_recipe(&recipe, &config, false)
            .unwrap_err()
            .contains("planner"));

        let mut recipe = base();
        recipe.roles = Vec::new();
        assert!(validate_recipe(&recipe, &config, false).is_err());

        let mut recipe = base();
        recipe.max_parallel_tasks = Some(0);
        assert!(validate_recipe(&recipe, &config, false)
            .unwrap_err()
            .contains("max_parallel_tasks"));
        let mut recipe = base();
        recipe.max_parallel_tasks = Some(33);
        assert!(validate_recipe(&recipe, &config, false).is_err());

        let mut recipe = base();
        recipe.policy_presets = BTreeMap::from([("worker".to_string(), "superuser".to_string())]);
        assert!(validate_recipe(&recipe, &config, false)
            .unwrap_err()
            .contains("unknown policy preset"));

        let mut recipe = base();
        recipe.planner_guidance = "x".repeat(MAX_PLANNER_GUIDANCE_CHARS + 1);
        assert!(validate_recipe(&recipe, &config, false)
            .unwrap_err()
            .contains("guidance"));

        let mut recipe = base();
        recipe.name = "   ".into();
        assert!(validate_recipe(&recipe, &config, false)
            .unwrap_err()
            .contains("no name"));

        let mut recipe = base();
        recipe.id = "Not A Slug!".into();
        assert!(validate_recipe(&recipe, &config, false).is_err());
    }

    #[test]
    fn crud_rejects_builtin_collisions_and_bad_ids_atomically() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".factory")).unwrap();
        let config = test_config();

        let collision = Recipe {
            id: STANDARD_RECIPE.into(),
            name: "Copy".into(),
            roles: vec!["planner".into()],
            ..empty()
        };
        assert!(create_recipe(dir.path(), &config, collision).is_err());
        assert!(!recipe_path(dir.path()).exists(), "nothing was written");

        let good = Recipe {
            id: "feature".into(),
            name: "Feature".into(),
            roles: vec!["planner".into(), "worker".into()],
            ..empty()
        };
        create_recipe(dir.path(), &config, good.clone()).unwrap();
        assert!(create_recipe(dir.path(), &config, good).is_err());

        let updated = Recipe {
            id: "feature".into(),
            name: "Renamed Feature".into(),
            roles: vec!["planner".into(), "worker".into(), "reviewer".into()],
            ..empty()
        };
        assert_eq!(
            update_recipe(dir.path(), &config, "feature", updated.clone())
                .unwrap()
                .name,
            "Renamed Feature"
        );
        let renamed = Recipe {
            id: "other".into(),
            ..updated
        };
        assert!(update_recipe(dir.path(), &config, "feature", renamed).is_err());

        assert!(update_recipe(dir.path(), &config, SECURITY_RECIPE, empty()).is_err());
        assert!(delete_recipe(dir.path(), &config, DOCS_RECIPE).is_err());
        assert!(delete_recipe(dir.path(), &config, "missing").is_err());

        delete_recipe(dir.path(), &config, "feature").unwrap();
        assert!(RecipeCatalog::load(dir.path())
            .unwrap()
            .get("feature")
            .is_none());

        // An invalid update never reaches the file.
        let before = std::fs::read_to_string(recipe_path(dir.path())).unwrap();
        let bad = Recipe {
            id: "another".into(),
            name: "Another".into(),
            roles: vec!["planner".into(), "ghost".into()],
            ..empty()
        };
        assert!(create_recipe(dir.path(), &config, bad).is_err());
        assert!(create_recipe(
            dir.path(),
            &config,
            Recipe {
                roles: vec!["planner".into(), "ghost".into()],
                ..empty()
            }
        )
        .is_err());
        assert_eq!(
            std::fs::read_to_string(recipe_path(dir.path())).unwrap(),
            before
        );
    }

    #[test]
    fn snapshot_restriction_resolves_named_presets() {
        let mut snapshot = empty().snapshot();
        snapshot
            .policy_presets
            .insert("worker".to_string(), "read_only".to_string());
        assert!(snapshot_policy_restriction(&snapshot, "worker").is_some());
        assert!(snapshot_policy_restriction(&snapshot, "reviewer").is_none());
        let read_only = snapshot_policy_restriction(&snapshot, "worker").unwrap();
        assert!(read_only.filesystem.unwrap().write.is_empty());
    }

    fn empty() -> Recipe {
        Recipe {
            id: "x".into(),
            name: "X".into(),
            description: String::new(),
            roles: vec!["planner".into()],
            routing: RecipeRouting::ProjectDefault,
            max_parallel_tasks: None,
            policy_presets: BTreeMap::new(),
            planner_guidance: String::new(),
            github: None,
            builtin: false,
        }
    }
}
