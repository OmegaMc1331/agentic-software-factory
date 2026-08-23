# Local API

`factory start` serves the dashboard and a semantic HTTP API on
`http://127.0.0.1:4321` (`--port` changes the port). The API binds to localhost
only and has no authentication — do not proxy or expose it. See
[Security](security.md).

Every route is an explicit Factory operation. There is no generic shell, command,
process-launch, or raw MCP JSON-RPC endpoint; the dashboard never executes agents
directly. Bodies and responses are JSON; errors return an appropriate status code
with `{"error": "<message>"}`. Unknown `/api/*` paths return `404`.

## Health

| Method | Route | Purpose |
| --- | --- | --- |
| GET | `/api/health` | Service health (`{"status": "ok"}`) |

## Workflows

| Method | Route | Purpose |
| --- | --- | --- |
| GET | `/api/runs` | Workflow summaries with task counts |
| POST | `/api/runs` | Create a workflow and plan it in the background |
| POST | `/api/runs/from-issue` | Import a GitHub Issue as a workflow |
| GET | `/api/runs/:id` | Workflow detail: tasks, attempts, sessions, stages, artifacts, integration branch |
| PUT | `/api/runs/:id/team` | Replace the workflow team before it starts |
| POST | `/api/runs/:id/start` | Validate and start a planned workflow; returns the team |
| POST | `/api/runs/:id/cancel` | Cancel the run's live operation |
| POST | `/api/runs/:id/recipe` | Save the run's configuration as a custom recipe |
| GET | `/api/runs/:id/artifacts` | Role artifacts persisted by the workflow |
| GET | `/api/runs/:id/delivery` | GitHub link, delivery state, and eligibility |
| GET | `/api/runs/:id/pr-preview` | Editable pull request preview and blockers |
| POST | `/api/runs/:id/pull-request` | Deliver: push `factory/run-<id>` and create/link the PR |

`POST /api/runs` accepts `{objective, team?, recipe?}`; `recipe` is either a
catalog id (`"security"`) or an inline recipe definition, validated and snapshotted
server-side. `POST /api/runs/from-issue` accepts `{issue, team?, recipe?}` where
`issue` is `#42`, `42`, or a full issue URL. Importing never executes anything;
nothing runs until `start`. See [Recipes](recipes.md) and [GitHub](github.md).

## Tasks

| Method | Route | Purpose |
| --- | --- | --- |
| POST | `/api/tasks/:id/retry` | Retry an eligible failed task within the attempt limit |
| GET | `/api/tasks/:id/artifacts` | Role artifacts produced by one task |
| GET | `/api/tasks/:id/routing-preview` | Informational preview of the likely routing choice |
| GET | `/api/tasks/:id/routing-decisions` | Durable audit trail of the task's routing decisions |
| PUT | `/api/tasks/:id/routing` | Pin (`{"agent": "codex"}`) or clear (`{"agent": null}`) the task's manual routing override |

The routing pin is validated against the task's role assignments up front and
re-checked at dispatch time; it never bypasses policies. See [Routing](routing.md).

## Roles

| Method | Route | Purpose |
| --- | --- | --- |
| GET | `/api/roles` | Role definitions and assignments |
| POST | `/api/roles` | Create a custom role |
| PUT | `/api/roles/:id` | Update a custom role definition |
| DELETE | `/api/roles/:id` | Delete an unused custom role |
| POST | `/api/roles/:id/assignments` | Assign an agent to a role (`{agent, preferred?}`) |
| DELETE | `/api/roles/:id/assignments/:agent` | Remove one role assignment |
| PUT | `/api/roles/:id/preferred` | Mark one assignment as preferred (`{agent}`) |
| PUT | `/api/roles/:id/policy` | Set or clear a role's policy preset (`{"preset": ... \| null}`) |
| GET | `/api/roles/:id/tools` | Effective discovered-tool decisions for a role |

