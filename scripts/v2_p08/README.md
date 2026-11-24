# P08 local qualification

These runners use disposable Git repositories, supported ASF APIs, native child processes and the embedded workspace. They never push, publish, create a PR, change credentials, grant approvals or apply a prepared application patch. Controlled process fixtures and real coding agents are reported separately.

Run Cargo modes sequentially in one target directory:

```powershell
$env:CARGO_TARGET_DIR = './target'
node scripts/v2_p08/validate.mjs scheduler
node scripts/v2_p08/validate.mjs lifecycle
node scripts/v2_p08/validate.mjs rust
node scripts/v2_p08/validate.mjs parallel
node scripts/v2_p08/validate.mjs frontend
node scripts/v2_p08/validate.mjs frontend-1
node scripts/v2_p08/validate.mjs embedded
New-Item -ItemType Directory -Force .factory/v2/p08/run
Copy-Item "$env:CARGO_TARGET_DIR/debug/factory.exe" .factory/v2/p08/run/factory-p08.exe
$env:ASF_P08_BINARY = (Resolve-Path .factory/v2/p08/run/factory-p08.exe).Path
```

Copy a rebuilt executable to a new filename while any earlier server is running; Windows locks the running image. Set `ASF_P08_BINARY` to that exact copy for browser runs. The final qualification uses `.factory/v2/p08/run/factory-p08-final-8b249b4.exe`; the real model runs retain their earlier binary hashes independently.

`validate.mjs` records exact argv, source fingerprint, exits, log paths and sampled process-tree memory. Earlier result summaries are archived. Full Rust serial and default-threaded runs remain separate gates. Default Vitest configuration uses two workers; one-worker verification does not establish that unrestricted parallelism is universally safe.

Real qualification requires Git, Python, native Codex CLI, its established authentication, a supported writable MXC Worker sandbox, read-only Planner/Reviewer profiles and installed Playwright Edge. No account setup or model downloads happen automatically. Choose a new output directory to preserve failed evidence:

```powershell
$env:ASF_P08_OUTPUT = Join-Path (Get-Location) '.factory/v2/p08/my-new-acceptance'
node scripts/v2_p08/acceptance.mjs
```

The harness creates three repositories outside ASF source. The browser submits a chat objective and authored visual plans. Four independent features and a dependent combined test exercise capacity two. Overlapping real process intervals and simultaneous Windows PIDs establish parallelism. Two overlapping feature edits exercise actual Git conflict/replanning. A producer/client migration exercises refusal of a semantically broken candidate. Models choose and write the implementation; the harness only prepares initial fixtures, submits objectives and independently checks outputs.

No acceptance result is inferred from a UI indicator, process exit zero or Reviewer prose alone. Each successful scenario requires a relevant diff, actual review, controlled publication, configured tests and a fresh checkout test. Primary HEAD and user checkout must remain unchanged; managed roots remain read-only through P05.

`ASF_P08_RESUME=1` permits explicit reconciliation on an identical binary. It does not hide a failed run, change its plan or automatically replay side effects. A provider usage limit is an external blocker; do not loop model invocations, switch credentials or treat an unexecuted retry as qualification. The retained first run at `.factory/v2/p08/acceptance` failed parallel qualification. The second run at `.factory/v2/p08/acceptance-parallel-r2` passed parallel/dependency/capacity and conflict/replanning before the provider refused further real coding work.

```powershell
node scripts/v2_p08/regressions.mjs
$env:ASF_P07_R1_BINARY = $env:ASF_P08_BINARY
node scripts/v2_p08/permission-regressions.mjs
$env:ASF_IDE_TEST_TOOLS = (Resolve-Path .factory/v2/p06/tools/profiles.json).Path
node scripts/v2_p08/validate.mjs tools
node scripts/v2_p08/replay.mjs
python scripts/v2_p08/verify-behavior.py
```

Browser regressions reuse P03-P07 assertions with separate P08 output directories and the selected P08 binary. The permission regression retains the original R1 assertions and controlled process boundary. Optional IDE tools must already be installed. `replay.mjs` restores the accepted durable state and inspects final assets/UI without starting another model mission. Reports distinguish actual execution, retained failures, unavailable model capacity, native Windows evidence, pending Linux/macOS and unverified remote CI. P09/P10 remain outside scope.
