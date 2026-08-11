# Agentic Software Factory

Agentic Software Factory orchestrates coding agents through structured, verifiable
execution: a model plans a software objective into ordered tasks, every task runs in
its own isolated git worktree, and the system persists all state locally in SQLite.

Instead of letting an agent roam freely over a repository, the factory decomposes work
into tasks with explicit acceptance criteria and dependencies, tracks their state as a
strict state machine, and records every step in a local database you can inspect
through a CLI, an HTTP API, and a dashboard.

## Core principles

- **The system owns state.** All of it lives in `.factory/`: the SQLite database, the
  worktrees, and nothing else. The repository stays clean.
- **Tasks are verifiable.** Every task carries acceptance criteria. A task is only ever
  `completed` by a human or agent that confirms those criteria, never by assumption.
- **Strict transitions.** Each task moves through `pending -> ready -> running ->
  completed` (or `failed`, with `blocked` derived from dependencies). A worktree is
  created while a task is `ready`; completing requires the task to be `running`.
  Invalid transitions are rejected, including any attempt to skip ahead.
- **Dependencies are transitive.** Marking a task as completed or failed cascades to its
  dependents. Blocked tasks propagate `blocked` up the graph until their blockers clear.
- **Isolation.** Each task gets a branch (`factory/t<id>`) and a worktree
  (`.factory/worktrees/t<id>`), so parallel agents never collide.
- **No magic.** A deterministic local planner is available when no model API key is
  configured, so the whole system works offline and is testable.

## Contents