Role creation accepts `{name, description, executionClass, instructions?, agents?,
preferredAgent?, policyPreset?, id?}`; `executionClass` is one of `planning`,
`advisory`, `execution`, `review`, `post_process`, and `policyPreset` is one of
`read_only`, `implementation`, `documentation`, `review`, `custom`. Built-in core
roles cannot be redefined or deleted — manage their assignments instead. See
[Roles](roles.md) and [Policies](policies.md).

## Recipes

| Method | Route | Purpose |
| --- | --- | --- |
| GET | `/api/recipes` | Built-in and custom workflow recipes |
| POST | `/api/recipes` | Create a custom recipe (validated atomically) |
| PUT | `/api/recipes/:id` | Update a custom recipe (built-ins are read-only) |
| DELETE | `/api/recipes/:id` | Delete a custom recipe |

Recipe bodies carry `name` (required) plus `description`, `roles`, `routing`
(`project_default` / `round_robin` / `performance`), `maxParallelTasks` (1–32),
`policyPresets`, `tools`, `plannerGuidance`, and `github.draft`. Recipes are
configuration only — never tasks or history. See [Recipes](recipes.md).

## Agents and sessions

| Method | Route | Purpose |
| --- | --- | --- |
| GET | `/api/agents` | Configured agents and executable status |
| GET | `/api/agents/:agent/tools` | Effective tool decisions for an agent's roles |
| GET | `/api/agents/:agent/sessions` | Recent persisted sessions for one agent |
| POST | `/api/agents/:agent/sessions` | Start an interactive console session (`{cols?, rows?}`) |
| GET | `/api/sessions/:id` | One session record |
| DELETE | `/api/sessions/:id` | Stop one live interactive session |
| GET | `/api/sessions/:id/stream` | SSE updates for one known automated session |
| GET | `/api/sessions/:id/terminal` | WebSocket for one live interactive session |

Interactive sessions launch only the selected configured agent in a Factory-owned
PTY; automated Planner/Worker/Reviewer sessions are non-interactive and stream
through SSE. Both transports close at a terminal state. See [Security](security.md).

## Graph and configuration

| Method | Route | Purpose |
| --- | --- | --- |
| GET | `/api/graph` | Agents, workflows, tasks, assignments, and semantic links |
| GET | `/api/graph/workspace` | Saved visual layout and custom topology |
| PUT | `/api/graph/workspace` | Validate and atomically save the graph workspace |
| GET | `/api/config` | The agent and role configuration |
| PUT | `/api/config` | Write a validated configuration atomically |

## GitHub

| Method | Route | Purpose |
| --- | --- | --- |
| GET | `/api/github/status` | `gh auth` status and the resolved GitHub remote |

Delivery operations live under the workflow routes above (`delivery`,
`pr-preview`, `pull-request`). See [GitHub](github.md).

## Performance

| Method | Route | Purpose |
| --- | --- | --- |
| GET | `/api/performance/agents` | Measured agent performance summaries |
| GET | `/api/performance/agents/:agent` | Full performance detail for one agent |

Both accept the query parameters `window` (`all`, `30d`, `7d`), `role`,
`operation`, and `language`. The endpoints are read-only; evaluation never
influences scheduling by itself. See [Evaluations](evaluations.md).

## External tools / MCP

| Method | Route | Purpose |
| --- | --- | --- |
| GET | `/api/mcp/servers` | Configured MCP servers and cached status |
| POST | `/api/mcp/servers` | Add a validated project-local MCP server |
| PUT | `/api/mcp/servers/:id` | Update one MCP server configuration |
| DELETE | `/api/mcp/servers/:id` | Remove a server and its discovery cache |
| POST | `/api/mcp/servers/:id/test` | Test MCP initialization and tool discovery |
| GET | `/api/mcp/servers/:id/tools` | Inspect discovered tool metadata and schemas |
| GET | `/api/mcp/invocations` | Compact external-tool invocation audit |
| GET | `/api/mcp/metrics` | External-tool operational metrics |

Task agents call tools through the scoped `factory mcp call` bridge, not through
this API. See [External Tools / MCP](mcp.md).
