use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use factory_db::FactoryDb;
use factory_mcp::{GrantContext, McpFailureKind, McpServerConfig, McpTransport, ToolRegistry};
use factory_policy::{PoliciesConfig, ToolPolicy};
use factory_types::ToolClassification;

fn python() -> &'static str {
    if cfg!(windows) {
        "python"
    } else {
        "python3"
    }
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("fake_mcp_server.py")
}

fn project() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".factory")).unwrap();
    FactoryDb::open(&root.path().join(".factory").join("db.sqlite3")).unwrap();
    root
}

fn config(timeout_ms: u64, max_response_bytes: usize) -> McpServerConfig {
    McpServerConfig {
        transport: McpTransport::Stdio,
        command: Some(python().into()),
        args: vec![fixture().to_string_lossy().to_string()],
        cwd: None,
        url: None,
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        timeout_ms,
        discovery_timeout_ms: 2_000,
        max_response_bytes,
        max_concurrency: 2,
        classifications: BTreeMap::new(),
    }
}

fn policy(patterns: &[&str]) -> factory_policy::EffectivePolicy {
    let mut policies = PoliciesConfig::default();
    policies.role_mut("worker").tools = Some(ToolPolicy {
        allow: patterns.iter().map(|value| (*value).to_string()).collect(),
        deny: Vec::new(),
    });
    policies.effective("worker", "codex")
}

fn registry(root: &Path, config: McpServerConfig) -> ToolRegistry {
    ToolRegistry::new(root, BTreeMap::from([("fake".into(), config)]))
}

fn context() -> GrantContext {
    GrantContext {
        run_id: None,
        task_id: None,
        attempt_id: None,
        session_id: None,
        agent: "codex".into(),
        role: "worker".into(),
        operation: Some("implement".into()),
    }
}

#[tokio::test]
async fn stdio_discovery_parses_schemas_and_annotations() {
    let root = project();
    let registry = registry(root.path(), config(1_000, 512_000));
    let tools = registry.discover_server("fake").await.unwrap();
    assert_eq!(tools.len(), 11);
    assert_eq!(
        tools
            .iter()
            .find(|tool| tool.name == "search")
            .unwrap()
            .classification,
        ToolClassification::Read
    );
    assert_eq!(
        tools
            .iter()
            .find(|tool| tool.name == "mutate")
            .unwrap()
            .classification,
        ToolClassification::Write
    );
    assert_eq!(
        tools
            .iter()
            .find(|tool| tool.name == "erase")
            .unwrap()
            .classification,
        ToolClassification::Destructive
    );
    assert_eq!(
        tools
            .iter()
            .find(|tool| tool.name == "ambiguous")
            .unwrap()
            .classification,
        ToolClassification::Unknown
    );
    assert_eq!(tools[0].input_schema["type"], "object");
}

#[tokio::test]
async fn output_is_bounded_marked_untrusted_and_secret_redacted() {
    let root = project();
    let secret = "fixture-secret-never-log";
    std::env::set_var("FACTORY_TEST_MCP_SECRET", secret);
    let mut server = config(1_000, 512_000);
    server
        .env
        .insert("FAKE_SECRET".into(), "${FACTORY_TEST_MCP_SECRET}".into());
    let registry = registry(root.path(), server);
    let tools = registry.discover_server("fake").await.unwrap();
    let allowed: Vec<_> = tools
        .into_iter()
        .filter(|tool| matches!(tool.name.as_str(), "injection" | "secret"))
        .collect();
    let grant = registry.create_grant(context(), &allowed).unwrap().unwrap();
    let effective = policy(&["fake.injection", "fake.secret"]);
    let injection = registry
        .execute_grant(
            &grant.token,
            "fake.injection",
            serde_json::json!({"query": "x"}),
            &effective,
        )
        .await
        .unwrap();
    assert!(injection
        .rendered
        .starts_with("UNTRUSTED EXTERNAL TOOL DATA"));
    assert!(injection.rendered.contains("IGNORE FACTORY POLICY"));
    assert!(injection
        .rendered
        .contains("END UNTRUSTED EXTERNAL TOOL DATA"));
    let secret_output = registry
        .execute_grant(
            &grant.token,
            "fake.secret",
            serde_json::json!({}),
            &effective,
        )
        .await
        .unwrap();
    assert!(!secret_output.rendered.contains(secret));
    let audit = FactoryDb::open(&root.path().join(".factory/db.sqlite3"))
        .unwrap()
        .list_tool_invocations()
        .unwrap();
    assert!(audit.iter().all(|row| !row.input_summary.contains(secret)));
    assert!(audit.iter().all(|row| !row.input_summary.contains("query")));
    std::env::remove_var("FACTORY_TEST_MCP_SECRET");
}

