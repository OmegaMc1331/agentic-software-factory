# API reference

The factory serves a small local HTTP API for the dashboard. It is read-only: all
mutations happen through the CLI so that state transitions stay auditable and
conversational.

## Running the server

```bash
factory serve [--port 4321]
```

The server binds `127.0.0.1` on port `4321` by default. CORS is permissive: the
dashboard dev server proxies `/api` to this address, and the endpoint set is read-only,
so relaxing origins does not expose any write surface.

## Endpoints

### `GET /api/health`

```json
{ "status": "ok" }
```

### `GET /api/runs`

List of runs with per-state task counts.

```json
[
  {
    "id": 2,
    "objective": "Build a small HTTP server in Rust.",
    "status": "running",
    "plannerAgent": "codex",
    "createdAt": "2026-01-15T10:04:12Z",
    "counts": {
      "pending": 1,
      "ready": 1,
      "running": 2,
      "blocked": 0,
      "failed": 0,
      "completed": 1,
      "total": 5
    }
  }
]
```

### `GET /api/runs/:id`

Full detail for one run: the run row and every task.

```json
{
  "run": {
    "id": 2,
    "objective": "Build a small HTTP server in Rust.",
    "status": "running",
    "plannerAgent": "codex",
    "createdAt": "2026-01-15T10:04:12Z",
    "updatedAt": "2026-01-15T10:07:55Z"
  },
  "tasks": [
    {
      "id": 6,
      "runId": 2,
      "title": "Scaffold the crate",
      "objective": "Create a minimal cargo crate with a hello service.",
      "acceptanceCriteria": ["cargo build succeeds", "cargo test passes"],
      "state": "completed",
      "position": 1,
      "dependencies": [],
      "worktreePath": "C:/factory/.factory/worktrees/t6",
      "createdAt": "2026-01-15T10:04:12Z",
      "updatedAt": "2026-01-15T10:06:40Z"
    }
  ]
}
```

Unknown run ids return `404 { "error": "..." }`. Server failures return
`500 { "error": "..." }`.

### `GET /api/graph`

The brain-like "Agent Graph" view data: every configured agent and role, every stored
run, and every task, plus the edges that connect them. The dashboard lays these out in
agent → role → run → task lanes; the API is layout-agnostic and only describes the
topology.

```json
{
  "nodes": [
    { "id": "agent:codex", "kind": "agent", "label": "codex", "available": true },
    { "id": "role:planner", "kind": "role", "label": "planner" },
    { "id": "role:worker", "kind": "role", "label": "worker" },
    { "id": "run:2", "kind": "run", "label": "Build a small HTTP server in Rust.", "status": "running", "plannerRole": "planner", "plannerAgent": "codex", "createdAt": "2026-01-15T10:04:12Z" },
    { "id": "task:6", "kind": "task", "label": "Scaffold the crate", "state": "completed", "runId": 2, "position": 1, "dependencies": [], "worktreePath": "C:/factory/.factory/worktrees/t6" }
  ],
  "edges": [
    { "id": "e1", "source": "role:planner", "target": "agent:codex", "kind": "binds" },
    { "id": "e2", "source": "run:2", "target": "role:planner", "kind": "uses" },
    { "id": "e3", "source": "run:2", "target": "task:6", "kind": "contains" }
  ],
  "metadata": {
    "agentCount": 1,
    "roleCount": 2,
    "runCount": 1,
    "taskCount": 1,
    "edgeCount": 3
  }
}
```

Node `kind` is one of `agent`, `role`, `run`, `task`. Agent nodes carry `available`
(true when the agent's command resolves on `PATH`). Run nodes carry `status`,
`plannerRole`, `plannerAgent`, and `createdAt`. Task nodes carry `state`, `runId`,
`position`, `dependencies`, and `worktreePath`.

Edge `kind` is one of:

- `binds` — a role is assigned to an agent (`role` → `agent`)
- `uses` — a run is planned by a role/agent (`run` → `role`, or `run` → `agent`)
- `contains` — a run contains a task (`run` → `task`)
- `depends` — a task depends on another (`task` → `task`)

The response uses the same read-only, permissive-CORS surface as the other endpoints.