# P06 local qualification

No command in this folder pushes, deploys or publishes. Generated repositories, downloaded optional tools, binaries and logs belong under `.factory/v2/p06/`; these are not production fixtures.

## Tools and provenance

ASF itself requires neither Node nor Python to start. Real language/debug qualification requires approved optional tools. This run used Node 24.22.1, TypeScript language server 6.0.2 with TypeScript 5.7.3, Pyright 1.1.414, rust-analyzer 1.99.1 (58cf8f9c), debugpy 1.8.22, js-debug 1.140.0 and CodeLLDB 1.12.3. The P06 report records the actual measured versions and native platform.

`prepare.mjs` only builds a profile/provenance file from installed tools. It does not execute a repository package installation script, download an arbitrary server or activate extensions. By default it reuses available P01 language tooling and the explicitly downloaded P06 debugger releases. Supply these environment variables to reproduce elsewhere:

```text
ASF_P06_NODE                 absolute Node executable
ASF_P06_TYPESCRIPT_SERVER    absolute typescript-language-server/lib/cli.mjs
ASF_P06_PYRIGHT              absolute pyright/langserver.index.js
ASF_P06_PYTHON               Python environment containing debugpy
ASF_P06_RUST_ANALYZER        rust-analyzer binary (default: rustup which rust-analyzer)
ASF_P06_JS_DEBUG             official js-debug/src/dapDebugServer.js
ASF_P06_CODELLDB             official CodeLLDB adapter executable
ASF_P06_SHELL                installed authorized shell
```

Trusted sources: [TypeScript language server](https://github.com/typescript-language-server/typescript-language-server), [Pyright](https://github.com/microsoft/pyright), [rust-analyzer](https://rust-analyzer.github.io/), [debugpy](https://github.com/microsoft/debugpy), [js-debug release 1.140.0](https://github.com/microsoft/vscode-js-debug/releases/tag/v1.140.0), [CodeLLDB release 1.12.3](https://github.com/vadimcn/codelldb/releases/tag/v1.12.3). The local downloads were checked against GitHub release asset digests and retained with SHA-256 provenance. Tool payloads/VSIX archives are not committed and ASF does not claim VSIX extension compatibility.

From the repository root:

```powershell
node scripts/v2_p06/prepare.mjs
$env:ASF_IDE_TEST_TOOLS = Join-Path (Get-Location) '.factory/v2/p06/tools/profiles.json'
$env:CARGO_TARGET_DIR = './target'
node scripts/v2_p06/validate.mjs targeted
node scripts/v2_p06/validate.mjs tools
node scripts/v2_p06/validate.mjs rust
node scripts/v2_p06/validate.mjs parallel
node scripts/v2_p06/validate.mjs frontend
node scripts/v2_p06/validate.mjs frontend-1
node scripts/v2_p06/validate.mjs frontend-2
node scripts/v2_p06/validate.mjs embedded
node scripts/v2_p06/validate.mjs tooling
```

Use a suitable target directory on other machines. On Unix, export the same variables with shell syntax and use installed native tools. Tests requiring Windows shells are conditional; their absence does not qualify Unix PTY behavior. The ignored real-tool tests are executed separately by `tools`; keep their language qualification results distinct from the deterministic adversarial Python protocol fixture.

The runner records exact argv, working directories, exits, durations and Windows owned-process memory samples. A low-memory refusal is a refusal, never a pass. It retains previous results under `logs/attempts`; source or test failures are not suppressed.

## Embedded browser tests

After `embedded`, copy the built executable to an owned run directory so Windows does not lock the build output:

```powershell
New-Item -ItemType Directory -Force .factory/v2/p06/run
Copy-Item "$env:CARGO_TARGET_DIR/debug/factory.exe" .factory/v2/p06/run/factory.exe
node scripts/v2_p06/browser.mjs
node scripts/v2_p06/benchmark.mjs
node scripts/v2_p06/launch-smoke.mjs
```

`ASF_P06_BINARY` overrides that path; `ASF_BROWSER_CHANNEL` selects a locally installed Playwright browser. The harness initializes only new generated Git repositories, supplies explicit project/tool trust, starts one authenticated local server and tests actual CodeMirror/Dockview services. It retains screenshots, API/browser evidence and error states. It stops only its owned server/browser processes. It never connects to a real publication remote.

The original extension fixtures are in `extensions/p06-editor` and `extensions/p06-workspace`. Imports are genuine local installations into the isolated application registry, not preactivated production metadata. `fixtures/protocol_peer.py` is a clearly labeled deterministic security fixture; it is not a language server qualification substitute.

Existing P03/P04/P05 browser scripts accept their documented binary overrides and should also run against the same P06 embedded executable. Exact-commit remote CI and native Linux/macOS qualification remain separate from these Windows commands.
