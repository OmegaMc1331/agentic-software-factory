# Agentic Software Factory

Agentic Software Factory is a local tool that orchestrates coding agents through
structured tasks and isolated git worktrees. You install and authenticate the agents
(Codex, Claude Code, OpenCode, Gemini CLI, or any custom CLI); the factory plans a run
into tasks with dependencies and acceptance criteria, gives each task its own git
worktree, and records every step in a local SQLite database.

![Factory network](docs/assets/dashboard-network.png)

## Install

Ready-made binaries for Windows (x86_64), Linux (x86_64), and macOS
(Apple Silicon and Intel). No Rust, Node, or administrator rights are required.

### Windows

From PowerShell, run:

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

If `factory` is not found, open a new terminal (Windows adds the install location to
your user PATH; macOS/Linux prints the one line to add to your shell profile).

Re-running the same command installs the latest release and replaces an older version.

### Uninstall

Remove the binary (and its PATH entry if you added one manually):

- Windows: delete `%LOCALAPPDATA%\Programs\AgenticSoftwareFactory`, then remove that
  `bin` directory from your user PATH (`System Properties → Environment Variables`).
- macOS / Linux: `rm ~/.local/bin/factory`.

Your project-local `.factory/` state is never touched.

## Quick start

```bash
cd my-project
factory init
factory start
```

`factory init` creates `.factory/` with a default configuration and works on a machine
with no coding agent installed. `factory start` starts one process that serves the API
and the dashboard, waits until the server is ready, then opens your browser at
`http://127.0.0.1:4321`.

## Configure agents

After `factory start`, open **Settings → Agents** in the dashboard to add the coding
agents you have installed (name, command, arguments) and assign them to the
**planner**, **worker**, and **reviewer** roles. There is no CLI configuration step.

Configuration lives in `.factory/config.toml`; the dashboard writes it for you, and you
can also edit it by hand:

```toml
[agents.codex]
command = "codex"
args = ["exec"]

[roles.planner]
agent = "codex"
```

The factory never talks to model providers. If a role has no agent, or the agent's
executable is not on your PATH, the factory fails with a clear message instead of using
a fallback.

## Dashboard

`factory start` serves the dashboard at `http://127.0.0.1:4321`:

- **Runs** — every run with its status and progress; opening a run shows its task list.
- **Agent Graph** — the whole factory as a network of agents, roles, and runs.
- **Settings** — add, edit, and remove agents, test executable availability, and assign
  the planner/worker/reviewer roles.

![Runs overview](docs/assets/dashboard-runs.png)
![Run detail](docs/assets/dashboard-run-detail.png)
![Blocked cascade](docs/assets/dashboard-blocked-cascade.png)

The local API is bound to `127.0.0.1`:

| Method | Route            | Description                                  |
| ------ | ---------------- | -------------------------------------------- |
| GET    | `/api/health`    | Service health                               |
| GET    | `/api/runs`      | Runs with per-state task counts              |
| GET    | `/api/runs/:id`  | One run and its tasks                        |
| GET    | `/api/graph`     | Agents, roles, runs, and tasks as a network  |
| GET    | `/api/agents`    | Configured agents with executable status     |
| GET    | `/api/config`    | The agent/role configuration                 |
| PUT    | `/api/config`    | Write a validated configuration (atomic)     |

## How it works

```text
CLI / Dashboard
      │
      ▼
Factory core ── planner runs a coding agent as a subprocess
      │
      ├── SQLite (runs, tasks, sessions)
      └── git worktrees (.factory/worktrees/t<task-id>)
```

`factory run "<objective>"` resolves the planner role from `.factory/config.toml`, asks
that agent for a JSON plan, validates it, and persists the run and its tasks. Tasks move
through a strict state machine (`pending → ready → running → completed`, or
`failed`/`blocked`) from the dashboard; each task's work is done in its own git worktree
so parallel agents never collide. Every agent invocation (command, exit code, output) is
recorded in SQLite.

All state lives in `.factory/`:

```text
.factory/
  db.sqlite3          SQLite database (runs, tasks, agent sessions)
  config.toml         agents and role assignment
  worktrees/t<id>/    one git worktree per task
```

## Current status

Working today: run creation through a configured planner, task state machine with
cascade propagation, run-status reconciliation, git worktrees per task, agent session
recording, versioned SQLite migrations, and a dashboard with runs, an agent network, and
agent/role configuration.

Not yet implemented: an autonomous worker loop, review execution, parallel agents,
automatic merging, and anything involving remote/cloud execution.

## Development

Contributors build the Rust workspace and the dashboard from source — see
[docs/development.md](docs/development.md) for the full workflow, including how the
dashboard is embedded into release binaries.

## License

MIT. See [LICENSE](LICENSE).