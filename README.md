# Agentic Software Factory

Agentic Software Factory is a local tool that orchestrates coding agents from an
interactive Agent Graph. You install and authenticate the agents (Codex, Claude Code,
OpenCode, Gemini CLI, or another CLI). The Factory plans workflows into real task DAGs,
runs tasks in isolated git worktrees, reviews evidence, and records each invocation in
local SQLite state.

The runtime is **role-aware**: a plan's tasks carry a semantic operation
(`advisory`, `implement`, `verify`, `review`, `post_process`), matched to each role's
execution class. Researchers and Architects produce persisted artifacts consumed by
implementation tasks; a Test Engineer verifies; specialized review roles such as the
Security Auditor evaluate the diff and can route `request_changes` back into bounded
implementation rework; a Documentation Writer runs last. Custom roles work without
source changes — for example a `performance_analyst` with `execution_class = "review"`
behaves like any other specialized reviewer.

Simple workflows stay simple: a small fix still plans as Worker → Reviewer.

![Factory Agent Graph](docs/assets/dashboard-network.png)

See [docs/roles.md](docs/roles.md) for execution classes, operations, artifacts,
specialized reviews, and custom roles.

## Install

Ready-made binaries are available for Windows (x86_64), Linux (x86_64), and macOS
(Apple Silicon and Intel). No Rust, Node, or administrator rights are required.

### Windows

```powershell
irm https://raw.githubusercontent.com/OmegaMc1331/agentic-software-factory/main/install.ps1 | iex
```

### Linux / macOS

```bash
curl -fsSL https://raw.githubusercontent.com/OmegaMc1331/agentic-software-factory/main/install.sh | sh
```

Then check the install:

```bash
factory --version
factory init
factory start
```

If `factory` is not found, open a new terminal. Windows adds the install location to
your user PATH; macOS and Linux print the line to add to your shell profile. Running the
installer again replaces an older version with the latest release.

### Uninstall

Remove the binary and any PATH entry you added:

- Windows: delete `%LOCALAPPDATA%\Programs\AgenticSoftwareFactory`, then remove its
  `bin` directory from your user PATH under **System Properties → Environment Variables**.
- macOS / Linux: `rm ~/.local/bin/factory`.

Your project-local `.factory/` state is not removed.

## Quick start

```bash
cd my-project
factory init
factory start
```

`factory init` creates `.factory/` with a default configuration. `factory start` serves
the local API and dashboard, waits until the server is ready, then opens
`http://127.0.0.1:4321`.

In Agent Graph, configure the Planner, Worker, and Reviewer agents. Create a Workflow,
review its task plan, then select **Start**. The Rust Factory process owns execution, so
closing the browser doesn't stop active work.

## Configure agents

Use Agent Graph or **Settings → Agents** to add installed coding agents and assign
roles. Configuration lives in `.factory/config.toml`; the dashboard writes it for you,
and you can edit it directly:

```toml
[agents.codex]
kind = "codex"
command = "codex"
args = ["exec"]

[[role_assignments]]
role = "planner"
agent = "codex"
preferred = true

[[role_assignments]]
role = "worker"
agent = "opencode"
```

Known coding agents are configured with their standard non-interactive workflow
invocation. Custom CLIs can choose whether Factory passes the mission through stdin or
as one process argument; `{mission}` can set that argument's exact position.

The Factory never talks to model providers. If a required role is missing or its
executable or workflow invocation is unavailable, the operation fails instead of
selecting a fallback agent.

## Roles

Roles describe responsibilities; agents are the CLIs that perform them. Eight core
roles are built in (Planner, Worker, Reviewer, Architect, Researcher, Test Engineer,
Security Auditor, Documentation Writer), a role can be filled by several agents at
once, and one agent can hold several roles — no `worker2`-style duplicates. You can
create custom roles with their own instructions from Agent Graph, and each workflow
selects the team of roles and agents that may participate. See
[docs/roles.md](docs/roles.md) for the full guide.

## Dashboard

Agent Graph is the primary operating interface:

- Create a Workflow from an objective, then inspect its real tasks and dependencies.
- Start, cancel, and retry supported workflow operations.
- Drag nodes; add agents, roles, groups, and notes; edit supported links; and use
  fit, center, zoom, or reset controls. Layout persists in `.factory/graph.json`.
- Select an agent to inspect real workflow sessions or explicitly start its interactive
  console.

The Runs and Settings views remain available for focused inspection and configuration.

![Factory Agent Console](docs/assets/dashboard-agent-console.png)

The local API is bound to `127.0.0.1`:

| Method | Route                         | Description                                      |
| ------ | ----------------------------- | ------------------------------------------------ |
| GET    | `/api/health`                 | Service health                                   |
| GET    | `/api/runs`                   | Workflow summaries and task counts               |
| POST   | `/api/runs`                   | Create and asynchronously plan a workflow        |
| GET    | `/api/runs/:id`               | Workflow, tasks, attempts, and sessions           |
| POST   | `/api/runs/:id/start`         | Validate and start a planned workflow             |
| POST   | `/api/runs/:id/cancel`        | Cancel a live workflow operation                  |
| PUT    | `/api/runs/:id/team`          | Replace the workflow team before it starts        |
| POST   | `/api/tasks/:id/retry`        | Retry an eligible failed task                     |
| GET    | `/api/roles`                  | Role definitions and assignments                  |
| POST   | `/api/roles`                  | Create a custom role                              |
| PUT    | `/api/roles/:id`              | Update a custom role definition                   |
| DELETE | `/api/roles/:id`              | Delete an unused custom role                      |
| POST   | `/api/roles/:id/assignments`  | Assign an agent to a role                         |
| GET    | `/api/graph`                  | Agents, workflows, tasks, and semantic links     |
| GET    | `/api/graph/workspace`        | Saved visual layout and custom topology          |
| PUT    | `/api/graph/workspace`        | Validate and atomically save the graph workspace |
| GET    | `/api/agents/:agent/sessions` | Recent persisted sessions for one agent          |
| POST   | `/api/agents/:agent/sessions` | Start an interactive session for that agent       |
| GET    | `/api/sessions/:id/stream`    | SSE updates for one known session                |
| GET    | `/api/sessions/:id/terminal`  | WebSocket for one live interactive session       |
| GET    | `/api/config`                 | The agent and role configuration                 |
| PUT    | `/api/config`                 | Write a validated configuration atomically       |

## How it works

```text
Agent Graph → local API → Factory Core → Planner → task DAG
                                      → Worker → worktree → evidence
                                      → Reviewer → approve or retry
```

**Plan** asks the selected Planner for structured tasks and dependencies without
changing the repository; tasks may target specific roles from the workflow's team.
**Start** validates the team, Git repository, and DAG, then
runs ready tasks sequentially. A task completes only after structured Reviewer
approval; process exit alone is not completion. Worker failures and change requests
retry up to three total attempts. Every invocation is an `AgentSession` that records
the role and agent that ran.

All state lives in `.factory/`:

```text
.factory/
  db.sqlite3          runs, tasks, attempts, and agent sessions
  config.toml         agents and role assignments
  graph.json          saved positions, visual nodes, and custom links
  worktrees/t<id>/    one git worktree per task
```

Not implemented: parallel scheduling, plan editing, automatic branch integration, or
remote/cloud execution.

## Development

See [docs/development.md](docs/development.md) for source builds, tests, and embedded
dashboard release builds.

## License

MIT. See [LICENSE](LICENSE).
