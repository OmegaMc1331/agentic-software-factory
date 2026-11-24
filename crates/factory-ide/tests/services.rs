use factory_files::{FileRef, FileService, Limits};
use factory_ide::{lsp::Document, IdeManager, IdeStore, ToolProfile};
use factory_runtime::RuntimeBudget;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Fixture {
    root: PathBuf,
    ide: IdeManager,
    root_id: String,
    _temp: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        Self::with_project("p_00000000000000000000000000000001")
    }
    fn with_project(project: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        std::fs::create_dir_all(root.join(".factory")).unwrap();
        assert!(std::process::Command::new("git")
            .args(["init", "--quiet"])
            .arg(&root)
            .status()
            .unwrap()
            .success());
        std::fs::write(root.join("main.ts"),"export function add(a: number, b: number) { return a + b; }\nconst value: string = add(1, 2);\n").unwrap();
        std::fs::write(root.join("main.py"),"def add(a: int, b: int) -> int:\n    return a + b\n\nvalue: str = add(1, 2)\nprint(value)\n").unwrap();
        std::fs::write(
            root.join("debug.py"),
            "value = 40\nvalue += 2\nprint(value)\n",
        )
        .unwrap();
        std::fs::write(
            root.join("debug.js"),
            "let value = 40;\nvalue += 2;\nconsole.log(value);\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='asf_p06_test'\nversion='0.1.0'\nedition='2021'\n",
        )
        .unwrap();
        std::fs::write(root.join("src/main.rs"),"fn add(a: i32, b: i32) -> i32 { a + b }\nfn main() {\n    let value = add(40, 2);\n    println!(\"{value}\");\n}\n").unwrap();
        let files = Arc::new(FileService::new(project, &root, Limits::default()).unwrap());
        let root_id = files.roots().unwrap()[0].id.clone();
        let store = Arc::new(IdeStore::open(&temp.path().join("ide.sqlite3")).unwrap());
        let ide = IdeManager::new(
            files,
            store,
            Arc::new(RuntimeBudget::new(8, 8)),
            Arc::new(RuntimeBudget::new(8, 8)),
        );
        Self {
            _temp: temp,
            root,
            ide,
            root_id,
        }
    }
    fn file(&self, path: &str) -> FileRef {
        FileRef {
            project_id: self.ide.files.project_id.clone(),
            root_id: self.root_id.clone(),
            path: path.into(),
        }
    }
    fn profile(&self, id: &str) -> ToolProfile {
        let path = std::env::var("ASF_IDE_TEST_TOOLS")
            .expect("set ASF_IDE_TEST_TOOLS to the verified tool profile JSON file");
        let source = std::fs::read_to_string(path).unwrap();
        let values: Vec<ToolProfile> =
            serde_json::from_str(source.trim_start_matches('\u{feff}')).unwrap();
        let profile = values
            .into_iter()
            .find(|p| p.id == id)
            .expect("required tool profile is absent");
        self.ide.approve_profile(profile, true).unwrap()
    }
}

