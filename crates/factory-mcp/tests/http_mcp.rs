use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use factory_mcp::{McpClient, McpServerConfig, McpTransport};
use factory_types::ToolClassification;

fn python() -> &'static str {
    if cfg!(windows) {
        "python"
    } else {
        "python3"
    }
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_http_mcp_server.py")
}

struct FakeHttp {
    child: Child,
    url: String,
}

impl FakeHttp {
    fn start() -> Self {
        let mut child = Command::new(python())
            .arg("-u")
            .arg(fixture())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut port = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut port)
            .unwrap();
        Self {
            child,
            url: format!("http://127.0.0.1:{}/mcp", port.trim()),
        }
    }
}

impl Drop for FakeHttp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test]
async fn streamable_http_discovers_and_calls_tools() {
    let server = FakeHttp::start();
    let config = McpServerConfig {
        transport: McpTransport::Http,
        command: None,
        args: Vec::new(),
        cwd: None,
        url: Some(server.url.clone()),
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        timeout_ms: 1_000,
        discovery_timeout_ms: 1_000,
        max_response_bytes: 32_000,
        max_concurrency: 2,
        classifications: BTreeMap::new(),
    };
    let client = McpClient::new(env!("CARGO_MANIFEST_DIR"));
    let tools = client.discover("docs", &config).await.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].qualified_name, "docs.search");
    assert_eq!(tools[0].classification, ToolClassification::Read);
    let output = client
        .call("docs", &config, "search", serde_json::json!({}))
        .await
        .unwrap();
    assert!(output.rendered.contains("http result"));
    assert!(output.rendered.starts_with("UNTRUSTED EXTERNAL TOOL DATA"));
}
