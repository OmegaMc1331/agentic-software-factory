use std::path::Path;

use factory_core::{AgentResolutionError, Agents, Config};
use tempfile::TempDir;

fn write_config(dir: &Path, content: &str) {
    let factory = dir.join(".factory");
    std::fs::create_dir_all(&factory).unwrap();
    std::fs::write(factory.join("config.toml"), content).unwrap();
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
    write_config(
        dir.path(),
        r#"
[agents.codex]
command = "codex"
[roles.planner]
agent = "codex"
"#,
    );
    let agents = Agents::load(dir.path()).unwrap();
    let command = agents.command_agent("planner").unwrap();
    assert_eq!(command.name(), "codex");
    assert_eq!(command.command(), "codex");
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
        .contains("no agent configured for role `planner`"));
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
        .contains("configured planner agent `ghost` is not available"));
}

#[test]
fn lists_configured_agents_with_availability() {
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"
[agents.codex]
command = "codex"
[agents.ghost]
command = "definitely-not-a-real-factory-test-binary"
"#,
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