1. [State and filesystem](#1-state-and-filesystem)
2. [Installation](#2-installation)
3. [The task lifecycle](#3-the-task-lifecycle)
4. [Worktrees](#4-worktrees)
5. [Model providers](#5-model-providers)
6. [The CLI](#6-the-cli)
7. [The HTTP API](#7-the-http-api)
8. [The dashboard](#8-the-dashboard)
9. [Testing](#9-testing)
10. [Roadmap](#10-roadmap)

## 1. State and filesystem

The factory root is the directory where you run `factory`. All state lives under
`.factory/`, which is ignored by Git:

```text
.factory/
  db.sqlite3                 # SQLite database (runs, tasks, dependencies)
  worktrees/t<task-id>/      # one isolated git worktree per task
```

## 2. Installation

Requirements: Rust (stable), Node.js 18+ (for the dashboard).

```bash
cargo build --release
# binary at target/release/factory
```

### Quick start (offline)

The factory ships with a deterministic local planner. Set the provider to `local` and
everything else works with zero configuration:

```bash
export FACTORY_PROVIDER=local
factory init
factory run "Build a small HTTP server in Rust"
factory status
```

### Using a model provider

```bash
export FACTORY_PROVIDER=openai
export FACTORY_BASE_URL=https://api.openai.com/v1
export FACTORY_API_KEY=sk-...
export FACTORY_MODEL=gpt-4o-mini
```

All variables are optional with sensible defaults:

| Variable             | Default                | Description                          |
| -------------------- | ---------------------- | ------------------------------------ |
| `FACTORY_PROVIDER`   | `openai` if no key     | Provider kind (`openai` or `local`)  |
| `FACTORY_BASE_URL`   | `https://api.openai.com/v1` | OpenAI-compatible base URL      |
| `FACTORY_API_KEY`    | *(none)*               | API key; `run` falls back to local   |
| `FACTORY_MODEL`      | `gpt-4o-mini`          | Model name                           |

`factory init` and most commands work without an API key (they use the local planner).
`factory run` builds the provider from the environment and fails with a clear message if
no key is configured and `FACTORY_PROVIDER` is not `local`. See `.env.example`.

## 3. The task lifecycle

A plan produces tasks with titles, objectives, acceptance criteria, and dependencies.
Every task exists in exactly one state; transitions are validated against a fixed table:

```text
pending -> ready      -> running -> completed
              |            |   |
              |            |   +--> failed
              |            +-----> blocked
              |
              +--------> blocked (derived, may also be marked directly)

blocked -> ready          (a blocker cleared or a failed task was retried)
failed  -> ready          (retry/replan the failed task)
completed                 (terminal: never changes again)
```

- `pending`   - not yet eligible (has uncompleted dependencies)
- `ready`     - dependencies are all completed; a worktree can be created
- `running`   - a worktree has been created and work is underway
- `completed` - acceptance criteria verified; only reachable from `running`
- `failed`    - work stopped; blocks dependents
- `blocked`   - at least one dependency is `failed` or `blocked`

**Cascade propagation.** Marking a task `completed`, `failed`, or `blocked` recomputes
every transitive dependent from its own dependencies: any `failed`/`blocked` dependency
makes it `blocked`; all dependencies `completed` makes it `ready`; otherwise it stays
`pending`. Tasks that are already `completed` or `failed` are never re-evaluated by a
cascade - their state is authoritative.

**Recovery.** `blocked -> ready` and `failed -> ready` are the only ways back, and both
re-evaluate the downstream chain: once the failed blocker is retried (`failed -> ready`)
or a `blocked` dependency clears, affected tasks move back to `ready` or stay `pending`
based on their own dependencies. There is no way to silently reorder history: a
`completed` task is terminal.

## 4. Worktrees

Each `ready` task gets a dedicated worktree when you create it:

```bash
factory worktree create 3
# created worktree at C:\path\.factory\worktrees\t3
```

- Branch name is `factory/t<task-id>`.
- The worktree lives inside `.factory/worktrees/`, so the main tree stays clean.
- Worktrees must be clean before removal; `factory worktree remove <id>` refuses to
  remove worktrees with uncommitted changes.
- `factory worktree status` lists every worktree of the repository.

## 5. Model providers

`factory-core` exposes a `Provider` trait with a single planning method. Two
implementations exist today:

- `OpenAICompatibleProvider` - calls any OpenAI-compatible chat completions endpoint
  with your configured model. The planner asks for a strict JSON object with
  `objective`, `tasks` (with `id`, `title`, `objective`, `acceptance_criteria`,
  `dependencies`), and `exit_criteria`.
- `LocalProvider` - a deterministic fallback that plans the objective into a fixed,
  ordered five-task pipeline with dependency chains and acceptance criteria. It always
  reports `local-planner` as the model and zero token usage.

Plans are validated: non-empty fields, dependency ids must exist, the dependency graph
must be acyclic, and at most 50 tasks. Invalid responses are rejected and re-requested
up to three times; code fences around the JSON are stripped automatically.

## 6. The CLI

`factory` (binary `factory`, crate `factory-cli`):

```bash
factory init [--force]              # initialize state in this directory
factory run "<objective>"           # plan a run and persist tasks
factory status                      # summary of the latest run and its tasks
factory tasks [--run <id>]          # list tasks of a run (latest by default)
factory inspect <task-id>           # full task detail + acceptance criteria
factory mark <task-id> <state>      # transition: pending|ready|running|blocked|failed|completed
factory worktree create <task-id>   # create an isolated worktree
factory worktree remove <task-id>   # remove a clean worktree
factory worktree status             # list repository worktrees
factory serve [--port 4321]         # serve the local HTTP API
```

## 7. The HTTP API

`factory serve` starts a local API (default `http://127.0.0.1:4321`) consumed by the
dashboard:

| Method | Route            | Description                        |
| ------ | ---------------- | ---------------------------------- |
| GET    | `/api/health`    | Service health                     |
| GET    | `/api/runs`      | Runs with per-state task counts    |
| GET    | `/api/runs/:id`  | Full run, tasks, and token usage   |

## 8. The dashboard

A dev-tool-style web dashboard (Vite + React + TypeScript) that reads the API and shows
runs, per-task status, token usage, a task list, and a dependency graph.

```bash
cd apps/dashboard
npm install
npm run dev        # http://localhost:5173 (proxies /api to the factory API)
```

Screenshots below are captured from a real local run (`FACTORY_PROVIDER=local`), not
mockups. The runs table lists every run with progress and token usage; opening a run
shows its task graph and the full task list, and a failed dependency renders the
transitive `blocked` cascade.

![Runs overview](docs/assets/dashboard-runs.png)
![Run detail with dependency graph](docs/assets/dashboard-run-detail.png)
![Blocked cascade after a failed task](docs/assets/dashboard-blocked-cascade.png)

## 9. Testing

```bash
cargo test              # all Rust crates (models, db, git, core, e2e)
cargo clippy --workspace --all-targets
cargo fmt --all -- --check

cd apps/dashboard
npm test                # vitest (graph layout and utilities)
npm run typecheck
npm run lint
```

The Rust suite covers the state machine and cascade propagation, plan validation
(unknown/cyclic dependencies, malformed and oversized plans), persistence round-trips,
and real git worktree creation/removal.

## 10. Roadmap

- Replanning for failed tasks with dependency rearrangement
- Task execution agents (auto-create worktree, run, commit, complete against criteria)
- Structured model usage tracking and run costing
- Plan review and approval before tasks are persisted
- Concurrency controls for multiple parallel agents
- Remote/virtual worktrees and cross-machine orchestration

## License

MIT. See [LICENSE](LICENSE).

## Copyright

© 2026 OmegaMc1331. See [LICENSE](LICENSE).