#[test]
fn profiles_extensions_and_restart_are_project_scoped() {
    let f = Fixture::new();
    let manifest:factory_ide::extensions::ExtensionManifest=serde_json::from_value(json!({"id":"asf.test","name":"Test","version":"1.0.0","publisher":"asf","description":"Original test contribution","entrypoint":"contributions","extension_api_version":1,"capabilities":["editor"],"permissions":["editor"],"supported_platforms":["any"],"contributions":{"formatters":[{"id":"trim","title":"Trim trailing spaces","operation":"trim-trailing-whitespace","languages":["python"]}]}})).unwrap();
    assert!(f.ide.extensions.install(manifest.clone(), false).is_err());
    assert!(
        !f.ide
            .extensions
            .install(manifest.clone(), true)
            .unwrap()
            .enabled
    );
    assert!(f.ide.extensions.install(manifest.clone(), true).is_err());
    assert_eq!(
        f.ide.extensions.enable("asf.test", true).unwrap().status,
        "active"
    );
    assert_eq!(f.ide.extensions.list().unwrap().len(), 1);
    let stored = Arc::new(IdeStore::open(&f._temp.path().join("ide.sqlite3")).unwrap());
    let other = factory_ide::extensions::ExtensionManager::new(
        "p_00000000000000000000000000000002",
        stored.clone(),
    );
    assert!(other.list().unwrap().is_empty());
    let restored = factory_ide::extensions::ExtensionManager::new(&f.ide.files.project_id, stored);
    assert!(restored.list().unwrap()[0].enabled);
    f.ide.extensions.enable("asf.test", false).unwrap();
    f.ide.extensions.uninstall("asf.test").unwrap();
    for (field, value) in [
        ("entrypoint", json!("../../escape.js")),
        ("extension_api_version", json!(99)),
        ("permissions", json!(["filesystem"])),
        ("supported_platforms", json!(["unrecognized"])),
    ] {
        let mut invalid = serde_json::to_value(&manifest).unwrap();
        invalid[field] = value;
        assert!(
            serde_json::from_value::<factory_ide::extensions::ExtensionManifest>(invalid)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    assert!(
        serde_json::from_value::<factory_ide::extensions::ExtensionManifest>(
            json!({"id":"x","scripts":{"install":"bad"}})
        )
        .is_err()
    );
}

#[cfg(windows)]
#[tokio::test]
async fn native_development_terminal_io_resize_reconnect_and_cleanup() {
    let f = Fixture::new();
    let profile:ToolProfile=serde_json::from_value(json!({"id":"cmd","name":"Command Prompt","kind":"shell","executable":PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe"),"arguments":["/d","/q"],"languages":[],"enabled":true})).unwrap();
    f.ide.approve_profile(profile, true).unwrap();
    assert!(f
        .ide
        .start_terminal(&f.root_id, "cmd", false, 80, 24)
        .is_err());
    let info = f
        .ide
        .start_terminal(&f.root_id, "cmd", true, 80, 24)
        .unwrap();
    let terminal = f.ide.terminal(&info.id).unwrap();
    terminal.process.resize(111, 37).unwrap();
    let mut subscriber = terminal.process.subscribe().unwrap();
    terminal
        .process
        .write(b"echo __ASF_P06_NATIVE__\r\n")
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(10), async {
        let mut bytes = Vec::new();
        while let Some(chunk) = subscriber.receiver.recv().await {
            bytes.extend_from_slice(&chunk);
            if chunk.windows(4).any(|window| window == b"\x1b[6n") {
                terminal.process.write(b"\x1b[1;1R").unwrap();
            }
            if String::from_utf8_lossy(&bytes).contains("__ASF_P06_NATIVE__") {
                break;
            }
        }
        bytes
    })
    .await
    .expect("native ConPTY did not return the controlled marker");
    assert!(String::from_utf8_lossy(&output).contains("__ASF_P06_NATIVE__"));
    drop(subscriber);
    let reconnected = terminal.process.subscribe().unwrap();
    assert!(!reconnected.snapshot.is_empty());
    assert!(terminal.process.write(&vec![b'x'; 65537]).is_err());
    f.ide.stop_terminal(&info.id).unwrap();
    let start = Instant::now();
    while terminal.process.exit_code().is_none() {
        assert!(start.elapsed() < Duration::from_secs(10));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[test]
fn extension_activation_and_integrity_failure_are_isolated() {
    let f = Fixture::new();
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../extensions/p06-editor/extension.json");
    let manifest: factory_ide::extensions::ExtensionManifest =
        serde_json::from_slice(&std::fs::read(source).unwrap()).unwrap();
    f.ide.extensions.install(manifest.clone(), true).unwrap();
    f.ide.extensions.enable(&manifest.id, true).unwrap();
    let mut dependent = manifest.clone();
    dependent.id = "asf.dependent".into();
    dependent
        .dependencies
        .insert(manifest.id.clone(), manifest.version.clone());
    f.ide.extensions.install(dependent.clone(), true).unwrap();
    f.ide.extensions.enable(&manifest.id, false).unwrap();
    assert!(f.ide.extensions.enable(&dependent.id, true).is_err());
    assert_eq!(
        f.ide
            .extensions
            .list()
            .unwrap()
            .iter()
            .find(|item| item.manifest.id == dependent.id)
            .unwrap()
            .status,
        "failed"
    );
    assert!(f.ide.files.read(&f.file("main.ts")).is_ok());
    f.ide.extensions.enable(&manifest.id, true).unwrap();
    f.ide.extensions.enable(&dependent.id, true).unwrap();
    assert!(f.ide.extensions.uninstall(&manifest.id).is_err());
    f.ide.extensions.enable(&dependent.id, false).unwrap();
    f.ide.extensions.uninstall(&dependent.id).unwrap();
    let store = IdeStore::open(&f._temp.path().join("ide.sqlite3")).unwrap();
    let mut corrupted = f.ide.extensions.list().unwrap()[0].clone();
    corrupted.sha256 = "incorrect digest".into();
    store
        .put(
            &f.ide.files.project_id,
            "extensions",
            &manifest.id,
            &corrupted,
        )
        .unwrap();
    assert!(f.ide.extensions.enable(&manifest.id, true).is_err());
    assert_eq!(f.ide.extensions.list().unwrap()[0].status, "failed");
    assert!(f.ide.files.read(&f.file("main.ts")).is_ok());
    f.ide.extensions.uninstall(&manifest.id).unwrap();
}

#[test]
fn root_uri_and_debug_configuration_refuse_escape_and_attach() {
    let f = Fixture::new();
    for uri in [
        "file:///outside.py",
        "https://example.com/main.py",
        "file:///C:/outside.py",
        "file:///C:/main.py?access=1",
    ] {
        assert!(factory_ide::lsp::reference(&f.ide.files, &f.root_id, uri).is_err());
    }
    let file = f.file("main.py");
    let valid = factory_ide::lsp::file_uri(&f.ide.files, &file).unwrap();
    assert_eq!(
        factory_ide::lsp::reference(&f.ide.files, &f.root_id, &valid).unwrap(),
        file
    );
    let mut config = factory_ide::dap::LaunchConfiguration {
        id: "test".into(),
        name: "Test".into(),
        profile_id: "adapter".into(),
        root_id: f.root_id.clone(),
        program: f.file("debug.py"),
        arguments: json!({}),
    };
    assert!(config.validate(&f.ide.files).is_ok());
    for field in [
        "pid",
        "processId",
        "connect",
        "listen",
        "runtimeExecutable",
        "envFile",
        "cwd",
        "console",
    ] {
        config.arguments = json!({field:"untrusted"});
        assert!(config.validate(&f.ide.files).is_err());
    }
    config.arguments = json!({});
    config.program.project_id = "p_00000000000000000000000000000002".into();
    assert!(config.validate(&f.ide.files).is_err());
    let mut file = f.file("../main.py");
    assert!(f.ide.files.launch_path(&file).is_err());
    file.path = ".factory/db.sqlite3".into();
    assert!(f.ide.files.launch_path(&file).is_err());
}

#[cfg(windows)]
#[tokio::test]
async fn native_powershell_starts_with_filtered_os_environment() {
    let f = Fixture::new();
    let profile:ToolProfile=serde_json::from_value(json!({"id":"powershell","name":"PowerShell","kind":"shell","executable":PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/WindowsPowerShell/v1.0/powershell.exe"),"arguments":["-NoLogo","-NoProfile"],"languages":[],"enabled":true})).unwrap();
    f.ide.approve_profile(profile, true).unwrap();
    let info = f
        .ide
        .start_terminal(&f.root_id, "powershell", true, 100, 28)
        .unwrap();
    let terminal = f.ide.terminal(&info.id).unwrap();
    let mut subscriber = terminal.process.subscribe().unwrap();
    terminal.process.write(b"Set-Content -LiteralPath terminal-proof.txt -Value '__ASF_P06_NATIVE__'; Get-Content -LiteralPath terminal-proof.txt\r\n").unwrap();
    let mut collected = String::from_utf8_lossy(&subscriber.snapshot).to_string();
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(chunk) = subscriber.receiver.recv().await {
            collected.push_str(&String::from_utf8_lossy(&chunk));
            if chunk.windows(4).any(|window| window == b"\x1b[6n") {
                terminal.process.write(b"\x1b[1;1R").unwrap();
            }
            if f.root.join("terminal-proof.txt").is_file() {
                return true;
            }
        }
        false
    })
    .await;
    assert!(
        matches!(result, Ok(true)),
        "PowerShell did not execute in its authorized root: {collected}"
    );
    assert_eq!(
        std::fs::read_to_string(f.root.join("terminal-proof.txt"))
            .unwrap()
            .trim(),
        "__ASF_P06_NATIVE__"
    );
    f.ide.stop_terminal(&info.id).unwrap();
}

fn lsp_test(profile: &str, path: &str, position: Value) {
    let f = Fixture::new();
    let p = f.profile(profile);
    let start = Instant::now();
    let info = f
        .ide
        .start_lsp(&f.root_id, &p.id, "test-window", true)
        .unwrap();
    eprintln!("{profile} startup_ms={}", start.elapsed().as_millis());
    let server = f.ide.lsp(&info.id).unwrap();
    let file = f.file(path);
    let disk = f.ide.files.read(&file).unwrap();
    let document = Document {
        file: file.clone(),
        base_revision: disk.revision.clone(),
        version: 1,
        text: disk.content.unwrap(),
        language_id: p.languages[0].clone(),
    };
    let uri = server.sync(document.clone(), false).unwrap()["uri"]
        .as_str()
        .unwrap()
        .to_owned();
    let deadline = Instant::now();
    let hover = loop {
        let hover = server.request(
            "textDocument/hover",
            json!({"textDocument":{"uri":uri},"position":position}),
            false,
        );
        if hover.as_ref().is_ok_and(|v| !v.is_null()) {
            break hover.unwrap();
        }
        assert!(
            deadline.elapsed() < Duration::from_secs(60),
            "semantic hover unavailable: {hover:?}"
        );
        std::thread::sleep(Duration::from_millis(250));
    };
    assert!(hover.get("contents").is_some());
    if profile != "rust" {
        let start = Instant::now();
        loop {
            if server.events(0).events.iter().any(|event| {
                event.message["method"] == "textDocument/publishDiagnostics"
                    && event.message["params"]["diagnostics"]
                        .as_array()
                        .is_some_and(|items| !items.is_empty())
            }) {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "real diagnostics not received: {:?}",
                server.events(0)
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    let mut changed = document.clone();
    changed.version = 2;
    if profile == "typescript" {
        changed.text="export function add(a: number, b: number) { return a + b; }\nconst value: number = add(1, 2);\n".into();
    } else if profile == "python" {
        changed.text = changed.text.replace("value: str", "value: int");
    } else {
        changed.text = changed.text.replace("let value", "let value: i32");
    }
    server.sync(changed.clone(), false).unwrap();
    assert!(server.sync(document.clone(), false).is_err());
    let symbol_started = Instant::now();
    let mut symbol_retries = 0;
    let symbol = loop {
        let symbol = server
            .request(
                "textDocument/documentSymbol",
                json!({"textDocument":{"uri":uri}}),
                false,
            )
            .unwrap();
        if !symbol.is_null() {
            break symbol;
        }
        symbol_retries += 1;
        assert!(
            symbol_started.elapsed() < Duration::from_secs(60),
            "document symbols unavailable after edit: {symbol:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    eprintln!(
        "{profile} symbol_retries={symbol_retries} symbols_ms={}",
        symbol_started.elapsed().as_millis()
    );
    assert!(!symbol.is_null());
    assert!(symbol.to_string().contains("\"name\":\"add\""));
    let definition = server
        .request(
            "textDocument/definition",
            json!({"textDocument":{"uri":uri},"position":position}),
            false,
        )
        .unwrap();
    eprintln!("{profile} definition={definition}");
    assert!(server
        .request(
            "workspace/executeCommand",
            json!({"command":"arbitrary"}),
            true
        )
        .is_err());
    assert!(server.prepare_edit(&json!({"changes":{"file:///C:/outside.txt":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":"bad"}]}})).is_err());
    let rename=server.prepare_edit(&json!({"documentChanges":[{"textDocument":{"uri":uri,"version":2},"edits":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":" "}]}]})).unwrap();
    assert_eq!(rename[0].before, changed.text);
    if profile == "typescript" {
        let formatting = server
            .request(
                "textDocument/formatting",
                json!({"textDocument":{"uri":uri},"options":{"tabSize":2,"insertSpaces":true}}),
                true,
            )
            .unwrap();
        let edit = json!({"changes":{uri.clone():formatting}});
        assert_eq!(
            server.prepare_verified_edit(&edit).unwrap()[0].document_version,
            Some(2)
        );
        changed.version = 3;
        changed.text.push('\n');
        server.sync(changed.clone(), false).unwrap();
        assert!(server.prepare_verified_edit(&edit).is_err());
        changed.version = 4;
        changed.text="export function add(a: number, b: number) { return a + b; }\nconst emoji = '😀'; const value = add(1, 2);\n".into();
        server.sync(changed.clone(), false).unwrap();
        let line = changed.text.lines().nth(1).unwrap();
        let character = line[..line.find("add(").unwrap() + 1]
            .encode_utf16()
            .count();
        let hover = server
            .request(
                "textDocument/hover",
                json!({"textDocument":{"uri":uri},"position":{"line":1,"character":character}}),
                false,
            )
            .unwrap();
        assert!(hover.to_string().contains("add"));
        let definitions = server
            .request(
                "textDocument/definition",
                json!({"textDocument":{"uri":uri},"position":{"line":1,"character":character}}),
                false,
            )
            .unwrap();
        assert!(!definitions.as_array().unwrap().is_empty());
    }
    std::fs::write(f.root.join(path), "external change\n").unwrap();
    assert!(server.prepare_edit(&json!({"changes":{uri.clone():[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":"x"}]}})).is_err());
    let observed = server.info().documents;
    let mut invalid_save = changed.clone();
    invalid_save.version += 1;
    invalid_save.text.push_str("not saved to disk");
    assert!(server.sync(invalid_save, true).is_err());
    assert_eq!(server.info().documents, observed);
    let pid = server.peer.pid;
    server.peer.kill();
    assert!(!server.peer.is_alive());
    let restarted = f
        .ide
        .start_lsp(&f.root_id, &p.id, "test-window", true)
        .unwrap();
    assert_ne!(restarted.pid, pid);
    let restored = f.ide.lsp(&restarted.id).unwrap();
    let disk = f.ide.files.read(&file).unwrap();
    changed.base_revision = disk.revision;
    restored.sync(changed, false).unwrap();
    f.ide.stop_lsp(&restarted.id).unwrap();
}

#[test]
#[ignore = "requires ASF_IDE_TEST_TOOLS with approved TypeScript language server"]
fn real_typescript_lsp() {
    lsp_test("typescript", "main.ts", json!({"line":0,"character":16}));
}
#[test]
#[ignore = "requires ASF_IDE_TEST_TOOLS with approved Pyright language server"]
fn real_python_lsp() {
    lsp_test("python", "main.py", json!({"line":0,"character":5}));
}
#[test]
#[ignore = "requires ASF_IDE_TEST_TOOLS with approved rust-analyzer and rust-src"]
fn real_rust_lsp() {
    lsp_test("rust", "src/main.rs", json!({"line":0,"character":4}));
}

fn debug_test(profile: &str, path: &str, line: u64) {
    let f = Fixture::new();
    let p = f.profile(profile);
    let program = if profile == "codelldb" {
        assert!(std::process::Command::new("cargo")
            .args(["build", "--quiet"])
            .env("CARGO_TARGET_DIR", f.root.join("target"))
            .env("CARGO_PROFILE_DEV_DEBUG", "2")
            .current_dir(&f.root)
            .status()
            .unwrap()
            .success());
        f.file(if cfg!(windows) {
            "target/debug/asf_p06_test.exe"
        } else {
            "target/debug/asf_p06_test"
        })
    } else {
        f.file(path)
    };
    let config = factory_ide::dap::LaunchConfiguration {
        id: "debug".into(),
        name: "Controlled fixture".into(),
        profile_id: p.id.clone(),
        root_id: f.root_id.clone(),
        program,
        arguments: json!({"stopOnEntry":false}),
    };
    f.ide.approve_configuration(config.clone(), true).unwrap();
    assert!(f.ide.start_debug("debug", json!([]), false).is_err());
    let start = Instant::now();
    let info = f
        .ide
        .start_debug(
            "debug",
            json!([{"file":f.file(path),"breakpoints":[{"line":line}]}]),
            true,
        )
        .unwrap();
    eprintln!("{profile} startup_ms={}", start.elapsed().as_millis());
    let session = f.ide.dap(&info.id).unwrap();
    let stopped = wait_event(&session, "stopped", 0);
    let threads = session.request("threads", json!({}), false).unwrap();
    assert!(!threads["threads"].as_array().unwrap().is_empty());
    let thread = stopped["body"]["threadId"].as_u64().unwrap();
    let stack = session
        .request("stackTrace", json!({"threadId":thread}), false)
        .unwrap();
    let frame = &stack["stackFrames"][0];
    assert!(frame["id"].is_number());
    assert!(!frame["asfFile"].is_null());
    let scopes = session
        .request("scopes", json!({"frameId":frame["id"]}), false)
        .unwrap();
    let variables = session
        .request(
            "variables",
            json!({"variablesReference":scopes["scopes"][0]["variablesReference"]}),
            false,
        )
        .unwrap();
    assert!(variables["variables"].is_array());
    let cursor = session.events(0).cursor;
    session
        .request("next", json!({"threadId":thread}), false)
        .unwrap();
    wait_event(&session, "stopped", cursor);
    let stepped_stack = session
        .request("stackTrace", json!({"threadId":thread}), false)
        .unwrap();
    let stepped_scopes = session
        .request(
            "scopes",
            json!({"frameId":stepped_stack["stackFrames"][0]["id"]}),
            false,
        )
        .unwrap();
    let mut observed_value = false;
    for scope in stepped_scopes["scopes"].as_array().unwrap().iter().take(3) {
        let values = session
            .request(
                "variables",
                json!({"variablesReference":scope["variablesReference"]}),
                false,
            )
            .unwrap();
        observed_value |= values["variables"]
            .as_array()
            .unwrap()
            .iter()
            .any(|variable| {
                variable["name"] == "value" && variable["value"].as_str() == Some("42")
            });
    }
    assert!(
        observed_value,
        "{profile} did not expose value 42 after stepping"
    );
    assert!(session
        .request("attach", json!({"processId":1}), true)
        .is_err());
    assert!(session
        .request(
            "evaluate",
            json!({"expression":"dangerous()","context":"repl"}),
            false
        )
        .is_err());
    f.ide.stop_debug(&info.id).unwrap();
    assert!(!session.peer.is_alive());
}

fn wait_event(session: &factory_ide::dap::DebugSession, name: &str, cursor: u64) -> Value {
    let started = Instant::now();
    loop {
        if let Some(event) = session
            .events(cursor)
            .events
            .into_iter()
            .find(|event| event.message["event"] == name)
        {
            return event.message;
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "debug event {name} timed out: {:?}",
            session.events(0)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "requires ASF_IDE_TEST_TOOLS with debugpy"]
fn real_python_dap() {
    debug_test("debugpy", "debug.py", 2);
}
#[test]
#[ignore = "requires ASF_IDE_TEST_TOOLS with js-debug"]
fn real_javascript_dap() {
    debug_test("js-debug", "debug.js", 2);
}
#[test]
#[ignore = "requires ASF_IDE_TEST_TOOLS with CodeLLDB and a Rust toolchain"]
fn real_rust_dap() {
    debug_test("codelldb", "src/main.rs", 3);
}

#[test]
#[ignore = "requires ASF_IDE_TEST_TOOLS TypeScript and debugpy; two real concurrent project contexts"]
fn real_multi_project_language_and_debug_isolation() {
    let a = Fixture::new();
    let b = Fixture::with_project("p_00000000000000000000000000000002");
    let source_b = "export const projectB: number = 'project B';\n";
    std::fs::write(b.root.join("main.ts"), source_b).unwrap();
    std::fs::write(
        b.root.join("debug.py"),
        "value = 140\nvalue += 2\nprint(value)\n",
    )
    .unwrap();
    let mut language = Vec::new();
    let mut debugging = Vec::new();
    for f in [&a, &b] {
        f.profile("typescript");
        f.profile("debugpy");
        let info = f
            .ide
            .start_lsp(&f.root_id, "typescript", "isolation", true)
            .unwrap();
        let server = f.ide.lsp(&info.id).unwrap();
        let snapshot = f.ide.files.read(&f.file("main.ts")).unwrap();
        let synced = server
            .sync(
                Document {
                    file: f.file("main.ts"),
                    base_revision: snapshot.revision,
                    version: 1,
                    text: snapshot.content.unwrap(),
                    language_id: "typescript".into(),
                },
                false,
            )
            .unwrap();
        language.push((info, server, synced["uri"].as_str().unwrap().to_owned()));
        let config = factory_ide::dap::LaunchConfiguration {
            id: "same-launch-id".into(),
            name: "Isolated Python".into(),
            profile_id: "debugpy".into(),
            root_id: f.root_id.clone(),
            program: f.file("debug.py"),
            arguments: json!({"stopOnEntry":false}),
        };
        f.ide.approve_configuration(config, true).unwrap();
        let info = f
            .ide
            .start_debug(
                "same-launch-id",
                json!([{"file":f.file("debug.py"),"breakpoints":[{"line":2}]}]),
                true,
            )
            .unwrap();
        let session = f.ide.dap(&info.id).unwrap();
        let stopped = wait_event(&session, "stopped", 0);
        debugging.push((info, session, stopped["body"]["threadId"].as_u64().unwrap()));
    }
    assert_ne!(language[0].0.pid, language[1].0.pid);
    assert!(a.ide.lsp(&language[1].0.id).is_err());
    assert!(b.ide.lsp(&language[0].0.id).is_err());
    assert!(language[0]
        .1
        .request(
            "textDocument/hover",
            json!({"textDocument":{"uri":language[1].2},"position":{"line":0,"character":13}}),
            false
        )
        .is_err());
    for (f, (_, server, uri)) in [(&a, &language[0]), (&b, &language[1])] {
        server
            .request(
                "textDocument/documentSymbol",
                json!({"textDocument":{"uri":uri}}),
                false,
            )
            .unwrap();
        let started = Instant::now();
        loop {
            let events = server.events(0);
            let diagnostics: Vec<_> = events
                .events
                .iter()
                .filter(|event| event.message["method"] == "textDocument/publishDiagnostics")
                .collect();
            assert!(diagnostics
                .iter()
                .all(|event| event.message["params"]["uri"] == *uri));
            if diagnostics.iter().any(|event| {
                event.message["params"]["diagnostics"]
                    .as_array()
                    .is_some_and(|items| !items.is_empty())
            }) {
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(30));
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(server.root_id, f.root_id);
    }
    assert_ne!(debugging[0].0.pid, debugging[1].0.pid);
    assert!(a.ide.dap(&debugging[1].0.id).is_err());
    assert!(b.ide.dap(&debugging[0].0.id).is_err());
    a.ide.stop_debug(&debugging[0].0.id).unwrap();
    assert!(debugging[1].1.peer.is_alive());
    let stack = debugging[1]
        .1
        .request("stackTrace", json!({"threadId":debugging[1].2}), false)
        .unwrap();
    let scopes = debugging[1]
        .1
        .request(
            "scopes",
            json!({"frameId":stack["stackFrames"][0]["id"]}),
            false,
        )
        .unwrap();
    let variables = debugging[1]
        .1
        .request(
            "variables",
            json!({"variablesReference":scopes["scopes"][0]["variablesReference"]}),
            false,
        )
        .unwrap();
    assert!(variables["variables"]
        .as_array()
        .unwrap()
        .iter()
        .any(|variable| variable["name"] == "value" && variable["value"] == "140"));
    b.ide.stop_debug(&debugging[1].0.id).unwrap();
    a.ide.stop_lsp(&language[0].0.id).unwrap();
    assert!(language[1].1.peer.is_alive());
    b.ide.stop_lsp(&language[1].0.id).unwrap();
}

#[test]
#[ignore = "requires ASF_IDE_TEST_TOOLS Python runtime; deterministic adversarial protocol fixture, not language qualification"]
fn controlled_protocol_cancellation_crash_bounds_and_server_requests() {
    let f = Fixture::new();
    let python = f.profile("debugpy").executable;
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/v2_p06/fixtures/protocol_peer.py")
        .canonicalize()
        .unwrap();
    for mode in [
        "oversized",
        "malformed",
        "flood",
        "server-edit",
        "crash",
        "hang",
    ] {
        let profile:ToolProfile=serde_json::from_value(json!({"id":"adversarial","name":"Deterministic protocol fixture","kind":"lsp","executable":python,"arguments":[script,mode],"languages":["text"],"enabled":true,"timeout_ms":2000})).unwrap();
        f.ide.approve_profile(profile, true).unwrap();
        let started = f
            .ide
            .start_lsp(&f.root_id, "adversarial", "test-fixture", true);
        if ["oversized", "malformed"].contains(&mode) {
            assert!(started.is_err());
            continue;
        }
        let info = started.unwrap();
        let server = f.ide.lsp(&info.id).unwrap();
        if mode == "hang" {
            let requested = server.clone();
            let handle = std::thread::spawn(move || {
                requested.request_token(
                    "workspace/symbol",
                    json!({"query":"slow"}),
                    false,
                    "cancel-me",
                )
            });
            let until = Instant::now();
            while server.cancel("cancel-me").is_err() {
                assert!(until.elapsed() < Duration::from_secs(2));
                std::thread::yield_now();
            }
            assert!(matches!(
                handle.join().unwrap(),
                Err(factory_ide::IdeError::Cancelled)
            ));
            let until = Instant::now();
            while !server
                .peer
                .events(0)
                .events
                .iter()
                .any(|event| event.message["method"] == "fixture/cancelled")
            {
                assert!(until.elapsed() < Duration::from_secs(2));
                std::thread::sleep(Duration::from_millis(5));
            }
        } else {
            let result = server.request("workspace/symbol", json!({"query":"test"}), false);
            if mode == "crash" {
                assert!(result.is_err());
            } else {
                result.unwrap();
            }
            if mode == "flood" {
                let page = server.peer.events(0);
                assert!(page.gap);
                assert!(page.events.len() <= 128);
            }
            if mode == "server-edit" {
                let until = Instant::now();
                loop {
                    if let Some(event) = server
                        .peer
                        .events(0)
                        .events
                        .iter()
                        .find(|event| event.message["method"] == "fixture/edit-refused")
                    {
                        assert_eq!(event.message["params"]["applied"], false);
                        break;
                    }
                    assert!(until.elapsed() < Duration::from_secs(2));
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
        f.ide.stop_lsp(&info.id).unwrap();
    }
}
