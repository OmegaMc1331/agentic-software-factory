# P08-R1 final acceptance

This follow-up reuses P08 parallel/dependency/conflict/replanning evidence. It adds real semantic repair and two real-agent interruption cases. It does not start P09 or publish anything.

Use installed, authorized Git, Python 3.11 or newer, native Codex and Playwright Edge. Python's standard-library TOML reader is used only by the evidence exporter, not required to launch ASF. Do not change accounts, credentials, models or endpoints to avoid a usage refusal. The preflight is the existing `scripts/v2_p07_r1/preflight.py`; run it once against the selected binary before model acceptance. Login status alone does not prove available provider capacity.

Build and copy the embedded executable to a unique filename; never replace a running Windows image. The scripts default to the previously qualified P08 executable, so explicitly select the new build when validating a correction.

```powershell
$env:CARGO_TARGET_DIR = './target'
$env:ASF_VALIDATION_OUTPUT = Join-Path (Get-Location) '.factory/v2/p08-r1/logs'
node scripts/v2_p08/validate.mjs frontend
node scripts/v2_p08/validate.mjs embedded
New-Item -ItemType Directory -Force .factory/v2/p08-r1/run
Copy-Item "$env:CARGO_TARGET_DIR/debug/factory.exe" .factory/v2/p08-r1/run/my-qualified-build.exe
$env:ASF_P08_R1_BINARY = (Resolve-Path .factory/v2/p08-r1/run/my-qualified-build.exe).Path
$env:ASF_P08_R1_OUTPUT = Join-Path (Get-Location) '.factory/v2/p08-r1/my-new-acceptance'
$env:ASF_P08_R1_MODE = 'all'
node scripts/v2_p08_r1/acceptance.mjs
```

`semantic` and `interruptions` modes permit independent qualification without repeating successful native scenarios. A PASS applies only to the selected mode; inspect `mode` and the individual tests. Existing output refuses overwrite without explicit `ASF_P08_R1_RESUME=1`, which also requires the identical executable hash. A refused or failed mission is never silently reset.

The harness creates unique repositories outside ASF source with no remotes or secrets. Original source and tests intentionally contain an interval defect. The actual pre-agent test output is retained in an immutable baseline JSON file. A controlled read-only verifier executes the failing suite; the actual configured Planner revises unfinished work, and a genuine Worker chooses the patch. A real Reviewer and configured integration checks must accept it. A fresh checkout runs all tests plus an independent closed-interval oracle. The primary checkout must remain unchanged.

Interruption mode starts actual coding Workers, waits for a meaningful Worker-written regression file and records process/session/claim ownership. Case A invokes the existing cancellation endpoint while a separate controlled project process runs. Case B kills only the harness-owned test server, restarts the same SQLite project and verifies conservative reconciliation. Neither case claims completion of the interrupted code change. No arbitrary interruption UI or suspension feature is added.

The selected profiles preserve P07-R1's workspace-write MXC Worker, read-only Planner/Reviewer, disabled login shells, finite command policy and no automatic approval. Policy denials and malformed evidence fail closed. A confirmed usage-limit refusal stops the harness without another model experiment. Model counts come from actual session records; monetary costs are unavailable.

Run Cargo modes sequentially in one target directory:

```powershell
node scripts/v2_p08/validate.mjs r1
node scripts/v2_p08/validate.mjs rust
node scripts/v2_p08/validate.mjs parallel
node scripts/v2_p08/validate.mjs frontend-1
$env:ASF_P08_BINARY = $env:ASF_P08_R1_BINARY
$env:ASF_BROWSER_REGRESSION_OUTPUT = Join-Path (Get-Location) '.factory/v2/p08-r1/regressions'
node scripts/v2_p08/regressions.mjs
$env:ASF_P07_R1_BINARY = $env:ASF_P08_R1_BINARY
$env:ASF_PERMISSION_OUTPUT = Join-Path (Get-Location) '.factory/v2/p08-r1/permissions'
node scripts/v2_p08/permission-regressions.mjs
```

The exporter retains failed attempts as well as successes, selected native headers, structured final output, Git diffs, review decisions, SQLite history, process observations, test summaries and hashes. It excludes credentials and private agent reasoning. Full local logs and disposable repositories are retained separately. Inspect screenshots rather than treating image generation as acceptance. CI, other native platforms and release qualification remain separate.

`replay.mjs` performs model-free inspection of the retained accepted semantic project and interruption records. It uses the selected final `binary.json`, checks actual patch/review data, opens readonly integration code, measures narrow viewport geometry and verifies all embedded assets. Select a unique `ASF_P08_R1_REPLAY_OUTPUT`. Its optional `ASF_P08_R1_STYLE_PROBE=1` is development-only and explicitly skips final asset qualification; production qualification must run with it unset or `0`. Final retained production evidence uses `.factory/v2/p08-r1/replay-final-confirmed` and the final browser rerun uses `.factory/v2/p08-r1/regressions-production`. The exporter preserves earlier failures and diagnostic probes separately.
