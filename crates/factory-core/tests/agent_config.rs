use std::collections::BTreeMap;
use std::path::Path;

use factory_core::{AgentEntry, AgentResolutionError, Agents, Config};
use tempfile::TempDir;

fn write_config(dir: &Path, content: &str) {
    let factory = dir.join(".factory");
    std::fs::create_dir_all(&factory).unwrap();
    std::fs::write(factory.join("config.toml"), content).unwrap();
}

fn build_entry(command: &str, args: Vec<String>) -> AgentEntry {
    AgentEntry {
        command: command.to_string(),
        args,
        env: BTreeMap::new(),
        capabilities: Vec::new(),
    }
}

#[test]
fn parses_agent_configuration() {
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"
[agents.codex]
command = "codex"
args = ["exec"]
env = { TEST = "1" }
capabilities = ["planner"]

[roles.planner]
agent = "codex"
"#,
    );
    let config = Config::load(dir.path()).unwrap();
    let agent = config.agent_config("codex").unwrap();
    assert_eq!(agent.name, "codex");
    assert_eq!(agent.command, "codex");
    assert_eq!(agent.args, vec!["exec".to_string()]);
    assert_eq!(agent.env.get("TEST").map(|s| s.as_str()), Some("1"));
    assert!(agent.capabilities.supports("planner"));
}

#[test]
fn resolves_role_to_agent() {
    let dir = TempDir::new().unwrap();
    let known_good = if cfg!(windows) { "powershell" } else { "sh" };
    write_config(
        dir.path(),
        &format!(
            r#"
[agents.codex]
command = "{known_good}"
[roles.planner]
agent = "codex"
"#
        ),
    );
    let agents = Agents::load(dir.path()).unwrap();
    let command = agents.command_agent("planner").unwrap();
    assert_eq!(command.name(), "codex");
    assert_eq!(command.command(), known_good);
}

#[test]
fn missing_role_configuration_is_an_error() {
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"
[agents.codex]
command = "codex"
"#,
    );
    let agents = Agents::load(dir.path()).unwrap();
    let err = agents.command_agent("planner").unwrap_err();
    assert!(matches!(err, AgentResolutionError::NoRole(_)));
    assert!(err
        .to_string()
        .contains("No agent is assigned to the planner role. Configure one from the dashboard."));
}

#[test]
fn unknown_role_agent_is_an_error() {
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"
[agents.codex]
command = "codex"
[roles.planner]
agent = "ghost"
"#,
    );
    let agents = Agents::load(dir.path()).unwrap();
    let err = agents.command_agent("planner").unwrap_err();
    assert!(matches!(err, AgentResolutionError::UnknownAgent(_, _)));
}

#[test]
fn missing_executable_detection() {
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"
[agents.ghost]
command = "definitely-not-a-real-factory-test-binary"
[roles.planner]
agent = "ghost"
"#,
    );
    let agents = Agents::load(dir.path()).unwrap();
    let err = agents.command_agent("planner").unwrap_err();
    assert!(matches!(err, AgentResolutionError::NotAvailable(_, _)));
    assert!(err
        .to_string()
        .contains("Planner agent `ghost` is not available. Check the agent configuration."));
}

#[test]
fn lists_configured_agents_with_availability() {
    let dir = TempDir::new().unwrap();
    let known_good = if cfg!(windows) { "powershell" } else { "sh" };
    write_config(
        dir.path(),
        &format!(
            r#"
[agents.codex]
command = "{known_good}"
[agents.ghost]
command = "definitely-not-a-real-factory-test-binary"
"#
        ),
    );
    let agents = Agents::load(dir.path()).unwrap();
    let infos = agents.list();
    assert_eq!(infos.len(), 2);
    let codex = infos.iter().find(|i| i.name == "codex").unwrap();
    assert!(codex.available);
    let ghost = infos.iter().find(|i| i.name == "ghost").unwrap();
    assert!(!ghost.available);
}

#[test]
fn default_config_is_valid_toml_with_all_roles() {
    let dir = TempDir::new().unwrap();
    let factory = dir.path().join(".factory");
    std::fs::create_dir_all(&factory).unwrap();
    std::fs::write(
        factory.join("config.toml"),
        factory_core::default_config_text(),
    )
    .unwrap();
    let config = Config::load(dir.path()).unwrap();
    assert_eq!(config.agent_for_role("planner"), Some("codex".to_string()));
    assert_eq!(
        config.agent_for_role("worker"),
        Some("opencode".to_string())
    );
    assert_eq!(
        config.agent_for_role("reviewer"),
        Some("claude".to_string())
    );
}

#[test]
fn init_does_not_require_agents_to_be_installed() {
    let dir = TempDir::new().unwrap();
    factory_core::Factory::init(dir.path(), false).unwrap();
    let config = Config::load(dir.path()).unwrap();
    assert!(config.agents.contains_key("codex"));
    assert!(config.roles.contains_key("planner"));
}

#[test]
fn write_atomic_round_trips_configuration() {
    let dir = TempDir::new().unwrap();
    let config = Config {
        agents: BTreeMap::from([(
            "codex".to_string(),
            build_entry("codex", vec!["exec".to_string()]),
        )]),
        roles: BTreeMap::from([(
            "planner".to_string(),
            factory_core::config::RoleEntry {
                agent: "codex".to_string(),
            },
        )]),
    };
    let path = config.write_atomic(dir.path()).unwrap();
    assert!(path.ends_with("config.toml"));
    let loaded = Config::load(dir.path()).unwrap();
    assert_eq!(loaded, config);
    assert!(!std::fs::read_dir(dir.path().join(".factory"))
        .unwrap()
        .into_iter()
        .any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("config.toml.tmp")));
}

#[test]
fn validation_rejects_unknown_role_agent() {
    let config = Config {
        agents: BTreeMap::from([("codex".into(), build_entry("codex", vec![]))]),
        roles: BTreeMap::from([(
            "planner".into(),
            factory_core::config::RoleEntry {
                agent: "ghost".into(),
            },
        )]),
    };
    let err = config.validate().unwrap_err();
    assert!(err.contains("unknown agent 'ghost'"));
}

#[test]
fn validation_rejects_empty_commands_and_invalid_names() {
    let config = Config {
        agents: BTreeMap::from([("bad name".into(), build_entry("", vec![]))]),
        roles: BTreeMap::new(),
    };
    let err = config.validate().unwrap_err();
    assert!(err.contains("invalid agent name"));

    let config = Config {
        agents: BTreeMap::from([("codex".into(), build_entry("", vec![]))]),
        roles: BTreeMap::new(),
    };
    let err = config.validate().unwrap_err();
    assert!(err.contains("empty command"));
}
