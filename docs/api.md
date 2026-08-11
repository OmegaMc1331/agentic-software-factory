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
    "model": "gpt-4o-mini",
    "totalTokens": 3840,
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

Full detail for one run: the run row, every task, and token usage.

```json
{
  "run": {
    "id": 2,
    "objective": "Build a small HTTP server in Rust.",
    "status": "running",
    "model": "gpt-4o-mini",
    "promptTokens": 2048,
    "completionTokens": 1792,
    "totalTokens": 3840,
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
  ],
  "usage": {
    "promptTokens": 2048,
    "completionTokens": 1792,
    "totalTokens": 3840
  }
}
```

Unknown run ids return `404 { "error": "..." }`. Server failures return
`500 { "error": "..." }`.