# P05 local verification

Run from the repository root. The scripts keep generated repositories, application registries, binaries and logs under `.factory/v2/p05` or the OS temporary directory. They do not operate on a user's source repository. The production entry is `/workspace.html`; `/` remains V1.

```powershell
$env:CARGO_TARGET_DIR = './target'
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_PROFILE_DEV_DEBUG = '0'
$env:CARGO_PROFILE_TEST_DEBUG = '0'
node scripts/v2_p05/validate.mjs targeted
node scripts/v2_p05/validate.mjs rust
node scripts/v2_p05/validate.mjs parallel
node scripts/v2_p05/validate.mjs frontend
node scripts/v2_p05/validate.mjs frontend-1
node scripts/v2_p05/validate.mjs frontend-2
node scripts/v2_p05/validate.mjs embedded
```

The custom target directory above reuses this machine's existing cache. Omit it to use the normal repository `target` directory. `frontend` executes the unchanged default test command; the one/two-worker runs execute the same complete suite and measure resource differences. On Windows the runner samples only its own child process tree. If remaining system commit falls below 512 MiB, it terminates that owned npm tree and records `REFUSED_MEMORY_RESERVE`, never a pass. No such refusal occurred in the final P05 runs.

Copy the newly built native executable into the packaging directory before browser tests, so running servers do not lock Cargo's build output:

```powershell
New-Item -ItemType Directory -Path .factory/v2/p05/packaging -Force | Out-Null
Copy-Item -LiteralPath "$env:CARGO_TARGET_DIR/debug/factory.exe" -Destination .factory/v2/p05/packaging/factory.exe
Copy-Item -LiteralPath LICENSE -Destination .factory/v2/p05/packaging/LICENSE
Copy-Item -LiteralPath docs/v2/third-party-notices-p05.txt -Destination .factory/v2/p05/packaging/THIRD-PARTY-P05.txt
$env:ASF_P05_BINARY = "$PWD/.factory/v2/p05/packaging/factory.exe"
$env:ASF_P04_BINARY = $env:ASF_P05_BINARY
$env:ASF_P03_BINARY = $env:ASF_P05_BINARY
node scripts/v2_p05/browser.mjs
node scripts/v2_p03/browser.mjs
node scripts/v2_p04/browser.mjs
```

Browser automation uses the dashboard's existing Playwright installation and Edge by default (`ASF_BROWSER_CHANNEL` can select another installed compatible channel). It launches one native local server, uses the actual authenticated project APIs, and independently verifies files on disk. The Windows server launch PATH includes Git and Windows system directories, excluding Node and Rust. Development tools are needed to execute the test harness, not to launch the packaged application.

The P05 harness includes real filesystem write refusal, external writer conflicts, read-only agent worktrees, copied-window draft contention, server restart, read-only diffs and bounded search. It never publishes Git changes or executes new terminal/LSP/DAP services. Performance fixtures are controlled real disk files, separated from live user data.

The collector requires the final expected passing counts and rejects production source drift. When collecting from the later documentation commit, set `ASF_P05_SOURCE_COMMIT` to the tested implementation SHA from the delivery report. Retained failure logs are explicitly distinguished from final gates. The full validation is Windows-local only; Linux/macOS, exact-commit CI, release packaging and P06–P14 remain separate gates.
