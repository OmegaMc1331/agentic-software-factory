use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use factory_core::Config;
use factory_db::FactoryDb;
use factory_mcp::{GrantContext, McpServerConfig, McpTransport, ToolRegistry};
use factory_policy::ToolPolicy;

const MAX_INPUT_BYTES: usize = 1_048_576;

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

struct Bridge {
    dir: tempfile::TempDir,
    token: String,
}

impl Bridge {
    fn root(&self) -> &Path {
        self.dir.path()
    }
}

fn bridge(tools: &[&str]) -> Bridge {
    let dir = tempfile::tempdir().unwrap();
    factory_core::Factory::init(dir.path()).unwrap();
    let mut config = Config::load(dir.path()).unwrap();
    config.policies.role_mut("worker").tools = Some(ToolPolicy {
        allow: tools.iter().map(|name| format!("fake.{name}")).collect(),
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
            timeout_ms: 5_000,
            discovery_timeout_ms: 5_000,
            max_response_bytes: 64_000,
            max_concurrency: 2,
            classifications: BTreeMap::new(),
        },
    );
    config.write_atomic(dir.path()).unwrap();
    let registry = ToolRegistry::new(dir.path(), config.mcp.clone());
    let discovered = registry.discover_server_blocking("fake").unwrap();
    let allowed: Vec<_> = discovered
        .into_iter()
        .filter(|tool| tools.contains(&tool.name.as_str()))
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
    Bridge {
        dir,
        token: grant.token,
    }
}

fn factory_command(root: &Path, token: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_factory"));
    command
        .current_dir(root)
        .env("FACTORY_MCP_ROOT", root)
        .env("FACTORY_MCP_GRANT", token);
    command
}

fn call_input_json(bridge: &Bridge, tool: &str, input_json: &str) -> Output {
    factory_command(bridge.root(), &bridge.token)
        .args(["mcp", "call", tool, "--input-json", input_json])
        .output()
        .unwrap()
}

