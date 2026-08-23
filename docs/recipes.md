# Workflow Recipes

A **Recipe** is a reusable workflow *configuration*: which roles a workflow
prefers, its routing and concurrency defaults, the policy presets it
references, and bounded guidance for the Planner.

A recipe defines **how** Factory should approach a workflow. It does **not**
define the task DAG:

```text
Recipe → workflow configuration → Planner → generated adaptive DAG
```

The Planner stays responsible for creating the actual plan from the
objective. Recipes never hardcode tasks, task ids, attempts, sessions, Git
commits, or generated DAGs.

## Recipe vs Plan

| | Recipe | Plan |
|---|---|---|
| What it is | Reusable configuration | A concrete task DAG for one objective |
| Created by | You (Settings, file, Save as Recipe) | The Planner agent |
| Contains | roles, routing, concurrency, policy presets, guidance | tasks, dependencies, acceptance criteria |
| Reused | Across any number of workflows | Exactly one workflow |
| Immutable snapshot | Stored per workflow at creation | Revised by edits/replans |

## Built-in recipes

The catalog ships six built-ins. They are read-only; duplicate one to
customize it.

| Recipe | Roles | Parallelism | Notes |
|---|---|---|---|
| **Standard Development** (`standard`) | planner, worker, reviewer | 4 | Balanced default. |
| **Thorough Development** (`thorough`) | planner, architect, worker, test_engineer, reviewer, documentation_writer | project default | Specialized roles stay optional; guidance asks the Planner to use them only when they materially help. |
| **Security Sensitive** (`security`) | planner, architect, worker, test_engineer, security_auditor, reviewer | 2 | References the stricter existing policy presets (`read_only` for planner/architect/security_auditor/reviewer, `implementation` for worker/test_engineer). |
| **Bug Fix** (`bugfix`) | planner, worker, test_engineer, reviewer | project default | Guidance favors a compact diagnosis / fix / regression-test workflow. |
| **Documentation** (`docs`) | planner, documentation_writer, reviewer | 2 | Guidance keeps every task inside documentation. |
| **Custom** (`custom`) | *(none)* | project default | The no-prefill placeholder for fully manual setup. |

## Custom recipes

Custom recipes live in `.factory/recipes.toml` (created lazily on first save
— existing projects never need to re-run `factory init`):

```toml
[recipes.backend_feature]
name = "Backend Feature"
description = "Backend implementation with architecture and verification."
roles = ["planner", "architect", "worker", "test_engineer", "reviewer"]
routing = "performance"
max_parallel_tasks = 3

planner_guidance = """
Prefer a compact dependency-aware plan.
Use Architect only when design work is materially useful.
Add verification for behavioral changes.
"""

[recipes.backend_feature.policy_presets]
worker = "implementation"
reviewer = "review"

[recipes.backend_feature.github]
draft = true
```

Recipes are configuration, not workflow history. The API and Settings UI are
the supported editors; the file is validated and written atomically on every
change, so an invalid recipe never lands on disk.

## Planner guidance

`planner_guidance` is bounded planning context (≤ 4000 characters). The
Planner receives it alongside the objective and the available roles:

```text
WORKFLOW OBJECTIVE
AVAILABLE ROLES
RECIPE
RECIPE GUIDANCE   (planning context only)
```

Guidance can never override role definitions, task-operation compatibility,
the Policy Engine, Factory invariants, repository boundaries, or the allowed
workflow roles. Current user replan instructions always take precedence over
generic recipe guidance, and replanning never mutates the original recipe.

## Role selection

A recipe may only use roles that exist in the project's Factory
configuration, and every selected role must resolve to at least one assigned
agent. If a recipe requires `security_auditor` but no agent is assigned to
that role, workflow creation fails with a clear error. Roles are never
silently replaced (e.g. by Worker), and global role assignments are never
mutated on a recipe's behalf.

Selected roles are **optional** in the plan: the Planner stays adaptive.
The Security Sensitive recipe plus a trivial CSS change may legitimately
produce just Worker → built-in review, without Architect, Security Auditor,
or Test Engineer tasks. Specialized roles appear only when materially
useful.

## Routing

A recipe selects one of:

- `project_default` — keep whatever `[routing] mode` the project configures;
- `round_robin` — pin the run to deterministic capacity-aware routing;
- `performance` — pin the run to performance-aware routing.

The recipe only selects the mode/default. The existing Intelligent Agent
Routing engine still owns candidate filtering, performance scoring,
capacity, policies, manual overrides, and the actual routing decisions.
Recipes can never select `manual` routing — manual is per-task pinning.

