# CLI reference

The `factory` binary is the primary interface for state changes. Every command resolves
the factory root from the current working directory and requires initialized state
(`.factory/db.sqlite3`) except `init` itself.

## Global behavior

- Configuration is read from the environment, then from `.env` if present
  (`dotenvy`). See [`.env.example`](../.env.example).
- Exit codes: `0` on success, non-zero on failure with a message on stderr.
- Errors are actionable ("no factory state found here; run `factory init` first").

## Commands

### `factory init [--force]`

Creates `.factory/db.sqlite3` in the current directory. `--force` re-creates the schema
(previous data is dropped) - use sparingly.

```bash
$ factory init
Initialized factory state at D:\factory\.factory
Database: D:\factory\.factory\db.sqlite3
Provider: local-planner
```

When `FACTORY_API_KEY` is exported the provider line shows the real model instead.

### `factory run "<objective>"`

Plans the objective through the configured provider, persists the run and its tasks,
and prints the plan. Requires `FACTORY_API_KEY` (with the OpenAI-compatible provider) or
`FACTORY_PROVIDER=local`.

```bash
$ factory run "Add a /health endpoint that returns JSON"
Run #3 planned (gpt-4o-mini, 5 tasks)
  #14   ready     Define the response contract [ ]
  #15   pending   Add the /health route [ #14 ]
  #16   pending   Add a unit test for the handler [ #15 ]
  #17   pending   Wire the route into the server [ #16 ]
  #18   pending   Verify build and test suite [ #17 ]
```

### `factory status`

Prints the latest run summary and its tasks:

```bash
Factory: D:\factory\.factory
Latest run: #3 (running)
  created 2026-01-15T10:04:12Z  model gpt-4o-mini  tokens 3840
  tasks: 0 pending, 1 ready, 1 running, 0 blocked, 0 failed, 3 completed
  #14   completed  Define the response contract
  #15   running    Add the /health route [ #14 ]
  ...
```

### `factory tasks [--run <id>]`

Lists tasks of the latest run by default, or of a specific run with `--run`.

### `factory inspect <task-id>`

Shows full task detail: state, position, objective, dependencies, worktree path,
timestamps, and acceptance criteria.

### `factory mark <task-id> <state>`

The only way to move a task. States: `pending`, `ready`, `running`, `blocked`,
`failed`, `completed`. `blocked` is normally derived by the cascade but the table also
permits marking it directly. All transitions are validated against the table; when a
completion, failure, or block happens, the cascade is logged:

```bash
$ factory mark 15 completed
task #15: running -> completed
  propagated: task #16 updated
```

Invalid moves are rejected with the reason:

```bash
$ factory mark 16 completed
Error: invalid state transition: pending -> completed
```

### `factory worktree create <task-id>`

Creates branch `factory/t<task-id>` and worktree `.factory/worktrees/t<task-id>` for a
`ready` or `running` task.

```bash
$ factory worktree create 15
created worktree at D:\factory\.factory\worktrees\t15
```

### `factory worktree remove <task-id>`

Removes the worktree after archiving; refuses if the worktree has uncommitted changes.

### `factory worktree status`

Lists every worktree of the repository with its path and branch.

### `factory serve [--port 4321]`

Starts the local HTTP API for the dashboard. See [API reference](api.md).