#[tokio::test]
async fn rejects_bad_input_timeout_crash_malformed_and_oversized_responses() {
    let root = project();
    // Leave enough scheduling headroom for the crash fixture to exit and close
    // its transport before Factory's deadline, even while the full workspace
    // suite is running. The slow fixture still exceeds this bound.
    let registry = registry(root.path(), config(500, 32_000));
    let tools = registry.discover_server("fake").await.unwrap();
    let allowed: Vec<_> = tools
        .into_iter()
        .filter(|tool| {
            matches!(
                tool.name.as_str(),
                "search" | "slow" | "crash" | "malformed" | "oversized"
            )
        })
        .collect();
    let grant = registry.create_grant(context(), &allowed).unwrap().unwrap();
    let effective = policy(&[
        "fake.search",
        "fake.slow",
        "fake.crash",
        "fake.malformed",
        "fake.oversized",
    ]);
    let cases = [
        (
            "fake.search",
            serde_json::json!({"query": 9}),
            McpFailureKind::InvalidSchema,
        ),
        ("fake.slow", serde_json::json!({}), McpFailureKind::Timeout),
        (
            "fake.crash",
            serde_json::json!({}),
            McpFailureKind::ProtocolError,
        ),
        (
            "fake.malformed",
            serde_json::json!({}),
            McpFailureKind::Timeout,
        ),
        (
            "fake.oversized",
            serde_json::json!({}),
            McpFailureKind::ResponseTooLarge,
        ),
    ];
    for (name, input, expected) in cases {
        let error = registry
            .execute_grant(&grant.token, name, input, &effective)
            .await
            .unwrap_err();
        assert_eq!(error.kind, expected, "unexpected error for {name}: {error}");
    }
}

#[tokio::test]
async fn unauthorized_call_is_audited_without_contacting_server() {
    let root = project();
    let mut server = config(100, 32_000);
    server.command = Some("definitely-not-a-real-mcp-server".into());
    let registry = registry(root.path(), server);
    let metadata = factory_types::McpToolMetadata {
        server: "fake".into(),
        name: "search".into(),
        qualified_name: "fake.search".into(),
        description: "search".into(),
        input_schema: serde_json::json!({"type": "object"}),
        annotations: None,
        classification: ToolClassification::Read,
        discovered_at: chrono::Utc::now().to_rfc3339(),
    };
    let grant = registry
        .create_grant(context(), &[metadata])
        .unwrap()
        .unwrap();
    let error = registry
        .execute_grant(
            &grant.token,
            "fake.not_granted",
            serde_json::json!({}),
            &policy(&["fake.*"]),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, McpFailureKind::PermissionDenied);
    let audit = FactoryDb::open(&root.path().join(".factory/db.sqlite3"))
        .unwrap()
        .list_tool_invocations()
        .unwrap();
    assert_eq!(audit.last().unwrap().status, "permission_denied");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parallel_calls_are_independent_and_cancellation_releases_the_server_slot() {
    let root = project();
    let mut server = config(2_000, 512_000);
    server.max_concurrency = 2;
    let registry = registry(root.path(), server);
    let tools = registry.discover_server("fake").await.unwrap();
    let allowed: Vec<_> = tools
        .into_iter()
        .filter(|tool| tool.name == "slow")
        .collect();
    let grant = registry.create_grant(context(), &allowed).unwrap().unwrap();
    let effective = policy(&["fake.slow"]);
    let first_registry = registry.clone();
    let first_token = grant.token.clone();
    let first_policy = effective.clone();
    let first = tokio::spawn(async move {
        first_registry
            .execute_grant(
                &first_token,
                "fake.slow",
                serde_json::json!({}),
                &first_policy,
            )
            .await
    });
    let second_registry = registry.clone();
    let second_token = grant.token.clone();
    let second_policy = effective.clone();
    let second = tokio::spawn(async move {
        second_registry
            .execute_grant(
                &second_token,
                "fake.slow",
                serde_json::json!({}),
                &second_policy,
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    first.abort();
    let _ = first.await;
    assert!(second.await.unwrap().is_ok());
    let third = registry
        .execute_grant(&grant.token, "fake.slow", serde_json::json!({}), &effective)
        .await;
    assert!(
        third.is_ok(),
        "cancelled calls must release their lease: {third:?}"
    );
    let audit = FactoryDb::open(&root.path().join(".factory/db.sqlite3"))
        .unwrap()
        .list_tool_invocations()
        .unwrap();
    assert!(audit.iter().any(|row| row.status == "cancelled"));
}
