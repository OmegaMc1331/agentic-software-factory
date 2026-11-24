# P07-R1 qualification runners

These commands create disposable local repositories and use established CLI authentication. They do not publish, push, create PRs, tag, deploy, change ACLs or grant approvals. No script generates an application correction; the real model decides and writes the calculator implementation and regression tests.

## Native Windows prerequisites

The observed environment uses Codex CLI 0.165.0, its supported MXC sandbox, Git, Python unittest, the Rust toolchain, Node and the locally installed Playwright Edge channel. ASF does not acquire model credentials. Optional tool prerequisites are independent of launching ASF. The profile deliberately remains workspace-write for Worker and read-only for Planner/Reviewer, with approval `never` inherited in the qualified environment. Managed requirements are not ignored. Do not replace these settings with full access or a sandbox-bypass argument.

Build from the repository root, running Cargo commands sequentially in the shared target directory:

```powershell
$env:CARGO_TARGET_DIR = './target'
node scripts/v2_p07_r1/validate.mjs permissions
node scripts/v2_p07_r1/validate.mjs rust
node scripts/v2_p07_r1/validate.mjs parallel
node scripts/v2_p07_r1/validate.mjs targeted
node scripts/v2_p07_r1/validate.mjs frontend
node scripts/v2_p07_r1/validate.mjs frontend-1
node scripts/v2_p07_r1/validate.mjs embedded
New-Item -ItemType Directory -Force .factory/v2/p07-r1/run
Copy-Item "$env:CARGO_TARGET_DIR/debug/factory.exe" .factory/v2/p07-r1/run/factory-r1.exe
```

The executable copy prevents Windows build-image locking. The gates record exact argv, cwd, exit codes and sampled process-tree memory in `.factory/v2/p07-r1/logs`; previous summaries are archived. Default frontend concurrency is two workers, with unchanged tests/assertions. An original unrestricted-worker attempt exhausted the 500 MiB free-commit reserve and was refused, not counted as a pass. Do not delete that evidence or describe the historical memory problem as universally solved.

## Real write preflight and coding journeys

Choose a new preflight output directory on every invocation; the wrapper refuses to overwrite an existing record. No cleanup recursively deletes a test root. It removes only a marker whose exact bytes were independently verified.

```powershell
python scripts/v2_p07_r1/preflight.py --cli codex --factory .factory/v2/p07-r1/run/factory-r1.exe --output .factory/v2/p07-r1/my-new-preflight
$env:ASF_P07_R1_PREFLIGHT = (Resolve-Path .factory/v2/p07-r1/my-new-preflight/preflight.json).Path
node scripts/v2_p07_r1/acceptance.mjs
```

`ASF_P07_R1_BINARY`, `ASF_P07_R1_CODEX`, and `ASF_BROWSER_CHANNEL` select explicitly installed local tools. Defaults refer to the qualification machine, not portable packaged dependencies. The wrapper is intentionally Windows-specific; native Linux/macOS qualification requires their supported sandbox/tool configuration and is not inferred from these results.

The acceptance harness first independently reproduces a real failing test in each repository. It then submits through the real chat, form and authored visual-plan controls, waits for actual runtime state, checks a meaningful implementation/test diff, matches the approving Reviewer to its actual attempt, and independently clones the integration SHA. It runs both the model-produced tests and a separately specified integer oracle. It verifies the unchanged primary `main` version, correct integration refs, project-owned persisted associations and P05's ordinary-editor write refusal on managed worktrees. Finally it opens an explicitly registered integration checkout in CodeMirror and checks rendered lines against disk.

If an acceptance assertion fails, results and screenshots remain under `.factory/v2/p07-r1/`. Fix the actual condition before retrying. An optional resume is explicit:

```powershell
$env:ASF_P07_R1_RESUME = '1'
node scripts/v2_p07_r1/acceptance.mjs
```

Resume requires the identical application binary. Accepted coding runs are not resubmitted; completed runs can be independently rechecked after a harness-only failure. A failed run remains recorded, and a subsequent corrected objective is a new explicit conversation submission. The registry entry is resolved by canonical root, never listing position. Every previous result file and screenshot set is archived. Do not use resume to claim unexecuted model work or hide a denied mission.

## Browser regressions and evidence

```powershell
node scripts/v2_p07_r1/regressions.mjs
node scripts/v2_p07_r1/permission-regressions.mjs
```

The regression wrapper runs the existing P07 controlled-process/recovery journeys and P03-P06 browser suites against the same binary, sequentially. `--existing-p07` and `--existing-recovery` only reuse verified passing runs on the identical binary hash; `--existing-p03-p05` requires passing records naming this exact binary copy. Controlled process agents are regression evidence, distinct from the real-model coding acceptance. The separate permission regression launches original controlled Python agents through actual Factory worktrees, tests, independent reviews and integration. It proves failed real tests cannot be approved/integrated and read-only project policy cannot inherit another project's write access. P06 requires its existing `.factory/v2/p06/tools/profiles.json` or `ASF_IDE_TEST_TOOLS` profile. It does not download or silently replace tools.

CI inspection is a read-only GitHub REST query for exact SHAs, across all trigger types. It never triggers a workflow. The exporter reads the original failed project's SQLite database in read-only mode, selects policy/header/denial fields, omits startup transport HTML and credentials, and exports actual patches, model tool traces, test results and screenshots. The final collector records hashes of those retained artifacts. The original P07 report and failure evidence remain authoritative historical records.

After a final embedded rebuild, a separate copy can replay the accepted durable state without submitting another mission:

```powershell
Copy-Item "$env:CARGO_TARGET_DIR/debug/factory.exe" .factory/v2/p07-r1/run/factory-final.exe
node scripts/v2_p07_r1/final-replay.mjs
```

That replay explicitly opens registered runtimes after restart, checks all three accepted conversation/run/integration associations, renders the approved patch in the browser, rejects a foreign conversation identity, compares every served production asset with the final frontend build and verifies V1. Its binary hash is recorded separately from the binary that ran the real model missions. It is not counted as another model execution.

No runner launches P08. Local acceptance is separate from remote CI, unavailable native platforms and ASF V2.0 release qualification.
