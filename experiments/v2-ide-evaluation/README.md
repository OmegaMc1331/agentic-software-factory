# ASF V2-P01 isolated IDE experiments

These are technical experiments, not a production frontend replacement. Read [the evaluation](../../docs/v2/ide-evaluation.md) and [ADR](../../docs/v2/adr-ide-foundation.md). No P02–P14 implementation, remote publication, AI mission, provider account or arbitrary extension execution is included.

## Layout

* `monaco/adapter.ts`: Monaco 0.57 native editor/LSP/diff and attempted monaco-yaml integration.
* `codemirror/adapter.ts`: CodeMirror 6 editor/merge/official LSP and real native YAML module.
* `shared/`: identical React/Dockview composition, authenticated client, debugger protocol client, xterm view and Vite build.
* `backend/`: independent Cargo workspace reusing actual Factory Runtime, process-tree ownership and source-included unchanged API security. It accepts only explicit owned P01 Git fixtures.
* `theia/`: genuine filesystem service integration spike; no full Theia workbench claim.
* `openvscode/`: official-release archive/native-runtime screening spike; no running workbench claim.
* `tests/`: browser workflows, adversarial API/protocol checks, comparable benchmarks and relocated offline packaging test.
* `scripts/`: fixture setup, runtime packaging, validation and derived metric summary.

Runtime state, virtualenvs, downloads, build assets, screenshots and logs go to `.factory/v2/p01/`. Dependencies and caches are ignored. Nothing here writes a production project, sends GitHub mutations or starts an agent. Fixture reset operations check the ownership marker. Do not point the probe at a real repository: it deliberately refuses unmarked roots and initializes Factory configuration inside its owned fixtures.

## Reproduce on the evaluated Windows host

Prerequisites for building experiments: Rust/Cargo, Git, Node 24.22.1/npm, Python, installed Microsoft Edge. The evaluated versions and host are retained with the evidence. End-user packaged launch is tested separately with Node/Python/Rust removed from PATH.

From repository root, use PowerShell:

```powershell
python -m venv .factory/v2/p01/python
.factory/v2/p01/python/Scripts/python.exe -m pip install debugpy==1.8.22
Push-Location experiments/v2-ide-evaluation
npm ci --ignore-scripts
npm run typecheck
npm run build:monaco
npm run build:codemirror
Pop-Location
python experiments/v2-ide-evaluation/scripts/prepare.py
cargo build --manifest-path experiments/v2-ide-evaluation/backend/Cargo.toml --locked --target-dir target -j 1
target/debug/asf-p01-ide-probe.exe .factory/v2/p01/config.json 4387
```

The server stays in the foreground and prints `P01_READY`. Open `http://127.0.0.1:4387/codemirror/?project=small` or `/monaco/?project=small`. The test fixture selector also offers medium, large and other independent roots. The conversation area is explicitly a composition surface without an AI backend. The process view and protocol data come from actual execution.

In another shell:

```powershell
Push-Location experiments/v2-ide-evaluation
node tests/evaluate.mjs
node tests/security.mjs
node tests/benchmark.mjs
node tests/services-benchmark.mjs
Pop-Location
python experiments/v2-ide-evaluation/scripts/summarize.py
```

The all-candidate functional command returns **1** for the recorded Monaco YAML incompatibility. That result is retained rather than skipped or labelled passed. To reproduce the selected stack alone:

```powershell
Push-Location experiments/v2-ide-evaluation
$env:P01_CANDIDATES = 'codemirror'
node tests/evaluate.mjs
Remove-Item Env:P01_CANDIDATES
Pop-Location
```

`P01_BASE_URL` can select a different loopback port. Tests mutate only marked fixture files; they reset experiment layout/plugin state and compare actual filesystem results. Protocol leases expire after 180 seconds. Do not run these suites concurrently against the same fixture roots. Do not rebuild static assets while a browser test is loading them. Benchmarks must run without regression builds/tests for comparable measurements; retain outliers and the raw JSON.

For graceful shutdown, use authenticated `POST /api/shutdown` after closing browser WebSockets. `tests/package.mjs` demonstrates the complete authenticated shutdown sequence. Killing the parent console is not the normal cleanup protocol. The probe's process ownership and fixed profiles are deliberately narrower than a production IDE.

## Screening spikes

```powershell
Push-Location experiments/v2-ide-evaluation/theia
npm ci --ignore-scripts --legacy-peer-deps
node spike.cjs
Pop-Location
python experiments/v2-ide-evaluation/openvscode/spike.py
```

Theia reads only an owned outside-project marker file to demonstrate default Node authority. No security boundary of a running Theia web application is attacked. OpenVSCode downloads the official release archive into ignored state, reads named regular members and records its Linux ELF runtime failing native Windows execution. The release query is time-dependent; retained evidence pins the actual tag/hash used in P01. Do not treat a future latest release as the same evaluated binary.

## Representative Windows package

Build assets first, then:

```powershell
cargo build --release --features embedded-assets --manifest-path experiments/v2-ide-evaluation/backend/Cargo.toml --locked --target-dir target -j 1
python experiments/v2-ide-evaluation/scripts/package_windows.py
Push-Location experiments/v2-ide-evaluation
node tests/package.mjs
Pop-Location
```

