# P10 local qualification

These runners preserve each failed attempt. They do not push or publish. Real-model acceptance uses disposable repositories outside the ASF source tree and never applies a predetermined patch. The model-free mode is explicitly a controlled process fixture.

On the validated Windows host, use the existing installed Rust, Python, Node, Edge/Playwright and trusted coding CLI. No model gateway or credential installation is introduced.

```powershell
$env:CARGO_TARGET_DIR='./target'
$env:CARGO_INCREMENTAL='0'
$env:CARGO_PROFILE_DEV_DEBUG='0'
$env:CARGO_PROFILE_TEST_DEBUG='0'
node scripts/v2_p10/validate.mjs rust
node scripts/v2_p10/validate.mjs parallel
node scripts/v2_p10/validate.mjs frontend
node scripts/v2_p10/validate.mjs embedded
```

Keep Rust source frozen during Cargo execution. Copy the built embedded executable to a unique path before starting it; do not replace a running Windows executable. Select it with `ASF_P10_BINARY`. Output directories must be unique or empty; the acceptance runner refuses to overwrite an existing result.

```powershell
$env:ASF_P10_BINARY='./target/debug/factory.exe'
$env:ASF_P10_OUTPUT='.factory/v2/p10/new-controlled-attempt'
node scripts/v2_p10/acceptance.mjs
```

After a controlled PASS, verify real availability once with the installed authorized CLI and intended model. The preflight requires `--cli`, `--factory` and `--output`; it preserves the probe repository and removes only the independently verified probe leaf.

```powershell
python -B scripts/v2_p10/preflight.py --cli codex --factory $env:ASF_P10_BINARY --output '.factory/v2/p10/preflight'
$env:ASF_P10_OUTPUT='.factory/v2/p10/native'
node scripts/v2_p10/acceptance.mjs --native
```

A provider usage refusal stops real-model qualification. Preserve completed evidence; do not poll quota or change accounts/endpoints to evade it. The runner checks the genuine preflight PASS and uses supported workspace-write/read-only modes without approval bypasses. Confirm the configured executable/model in the script matches the user's authorized installation.

The combined journey deliberately interrupts owned child Workers and an owned test server. It never targets an OS PID supplied by the browser or an unrelated process. The fixture retains original failing tests, code diffs, worktree/checkpoint metadata, real review decisions and fresh-checkout verification. The 35-second original validation delay makes the genuine coding operation observable; it must not be removed to accelerate the run.


Before final qualification, inspect screenshots, check native process cleanup and independent Git/test results, run all requested gates, verify source/binary provenance and commit the report with a clean tree. Local results do not qualify remote CI, native Linux/macOS, universal provider support or release readiness.