## Concurrency

`max_parallel_tasks` (1–32) is a per-run preference for the parallel
scheduler's width bound; it is validated against the same safety limits as
`[runtime] max_parallel_tasks` and displayed in the Workflow Inspector.
There is no separate concurrency system: agent-level `max_concurrency` and
the integration lane behave exactly as before.

## Policies

Recipes may reference existing policy presets
(`read_only`, `implementation`, `documentation`, `review`, `custom`) per
role. A recipe can only **narrow** behavior: the preset applies as one extra
restriction layer on top of the Policy Engine's resolution (baseline
invariants → role scope → agent scope). Filesystem allow lists intersect,
deny lists union, and a recipe can never elevate privileges beyond what the
Policy Engine already permits. The Policy Engine remains authoritative.

Recipes may also declare `tools = ["docs.*", "postgres.inspect_schema"]`.
This is a selection layer over already permitted MCP tools: it intersects with
the effective role/agent tool policy and can never grant access. Tool denies and
the exact-allow requirement for write, destructive, or unknown tools still apply.
See [External Tools / MCP](mcp.md#permissions).

## Workflow creation with recipes

`+ Workflow` in the Agent Graph offers a recipe picker on both tabs
(*From objective* and *From GitHub Issue*). Selecting a recipe pre-fills the
team and defaults; you can still customize the team before planning:

```text
Objective  [ Add secure password reset      ]
Recipe     [ Security Sensitive             ]
Team
Planner          codex
Worker           Automatic
Reviewer         Automatic
Security Auditor Automatic
```

GitHub Issue workflows use the same catalog — Issue #42 + the Bug Fix recipe
flows Issue → Recipe → Planner → editable DAG → execution. There is no
separate GitHub recipe system.

## Workflow recipe snapshot

When a workflow is created from a recipe, an **immutable snapshot** of the
applied configuration is persisted with the run: recipe id and name,
selected roles/team defaults, routing mode, concurrency preference, Planner
guidance, and policy preset references.

Editing or deleting the recipe afterwards never mutates existing workflows.
Historical runs stay reproducible and auditable; the Workflow Inspector
shows the snapshot's applied settings.

## Save as Recipe

Any planned workflow (planning / planned / blocked) offers **Save as
Recipe**. It saves the reusable configuration only — roles, routing,
concurrency, policy presets, and Planner guidance. It never saves generated
tasks, DAG edges, task ids, attempts, AgentSessions, RoleArtifacts, or
Git/integration state. It is configuration capture, not "Clone Workflow".

## API

```text
GET    /api/recipes          list built-in + custom recipes
POST   /api/recipes          create a custom recipe (validated, atomic)
PUT    /api/recipes/:id      update a custom recipe (built-ins are read-only)
DELETE /api/recipes/:id      delete a custom recipe
POST   /api/runs/:id/recipe  save a run's configuration as a custom recipe
```

Workflow creation accepts either a recipe id or a resolved recipe definition
(inline object), which is validated and snapshotted server-side:

```json
POST /api/runs
{ "objective": "Add secure password reset", "recipe": "security" }

POST /api/runs
{ "objective": "Add secure password reset",
  "recipe": { "name": "Inline Run", "roles": ["planner", "worker", "reviewer"],
              "routing": "performance", "maxParallelTasks": 2 } }

POST /api/runs/from-issue
{ "issue": "#42", "recipe": "bugfix" }
```

There is no arbitrary TOML editing through the API.

## Validation

Recipes are validated atomically before anything is written or applied:

- stable recipe id (lowercase slug; ids are immutable after creation);
- non-empty name (≤ 80 characters), bounded description;
- known roles, no duplicates, planner included (custom recipes);
- valid routing mode (`project_default` / `round_robin` / `performance`);
- concurrency within 1–32;
- known policy preset references for known roles;
- bounded Planner guidance (≤ 4000 characters).

## Backward compatibility

Existing projects and workflows remain fully valid without re-running
`factory init` or touching `.factory`: the recipe catalog is built-ins plus
an optional `recipes.toml`, and workflows without recipes display **No
Recipe** in the inspector and behave exactly as before.

## What recipes deliberately are not

- No fixed DAG templates, YAML pipeline language, or Actions-like scripting —
  the Planner generates every DAG.
- No recipe marketplace, cloud sync, or automatic LLM recipe selection.
- No permission escalation — the Policy Engine stays authoritative.
- No recipe-specific execution engine — recipes reuse the existing Factory
  runtime end to end.
