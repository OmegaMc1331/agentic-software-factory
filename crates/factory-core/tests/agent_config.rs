use std::collections::BTreeMap;
use std::path::Path;

use factory_agent::{AgentKind, PromptTransport};
use factory_core::{AgentEntry, AgentResolutionError, Agents, Config, RoleAssignment};
use tempfile::TempDir;

fn write_config(dir: &Path, content: &str) {
    let factory = dir.join(".factory");
    std::fs::create_dir_all(&factory).unwrap();
    std::fs::write(factory.join("config.toml"), content).unwrap();
}

fn build_entry(command: &str, args: Vec<String>) -> AgentEntry {
    AgentEntry {
        kind: None,
        command: command.to_string(),
        args,
        env: BTreeMap::new(),
        prompt_transport: None,
        interactive_args: None,
        capabilities: Vec::new(),
        max_concurrency: None,
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
    assert_eq!(agent.kind, AgentKind::Codex);
    assert_eq!(agent.prompt_transport, PromptTransport::Argument);
}

#[test]
fn legacy_custom_configuration_keeps_stdin_transport() {
    let entry = build_entry("my-coding-agent", vec!["--batch".into()]);
    let config = Config {
        agents: BTreeMap::from([("custom".into(), entry)]),
        roles: BTreeMap::new(),
        role_assignments: Vec::new(),
        runtime: Default::default(),
        context: Default::default(),
    };
    let agent = config.agent_config("custom").unwrap();
    assert_eq!(agent.kind, AgentKind::Custom);
    assert_eq!(agent.prompt_transport, PromptTransport::Stdin);
    assert!(agent.interactive_args.is_none());
}

#[test]
fn explicit_known_kind_supplies_its_workflow_arguments() {
    let config: Config = toml::from_str(
        r#"
[agents.gemini]
kind = "gemini_cli"
command = "gemini"
"#,
    )
    .unwrap();
    let agent = config.agent_config("gemini").unwrap();
    assert_eq!(agent.args, vec!["-p"]);
    assert_eq!(agent.prompt_transport, PromptTransport::Argument);
}

#[test]
fn interactive_only_agent_is_rejected_for_a_workflow_role() {
    let dir = TempDir::new().unwrap();
    let known_good = if cfg!(windows) { "powershell" } else { "sh" };
    write_config(
        dir.path(),
        &format!(
            r#"
[agents.console]
kind = "custom"
command = "{known_good}"
prompt_transport = "disabled"
interactive_args = []
[roles.worker]
agent = "console"
"#
        ),
    );
    let error = Agents::load(dir.path())
        .unwrap()
        .command_agent("worker")
        .unwrap_err();
    assert!(matches!(
        error,
        AgentResolutionError::AutomatedUnavailable(_, _)
    ));
    assert!(error
        .to_string()
        .contains("cannot be used as Worker because it has no non-interactive invocation"));
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

#[cfg(windows)]
#[test]
fn a_broken_executable_installation_is_reported_not_available() {
    let dir = TempDir::new().unwrap();
    let target_dir = dir.path().join("fake-package").join("bin");
    std::fs::create_dir_all(&target_dir).unwrap();
    let target = target_dir.join("fake-agent.exe");
    std::fs::write(&target, "text placeholder shipped by a broken package\n").unwrap();
    let shim = dir.path().join("fake-agent.cmd");
    std::fs::write(
        &shim,
        "@ECHO off\r\n\"%dp0%\\fake-package\\bin\\fake-agent.exe\" %*\r\n",
    )
    .unwrap();
    let command = shim.to_string_lossy().replace('\\', "\\\\");
    write_config(
        dir.path(),
        &format!(
            r#"
[agents.broken]
command = "{command}"
[roles.worker]
agent = "broken"
"#
        ),
    );

    let agents = Agents::load(dir.path()).unwrap();
    let infos = agents.list();
    let broken = infos.iter().find(|i| i.name == "broken").unwrap();
    assert!(!broken.available);
    assert_eq!(broken.status, factory_core::AgentStatus::Broken);
    assert!(broken.resolution_shim.is_some());
    assert!(broken
        .resolution_target
        .as_deref()
        .unwrap()
        .contains("fake-package"));
    let error = broken
        .resolution_error
        .as_deref()
        .expect("broken agent carries a resolution error");
    assert!(error.contains("invalid"));
    assert!(error.contains("Reinstall the CLI"));

    let err = agents.command_agent("worker").unwrap_err();
    assert!(matches!(err, AgentResolutionError::Broken(_, _, _)));
    assert!(err.to_string().contains(
        "Worker agent `broken` cannot start because its executable installation is broken"
    ));
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
    assert!(codex.resolved_executable.is_some());
    assert!(codex.resolution_error.is_none());
    let ghost = infos.iter().find(|i| i.name == "ghost").unwrap();
    assert!(!ghost.available);
    assert!(ghost.resolved_executable.is_none());
    assert!(ghost
        .resolution_error
        .as_deref()
        .unwrap()
        .contains("PATH visible to Factory"));
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
    factory_core::Factory::init(dir.path()).unwrap();
    let config = Config::load(dir.path()).unwrap();
    assert!(config.agents.contains_key("codex"));
    assert_eq!(config.agent_for_role("planner"), Some("codex".to_string()));
}

#[test]
fn write_atomic_round_trips_configuration() {
    let dir = TempDir::new().unwrap();
    let config = Config {
        agents: BTreeMap::from([(
            "codex".to_string(),
            build_entry("codex", vec!["exec".to_string()]),
        )]),
        roles: BTreeMap::new(),
        role_assignments: vec![RoleAssignment {
            role: "planner".to_string(),
            agent: "codex".to_string(),
            preferred: true,
        }],
        runtime: Default::default(),
        context: Default::default(),
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
        roles: BTreeMap::new(),
        role_assignments: vec![RoleAssignment {
            role: "planner".into(),
            agent: "ghost".into(),
            preferred: false,
        }],
        runtime: Default::default(),
        context: Default::default(),
    };
    let err = config.validate().unwrap_err();
    assert!(err.contains("unknown agent 'ghost'"));
}

#[test]
fn validation_rejects_empty_commands_and_invalid_names() {
    let config = Config {
        agents: BTreeMap::from([("bad name".into(), build_entry("", vec![]))]),
        roles: BTreeMap::new(),
        role_assignments: Vec::new(),
        runtime: Default::default(),
        context: Default::default(),
    };
    let err = config.validate().unwrap_err();
    assert!(err.contains("invalid agent name"));

    let config = Config {
        agents: BTreeMap::from([("codex".into(), build_entry("", vec![]))]),
        roles: BTreeMap::new(),
        role_assignments: Vec::new(),
        runtime: Default::default(),
        context: Default::default(),
    };
    let err = config.validate().unwrap_err();
    assert!(err.contains("empty command"));
}

#[test]
fn legacy_config_file_migrates_on_disk_with_a_backup() {
    let dir = TempDir::new().unwrap();
    write_config(
        dir.path(),
        r#"
[agents.codex]
command = "codex"
args = ["exec"]

[agents.opencode]
command = "opencode"
args = ["run"]

[agents.claude]
command = "claude"
args = ["-p"]

[roles.planner]
agent = "codex"

[roles.worker]
agent = "opencode"

[roles.reviewer]
agent = "claude"
"#,
    );
    let config = Config::load_and_migrate(dir.path()).unwrap();
    assert_eq!(config.agent_for_role("planner"), Some("codex".to_string()));
    assert_eq!(
        config.agent_for_role("worker"),
        Some("opencode".to_string())
    );
    assert_eq!(
        config.agent_for_role("reviewer"),
        Some("claude".to_string())
    );

    let written = std::fs::read_to_string(dir.path().join(".factory").join("config.toml")).unwrap();
    assert!(written.contains("[[role_assignments]]"), "got:\n{written}");
    assert!(written.contains("role = \"planner\""));
    assert!(written.contains("preferred = true"));
    assert!(!written.contains("[roles.planner]"));

    let backup =
        std::fs::read_to_string(dir.path().join(".factory").join("config.toml.bak")).unwrap();
    assert!(backup.contains("[roles.planner]\nagent = \"codex\""));

    let reloaded = Config::load_and_migrate(dir.path()).unwrap();
    assert_eq!(reloaded, config);
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".factory").join("config.toml")).unwrap(),
        written,
        "second load must not rewrite the file"
    );
}

#[test]
fn current_config_files_load_without_migration_writes() {
    let dir = TempDir::new().unwrap();
    write_config(dir.path(), &factory_core::default_config_text());
    let before = std::fs::read_to_string(dir.path().join(".factory").join("config.toml")).unwrap();
    Config::load_and_migrate(dir.path()).unwrap();
    let after = std::fs::read_to_string(dir.path().join(".factory").join("config.toml")).unwrap();
    assert_eq!(before, after);
    assert!(!dir.path().join(".factory").join("config.toml.bak").exists());
}
