use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use factory_core::Config;
use factory_mcp::{GrantContext, McpServerConfig, McpTransport, ToolRegistry};
use factory_policy::ToolPolicy;

fn python() -> &'static str {
    if cfg!(windows) {
        "python"
    } else {
        "python3"
    }
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../factory-mcp/tests/fixtures/fake_mcp_server.py")
}

#[test]
fn scoped_cli_bridge_invokes_only_granted_tools() {
    let root = tempfile::tempdir().unwrap();
    factory_core::Factory::init(root.path()).unwrap();
    let mut config = Config::load(root.path()).unwrap();
    config.policies.role_mut("worker").tools = Some(ToolPolicy {
        allow: vec!["fake.search".into()],
        deny: Vec::new(),
    });
    config.mcp.insert(
        "fake".into(),
        McpServerConfig {
            transport: McpTransport::Stdio,
            command: Some(python().into()),
            args: vec![fixture().to_string_lossy().to_string()],
            cwd: None,
            url: None,
            env: BTreeMap::new(),
            headers: BTreeMap::new(),
            timeout_ms: 1_000,
            discovery_timeout_ms: 2_000,
            max_response_bytes: 64_000,
            max_concurrency: 2,
            classifications: BTreeMap::new(),
        },
    );
    config.write_atomic(root.path()).unwrap();
    let registry = ToolRegistry::new(root.path(), config.mcp.clone());
    let tools = registry.discover_server_blocking("fake").unwrap();
    let allowed: Vec<_> = tools
        .into_iter()
        .filter(|tool| tool.name == "search")
        .collect();
    let grant = registry
        .create_grant(
            GrantContext {
                run_id: None,
                task_id: None,
                attempt_id: None,
                session_id: None,
                agent: "codex".into(),
                role: "worker".into(),
                operation: Some("implement".into()),
            },
            &allowed,
        )
        .unwrap()
        .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_factory"))
        .current_dir(root.path())
        .arg("mcp")
        .arg("call")
        .arg("fake.search")
        .arg("--input-json")
        .arg(r#"{"query":"rust"}"#)
        .env("FACTORY_MCP_ROOT", root.path())
        .env("FACTORY_MCP_GRANT", &grant.token)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("UNTRUSTED EXTERNAL TOOL DATA"));
    assert!(stdout.contains("fake.search"));
    assert!(!stdout.contains(&grant.token));

    let denied = Command::new(env!("CARGO_BIN_EXE_factory"))
        .current_dir(root.path())
        .arg("mcp")
        .arg("call")
        .arg("fake.erase")
        .arg("--input-json")
        .arg("{}")
        .env("FACTORY_MCP_ROOT", root.path())
        .env("FACTORY_MCP_GRANT", &grant.token)
        .output()
        .unwrap();
    assert!(!denied.status.success());
    assert!(!String::from_utf8_lossy(&denied.stderr).contains(&grant.token));
}