fn call_input_stdin(root: &Path, token: &str, tool: &str, payload: &[u8]) -> Output {
    let mut child = factory_command(root, token)
        .args(["mcp", "call", tool, "--input-stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        let _ = stdin.write_all(payload);
    }
    drop(child.stdin.take());
    child.wait_with_output().unwrap()
}

fn invocation_count(bridge: &Bridge) -> usize {
    let db = FactoryDb::open(&bridge.root().join(".factory").join("db.sqlite3")).unwrap();
    db.list_tool_invocations().unwrap().len()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
fn scoped_cli_bridge_invokes_only_granted_tools() {
    let bridge = bridge(&["search"]);
    let output = call_input_json(&bridge, "fake.search", r#"{"query":"rust"}"#);
    assert!(output.status.success(), "{}", stderr_of(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("UNTRUSTED EXTERNAL TOOL DATA"));
    assert!(stdout.contains("fake.search"));
    assert!(!stdout.contains(&bridge.token));

    let denied = call_input_json(&bridge, "fake.erase", "{}");
    assert!(!denied.status.success());
    assert!(!stderr_of(&denied).contains(&bridge.token));
}

#[test]
fn stdin_transport_invokes_granted_tools_with_untrusted_output() {
    let bridge = bridge(&["search"]);
    let output = call_input_stdin(
        bridge.root(),
        &bridge.token,
        "fake.search",
        br#"{"query":"rust"}"#,
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("UNTRUSTED EXTERNAL TOOL DATA"));
    assert!(stdout.contains("END UNTRUSTED EXTERNAL TOOL DATA"));
    assert!(stdout.contains("fake.search"));
    assert!(stdout.contains("rust"));
    assert!(!stdout.contains(&bridge.token));
}

#[test]
fn stdin_transport_accepts_nested_unicode_and_escaped_json() {
    let bridge = bridge(&["measure"]);
    let payload = serde_json::json!({
        "query": format!("{}{}", "é".repeat(1_234), "→".repeat(567)),
        "nested": {"list": [1, {"deep": null}], "flag": true},
    })
    .to_string();
    let crlf = format!("\r\n{payload}\r\n");
    let output = call_input_stdin(
        bridge.root(),
        &bridge.token,
        "fake.measure",
        crlf.as_bytes(),
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1801"), "query characters: {stdout}");
    assert!(stdout.contains("4169"), "query UTF-8 bytes: {stdout}");
    assert!(stdout.contains("argument_keys"));

    let escaped = format!(
        "{{\"query\":\"{}{}{}\"}}",
        "\\\"".repeat(100),
        "\\u00e9".repeat(1_234),
        "\\n".repeat(10),
    );
    let output = call_input_stdin(
        bridge.root(),
        &bridge.token,
        "fake.measure",
        escaped.as_bytes(),
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1344"), "query characters: {stdout}");
    assert!(stdout.contains("2578"), "query UTF-8 bytes: {stdout}");
}

#[test]
fn stdin_transport_carries_payloads_beyond_command_line_capacity() {
    let bridge = bridge(&["search"]);
    let sentinel = "|end-of-payload-sentinel|";
    let payload = format!(r#"{{"query":"{}{}"}}"#, "A".repeat(60_000), sentinel);
    let output = call_input_stdin(
        bridge.root(),
        &bridge.token,
        "fake.search",
        payload.as_bytes(),
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(String::from_utf8_lossy(&output.stdout).contains(sentinel));
}

#[test]
fn stdin_transport_enforces_the_input_byte_limit_boundary() {
    let bridge = bridge(&["measure"]);
    let just_below = format!(r#"{{"query":"{}"}}"#, "x".repeat(MAX_INPUT_BYTES - 100));
    let output = call_input_stdin(
        bridge.root(),
        &bridge.token,
        "fake.measure",
        just_below.as_bytes(),
    );
    assert!(output.status.success(), "{}", stderr_of(&output));

    let at_limit = format!(r#"{{"query":"{}"}}"#, "x".repeat(MAX_INPUT_BYTES - 12));
    assert_eq!(at_limit.len(), MAX_INPUT_BYTES);
    let output = call_input_stdin(
        bridge.root(),
        &bridge.token,
        "fake.measure",
        at_limit.as_bytes(),
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(String::from_utf8_lossy(&output.stdout).contains("1048564"));

    let one_over = format!(r#"{{"query":"{}"}}"#, "x".repeat(MAX_INPUT_BYTES - 11));
    assert_eq!(one_over.len(), MAX_INPUT_BYTES + 1);
    let output = call_input_stdin(
        bridge.root(),
        &bridge.token,
        "fake.measure",
        one_over.as_bytes(),
    );
    assert!(!output.status.success());
    let stderr = stderr_of(&output);
    assert!(stderr.contains("limit"));
    assert!(!stderr.contains(&bridge.token));
    assert_eq!(invocation_count(&bridge), 2);
}

#[test]
fn stdin_reader_stops_consuming_an_unbounded_producer() {
    let bridge = bridge(&["measure"]);
    let payload = format!(r#"{{"query":"{}"}}"#, "x".repeat(3 * MAX_INPUT_BYTES));
    let output = call_input_stdin(
        bridge.root(),
        &bridge.token,
        "fake.measure",
        payload.as_bytes(),
    );
    assert!(!output.status.success());
    let stderr = stderr_of(&output);
    assert!(stderr.contains("limit"));
    assert!(!stderr.contains(&bridge.token));
    assert_eq!(invocation_count(&bridge), 0);
}

#[test]
fn stdin_transport_rejects_malformed_and_misshapen_input() {
    let bridge = bridge(&["measure"]);
    let cases: &[(&[u8], &str)] = &[
        (b"", "empty"),
        (b"   \r\n", "empty"),
        (b"{\"query\": ", "JSON"),
        (b"[1, 2, 3]", "object"),
        (b"\"text\"", "object"),
        (b"42", "object"),
        (b"null", "object"),
        (b"true", "object"),
        (b"{} trailing", "JSON"),
        (b"{\"a\":\xff}", "UTF-8"),
    ];
    for (payload, expected) in cases {
        let output = call_input_stdin(bridge.root(), &bridge.token, "fake.measure", payload);
        assert!(
            !output.status.success(),
            "payload {payload:?} should have been rejected"
        );
        let stderr = stderr_of(&output);
        assert!(
            stderr.contains(expected),
            "expected '{expected}' in: {stderr}"
        );
        assert!(!stderr.contains(&bridge.token));
    }
    assert_eq!(invocation_count(&bridge), 0);
}

#[test]
fn mcp_call_input_flags_are_mutually_exclusive_and_required() {
    let bridge = bridge(&["search"]);
    let both = factory_command(bridge.root(), &bridge.token)
        .args([
            "mcp",
            "call",
            "fake.search",
            "--input-json",
            "{}",
            "--input-stdin",
        ])
        .output()
        .unwrap();
    assert!(!both.status.success());
    assert!(stderr_of(&both).contains("cannot be used with"));

    let neither = factory_command(bridge.root(), &bridge.token)
        .args(["mcp", "call", "fake.search"])
        .output()
        .unwrap();
    assert!(!neither.status.success());
    let stderr = stderr_of(&neither);
    assert!(stderr.contains("exactly one input source"));
    assert!(!stderr.contains(&bridge.token));
}

#[test]
fn stdin_transport_preserves_grant_and_policy_enforcement() {
    let bridge = bridge(&["search"]);
    let denied = call_input_stdin(
        bridge.root(),
        &bridge.token,
        "fake.erase",
        br#"{"query":"x"}"#,
    );
    assert!(!denied.status.success());
    assert!(!stderr_of(&denied).contains(&bridge.token));

    let invalid = call_input_stdin(
        bridge.root(),
        "not-a-real-grant-token",
        "fake.search",
        br#"{"query":"x"}"#,
    );
    assert!(!invalid.status.success());
    let stderr = stderr_of(&invalid);
    assert!(stderr.contains("grant"));
    assert!(!stderr.contains(&bridge.token));
}
