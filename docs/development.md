# Development

Use Rust stable and Node.js 20 or later. Normal users install a release and operate
workflows from Agent Graph; contributor tests don't use a public workflow CLI command.

## Quality gates

Run the Rust workspace gates from the repository root:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Run the dashboard gates from `apps/dashboard`:

```bash
npm ci
npm run format:check
npm run lint
npm run typecheck
npm test
npm run build
```

Core and API workflow tests use configured fake executables and temporary Git
repositories. CI doesn't require Codex, Claude Code, or OpenCode. Git and worktree tests
are intentionally slower than pure unit tests.

## Local dashboard development

Build the dashboard once, then run the API and Vite in separate terminals:

```bash
cd apps/dashboard
npm ci
npm run build
cd ../..
cargo run -p factory-cli -- dev serve
```

```bash
cd apps/dashboard
npm run dev
```

Vite serves `http://localhost:5173` and proxies `/api` to the Factory process on port
4321. The dashboard's pure layout modules (`src/layout.ts` and
`src/networkLayout.ts`) stay independent of React.

## Workflow runtime

`POST /api/runs` persists a `planning` Run and asks `factory-runtime` to own the
background operation. Start, cancel, and retry use their explicit routes; don't add a
generic action, command, or process endpoint.

Factory Core owns workflow transitions and validation. The runtime owns only active
Tokio jobs and run-scoped cancellation signals. Keep database operations short: never
hold the API `FactoryDb` mutex or a SQLite transaction while an external agent runs.

Every Planner, Worker, and Reviewer invocation must create an `AgentSession`. Worker
execution also creates a durable `TaskAttempt` containing status, worktree, exit code,
evidence, and structured review. Process exit code 0 doesn't complete a task without
Reviewer approval. The centralized retry limit is `MAX_TASK_ATTEMPTS`.

Agent entries have a small invocation profile: kind, workflow arguments, prompt
transport, environment, and optional interactive arguments. Automated execution uses
either a stdin payload or one process argument and never shell interpolation. Treat
missing executables, disabled automated transport, invalid placeholders, and detected
TTY requirements as configuration failures; they must not consume the normal retry
loop.

Executable discovery is centralized in `factory-agent`. It resolves explicit paths and
the Factory process `PATH`; on Windows it also follows `PATHEXT` for native executables
and `.cmd`/`.bat` shims. Automated and PTY invocations consume that same resolved
target. npm shims are unwrapped to their packaged native executable or Node entry point
when possible, without rewriting `.factory/config.toml` or interpolating mission text
through a shell. Keep the controlled Windows shim execution and ConPTY regressions in
the test suite.

Opening the API state reconciles records left `running` by a previous process. Add
migrations for schema changes; don't rewrite existing migrations.

Focused commands:

```bash
cargo test -p factory-core --test workflow_runtime
cargo test -p factory-db
cargo test -p factory-api --test api workflow_
```

## Agent Graph and sessions

The Agent Graph uses `@xyflow/react` for pointer dragging, connection handles, and
viewport controls. Keep Factory entities from `GET /api/graph` separate from visual
state in `.factory/graph.json`. Save positions on drag end, not during movement.

Workflow nodes come from real Runs. Task nodes and dependency edges come from SQLite.
Role assignments update `.factory/config.toml`; custom links, groups, notes, and
memberships remain visual-only.

The Agent Console consumes persisted session data. Automated sessions use the
session-scoped SSE route. Interactive sessions are started for a configured agent,
owned by `factory-runtime`, and connected through a session-scoped WebSocket to a
`portable-pty` terminal. Propagate xterm dimensions to `MasterPty::resize`; never add a
generic shell or executable endpoint.

Invocation tests construct commands without authenticated external agents. The runtime
PTY probe runs the test executable inside ConPTY/PTY and asserts that stdin is a
terminal. Keep Windows coverage enabled where ConPTY is available.

Frontend tests cover workflow creation, plan inspection, start errors, graph dragging,
custom edge deletion, and Agent Console states:

```bash
cd apps/dashboard
npm test
```

## Embedded dashboard build

Release binaries embed `apps/dashboard/dist` with `rust-embed`:

```bash
cd apps/dashboard
npm ci
npm run build
cd ../..
cargo build --release --features embedded-dashboard -p factory-cli
```

The embedded feature fails to compile when `dist` is missing. A normal development
build reads `apps/dashboard/dist` from disk or serves a page explaining that it must be
built. Test embedding with:

```bash
cargo test --release --features embedded-dashboard -p factory-api
```

## Releasing

Pushing a version tag runs [the release workflow](../.github/workflows/release.yml),
builds all supported platform archives, verifies installers, and creates a GitHub
Release. Do not tag routine changes automatically.

```bash
git tag v0.3.3
git push origin v0.3.3
```

[CI](../.github/workflows/ci.yml) runs Rust checks on Ubuntu and Windows, dashboard
checks, an embedded release-style smoke test, and installer tests.
