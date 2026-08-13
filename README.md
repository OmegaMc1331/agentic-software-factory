# Agentic Software Factory

Agentic Software Factory is a local tool that orchestrates coding agents from an
interactive Agent Graph. You install and authenticate the agents (Codex, Claude Code,
OpenCode, Gemini CLI, or another CLI). The Factory plans workflows into real task DAGs,
runs tasks in isolated git worktrees, reviews evidence, and records each invocation in
local SQLite state.

![Factory Agent Graph](docs/assets/dashboard-network.png)

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

Use Agent Graph or **Settings → Agents** to add installed coding agents and assign the
**planner**, **worker**, and **reviewer** roles. Configuration lives in
`.factory/config.toml`; the dashboard writes it for you, and you can edit it directly:

```toml
[agents.codex]
command = "codex"
args = ["exec"]

[roles.planner]
agent = "codex"
```

The Factory never talks to model providers. If a required role is missing or its
executable is unavailable, the operation fails instead of selecting a fallback agent.

## Dashboard

Agent Graph is the primary operating interface:

- Create a Workflow from an objective, then inspect its real tasks and dependencies.
- Start, cancel, and retry supported workflow operations.
- Drag nodes; add agents, roles, groups, and notes; edit supported links; and use
  fit, center, zoom, or reset controls. Layout persists in `.factory/graph.json`.
- Select an agent to inspect real Planner, Worker, and Reviewer sessions in its console.

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
| POST   | `/api/tasks/:id/retry`        | Retry an eligible failed task                     |
| GET    | `/api/graph`                  | Agents, workflows, tasks, and semantic links     |
| GET    | `/api/graph/workspace`        | Saved visual layout and custom topology          |
| PUT    | `/api/graph/workspace`        | Validate and atomically save the graph workspace |
| GET    | `/api/agents/:agent/sessions` | Recent persisted sessions for one agent          |
| GET    | `/api/sessions/:id/stream`    | SSE updates for one known session                |
| GET    | `/api/config`                 | The agent and role configuration                 |
| PUT    | `/api/config`                 | Write a validated configuration atomically       |

## How it works

```text
Agent Graph → local API → Factory Core → Planner → task DAG
                                      → Worker → worktree → evidence
                                      → Reviewer → approve or retry
```

**Plan** asks the configured Planner for structured tasks and dependencies without
changing the repository. **Start** validates the roles, Git repository, and DAG, then
runs ready tasks sequentially. A task completes only after structured Reviewer
approval; process exit alone is not completion. Worker failures and change requests
retry up to three total attempts. Every invocation is an `AgentSession`.

All state lives in `.factory/`:

```text
.factory/
  db.sqlite3          runs, tasks, attempts, and agent sessions
  config.toml         agents and role assignments
  graph.json          saved positions, visual nodes, and custom links
  worktrees/t<id>/    one git worktree per task
```

Not implemented: parallel scheduling, interactive session stdin, plan editing,
automatic branch integration, or remote/cloud execution.

## Development

See [docs/development.md](docs/development.md) for source builds, tests, and embedded
dashboard release builds.

## License

MIT. See [LICENSE](LICENSE).