The recipe verifies published hashes of Node 24.22.1 and Python 3.14.10 Windows x64 archives, copies pinned tooling and notices, embeds both comparison frontends in Rust, and generates a local archive/manifest/checksum. It does not publish anything. Node runs only as a supervised tool, not a second HTTP backend. Python 3.14.10 is bundled rather than copying a non-relocatable venv interpreter.

The test expands the zip to a different owned directory, launches its PowerShell wrapper with only Windows system directories on PATH, blocks external browser HTTP requests, saves actual files, connects bundled LSP/DAP, activates YAML, exercises existing ConPTY and restores state. It verifies launcher/backend/helper/PTY exit. Its user-data fixture is outside installation. The test runner itself uses development Node/Playwright/Edge; the launched application/helpers must use packaged paths. Blocking browser requests is not an OS-wide firewall test; no claim that trusted helper code is sandboxed is made.

The package is a local feasibility artifact. It is not an ASF release, a finished updater or a complete installer qualification. Per-package notices are preserved; shipping SBOM/source-obligation completeness remains a release gate.

## Proposed four-target production pipeline — not executed or installed by P01

This is an exact pipeline proposal for later implementation. The existing `.github/workflows/release.yml`, installers and product packages remain unchanged.

| Native runner | Rust target | Node helper artifact | Python/debugger pack strategy | Required native smoke |
| --- | --- | --- | --- | --- |
| Windows x64 | `x86_64-pc-windows-msvc` | `node-v24.22.1-win-x64.zip` | Verified CPython 3.14.10 embeddable amd64 + target debugpy wheel/source/notices | Embedded launch, auth, files, LSP, DAP, ConPTY, cleanup, offline and relocation |
| Linux x64 | `x86_64-unknown-linux-gnu` | `node-v24.22.1-linux-x64.tar.xz` | Approved reproducible relocatable CPython pack or native build with pinned sources; debugpy matching ABI/architecture | Same plus PTY/process group, loader/libc dependency audit |
| macOS Intel | `x86_64-apple-darwin` | `node-v24.22.1-darwin-x64.tar.gz` | Pinned native relocatable CPython build/pack and target debugpy; review SDK/deployment target | Native Intel execution, tool launch, PTY, codesigning/notarization and cleanup |
| macOS ARM64 | `aarch64-apple-darwin` | `node-v24.22.1-darwin-arm64.tar.gz` | Same native ARM64 runtime/adapter pack | Native Apple Silicon execution and same gates |

Sequence on each runner:

1. Checkout the exact approved commit, verify existing P00 inventory/fixtures, install pinned build tools and run existing CI gates.
2. `npm ci --ignore-scripts` in the isolated prototype (later the approved production frontend); `npm run typecheck`; build assets. Match lockfile integrity/provenance and license inventory.
3. `cargo build --release --locked --target <target> --features embedded-assets --manifest-path experiments/v2-ide-evaluation/backend/Cargo.toml`. This is the probe command; a production feature/package name requires its later authorized implementation.
4. Fetch target Node from the official distribution; verify official SHA-256/signatures according to the shipping trust policy. Assemble ABI-matched Python/debugpy packs, verify checksums/provenance/notices and native dynamic libraries. Linux/macOS CPython relocation recipes are **pending**, not demonstrated by the Windows embeddable zip.
5. Package assets/binaries/tool packs read-only, generate SBOM/license notices/per-file manifest, sign as applicable, produce archives and SHA-256. Extensions/configuration live under user data, projects remain at their roots.
6. Extract to a clean directory and run the native offline/relocation/API/browser/protocol/PTY/process-exit tests with installed development toolchains removed from PATH. Test upgrade/uninstall preserving user/project data and rollback after interrupted updates.
7. Collect exact-commit native evidence. Do not publish until P14 Go/No-Go and explicit release authorization.

The selected static architecture does not depend on native Node addons for its workbench. Adapter packs can contain native binaries and must be audited independently. No Linux/macOS runtime success, portable Python pack or installer result is inferred from this pipeline text. Missing adapters must appear unavailable in the product until installed/qualified, and all mandatory V2 IDE journeys must work before release.

## Local quality gates

```powershell
python experiments/v2-ide-evaluation/scripts/validate.py
```

This runs production Rust fmt/Clippy/tests, P00 inventory/tooling, checksum tests, dashboard format/lint/typecheck/test/build, and experimental Rust/type checks. The dashboard test suite uses `--maxWorkers=1` because this 16 GB host has the memory/commit failures documented by P00; no test is removed. Exact commands, exit codes, durations and log hashes are emitted to `.factory/v2/p01/validation.json`. The final all-candidate browser result remains separate from production regressions. Local success does not imply a remote CI run.

The React quality review notes explicit cleanup of editor/xterm/plugin instances and stable top-level components. Prototype-only limitations include coarse subscription rerenders, a full DOM file list, view-level lifecycle tied to the page, `any` at protocol/test seams and no screen-reader/IME qualification. These are identified production work, not evidence of P02 completion. The experiment intentionally makes no design-system/AA or final product-performance claim.
