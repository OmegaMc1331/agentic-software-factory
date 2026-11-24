# P07 reproducible local qualification

No script pushes, publishes, creates a PR, tags or deploys. Inputs, generated Git repositories, binaries and runtime databases are isolated under ignored `.factory/v2/p07/` and temporary registry directories. Existing user repositories are not test fixtures.

The tests use the existing Rust/Node/Edge developer toolchain. ASF's installed application does not require those tools merely to launch. The original controlled Rust agent in `crates/factory-conversation/tests/support/` is a deterministic process fixture, explicitly distinct from a real model. Browser real-agent checks select an installed Codex executable with its established authentication; no account setup or credential extraction is performed.

From the repository root on the validated Windows machine:

```powershell
$env:CARGO_TARGET_DIR = './target'
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_PROFILE_DEV_DEBUG = '0'
$env:CARGO_PROFILE_TEST_DEBUG = '0'
node scripts/v2_p07/validate.mjs targeted
node scripts/v2_p07/validate.mjs rust
node scripts/v2_p07/validate.mjs parallel
node scripts/v2_p07/validate.mjs frontend
node scripts/v2_p07/validate.mjs frontend-1
node scripts/v2_p07/validate.mjs embedded
node scripts/v2_p07/validate.mjs tooling
node scripts/v2_p07/validate.mjs baseline
```

Use a native target directory/shell on other platforms. Run Cargo gates sequentially in a shared Windows target directory: a concurrent rebuild can lock or replace an executing test image. The runner records exact argv/cwd/exits, timings, logs and sampled owned-process memory. Prior attempt results remain in `logs/attempts`. Do not count a refused or failed run as a pass. Do not edit source during a final gate run; a mixed source snapshot is not a qualification result.

After the embedded build, copy its executable to the owned run directory, avoiding Windows locking the Cargo build output:

```powershell
New-Item -ItemType Directory -Force .factory/v2/p07/run
Copy-Item "$env:CARGO_TARGET_DIR/debug/factory.exe" .factory/v2/p07/run/factory.exe
node scripts/v2_p07/browser.mjs
node scripts/v2_p07/benchmark.mjs
python scripts/v2_p07/session-metrics.py
node scripts/v2_p07/recovery-browser.mjs
```

`ASF_P07_BINARY` overrides the executable. `ASF_P07_OUTPUT_DIR` separates a new qualification attempt from retained evidence; use a separately copied executable for concurrent Windows servers. `ASF_P07_CODEX` selects an installed Codex CLI. `ASF_BROWSER_CHANNEL` selects a locally installed Playwright browser. The default real-model journey requests a code-writing mission and must not be reported successful if the agent refuses writes or provider access is unavailable.

For a separately identified **read-only model qualification**, set `ASF_P07_REAL_MODE=read_only`. That mode requests an actual research mission using an explicitly configured existing researcher role/recipe. It still plans and executes through the production engine, but it does not qualify model-generated filesystem writes. Retain the original mutating-attempt failure separately and state this limitation. The controlled form/visual/cancellation cases continue to use actual test processes and repositories.

`ASF_P07_REAL_MODE=controlled` runs the final application-mechanics journeys with the original deterministic process agent in all fixture projects. Result names and metadata explicitly exclude model qualification. This mode is used when the real provider is unavailable, while retaining the failed provider refresh. It does not establish that a real model can write code. `real-agent-evidence.py` exports the earlier successful native Codex records read-only from the fixture identified by `real-session-metrics.json`; it does not synthesize a successful browser run. Preserve that file before a controlled `session-metrics.py` run replaces the current metrics.

The benchmark uses controlled agents to measure the application rather than provider inference: 101 conversations, 200 messages with long code content, bounded history rendering, SSE acknowledgment and reload. Its observations are debug/native measurements, not latency guarantees.

Run P03-P06 harnesses against the same final executable using `ASF_P03_BINARY`, `ASF_P04_BINARY`, `ASF_P05_BINARY` and `ASF_P06_BINARY`. P04 now asserts the visible project-owned view header because P07 replaces the former project-heading home. Its active-run journey uses Workflows and the real Newest workflows control; the original ownership, execution and layout assertions remain. P03/P05/P06 harnesses are unchanged. P06 optional real tools use its documented `ASF_IDE_TEST_TOOLS` profile file. Installer/checksum/baseline tooling and native-platform limitations remain separate. No environment variable in these scripts authorizes a remote push or publication.

The recovery harness reuses the main browser fixture after that server stops. It performs real Enter/Shift-Enter submission, drops a POST acknowledgment after backend acceptance, verifies Retry creates no duplicate message, restores an unsent local draft and captures the real graph editor without submitting that draft. A fourth case temporarily configures an original test-only Node planner wrapper and the existing controlled worker latch: the actual backend waits in planning and execution until the test releases each process. Captures assert those states in the production Progress view before release, then wait for the workflow list and Progress to agree on completion. The fixture configuration is restored in `finally`. These are controlled process-state observations, not model-inference evidence. `session-metrics.py` reads the fixture database read-only to derive native durations/handoff times. The WAL mechanism can be reproduced separately with `python scripts/v2_p07/wal-reproduction.py`.
