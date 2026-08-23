# Designing Effective Roles

How to design specialized roles that improve Agentic Software Factory workflows.

This guide teaches **design**: when to create a custom role, how to scope it, how to
write its description and instructions, and how to keep it useful over time. The
technical reference for the role system — execution classes, operations, artifacts,
configuration format, runtime behavior — lives in [roles.md](roles.md).

## Core principle

**A role is not a persona.**

Bad:

```text
You are a world-class senior expert developer with 20 years of experience.
Always do your best and write perfect code.
```

Good:

```text
Own database schema and migration design. Identify migration risks, compatibility
constraints and rollback requirements. Produce implementation-ready decisions for
downstream execution tasks.
```

A role should encode:

```text
responsibility + scope + constraints + expected output
```

not personality. Factory's runtime does not branch on role names or personas: it
dispatches every task by its **operation** (`advisory`, `implement`, `verify`,
`review`, `post_process`), which must be compatible with the role's **execution
class**. A well-designed role gives the Planner a clear reason to use it, gives the
agent a bounded job, and gives the Reviewer something objective to check.

## 1. When should I create a custom role?

Create a role when a responsibility:

- appears repeatedly across workflows;
- has a clear engineering boundary (distinct from generic implementation);
- benefits from specialized instructions;
- produces a distinct artifact or review;
- can be independently assigned to tasks.

Good candidates:

```text
Database Engineer        API Designer             Performance Analyst
Accessibility Reviewer   Migration Specialist     Frontend Engineer
Backend Engineer         Release Engineer         Localization Reviewer
Data Engineer
```

Bad candidates:

```text
Genius Developer         Senior Coder             Best Programmer
Fix Everything Expert    React Tailwind TypeScript Node Docker PostgreSQL Expert
```

Persona roles fail because they have no scope, no output contract, and no decision
rule for the Planner: there is no objective way to know when a task needs the
"Genius Developer" instead of the Worker. Mega-roles fail because they overlap
every other role, duplicate work between agents, and cannot be reviewed as a unit —
their description restates what the task objective should already say.

## 2. When NOT to create another role

A new role is unnecessary when:

- the Worker's instructions already cover the responsibility;
- the distinction is only a technology preference (`TypeScript Engineer` vs
  `Python Engineer` for general implementation tasks);
- it would be used for one trivial task — put that in the task objective instead;
- it overlaps almost completely with an existing role;
- it exists only to use another model or agent.

The last point matters most:

```text
Agents and roles are different concepts.
```

Do not create:

```text
worker_codex    worker_claude    worker_qwen
```

Assign several agents to the one `worker` role instead
(`[[role_assignments]]` entries). Factory routes attempts across the assigned pool;
duplicating role definitions per agent breaks routing history, performance
breakdowns, and policy scopes, which are all keyed by role. See
[Roles — Roles vs agents](roles.md#roles-vs-agents).

## 3. The role design method

Six steps, in order.

### Step 1 — Define ownership

Finish the sentence: **"This role owns …"**

```text
This role owns relational schema design, migrations, indexes, constraints and
data-integrity implications.
```

If you cannot write this sentence precisely, the role is probably too vague — go
back to the drawing board before writing any instructions.

### Step 2 — Define boundaries

Finish the sentence: **"This role does not own …"**

```text
Does not redesign unrelated backend APIs or frontend behavior.
```

Narrow boundaries reduce duplicated work between agents. Every responsibility a
role claims is work another role may also do; explicit exclusions keep the DAG's
tasks disjoint and reviewable.

### Step 3 — Choose the execution class

Every role has exactly one execution class; it determines which operations the role
may perform ([roles.md](roles.md#execution-classes-and-operations)):

```text
planning     -> planning        (the Planner itself; never a scheduled task)
advisory     -> advisory        (produce context for later tasks; zero file changes is success)
execution    -> implement, verify
review       -> review          (approve / request_changes with findings)
post_process -> post_process, implement (where explicitly allowed)
```

Choose by the **behavior and output you need**, not by the role's name:

| Role | Class | Why |
| --- | --- | --- |
| Database Architect | `advisory` | Produces design decisions an implementation task consumes. |
| Database Engineer | `execution` | Changes schema/migrations in a worktree and verifies them. |
| Performance Reviewer | `review` | Evaluates a diff and returns findings. |
| Release Notes Writer | `post_process` | Updates documentation after implementation. |

A role named "Analyst" that must edit files needs `execution`, not `advisory`; a
role named "Engineer" that must only recommend needs `advisory`. Declared operations
that contradict the class are rejected and the plan is repaired — Factory never
reinterprets role semantics silently.

### Step 4 — Define inputs

What should this role inspect? Typical inputs Factory actually provides:

- the task objective and acceptance criteria;
- the run objective;
- repository context (bounded index of paths, symbols, related tests);
- artifacts of the task's **direct dependencies** (research, architecture,
  reviews);
- for reviews: the implementation diff (bounded) and the producer's evidence;
- for verification: test commands and their observed results.

Do not demand data Factory cannot provide (full workflow history, other tasks'
artifacts, provider metrics). Upstream context follows the DAG and is capped; see
[roles.md — Upstream context and artifacts](roles.md#upstream-context-and-artifacts).

### Step 5 — Define the output contract

Every good role has an observable result. The runtime declares one contract per
operation; design role instructions so they fill it well:

```text
advisory      decisions, affected components, interfaces, constraints,
              migration considerations           → persisted role artifact
review        approve | request_changes + concrete findings (severity,
              summary, evidence)                 → persisted review artifact
execution     code changes, tests, commands run, evidence reported as observed
verify        test commands and results exactly as observed
post_process  documentation changes consistent with what was implemented
```

The role should make downstream work easier: an implementation task consuming an
advisory artifact should need no further design decisions; a reviewer's findings
must be specific enough to act on (`src/auth.rs:9`, not "somewhere in auth").

### Step 6 — Define constraints

Examples that hold up in practice:

```text
- do not expand scope beyond the task objective;
- do not modify unrelated files;
- do not invent test results, coverage, or benchmark numbers;
- do not duplicate another role's responsibility;
- prefer existing project conventions over new patterns;
- escalate uncertainty explicitly instead of guessing.
```

**Permissions do not belong in role instructions.** What a role may read, write,
execute, or call is decided by the **Policy Engine** (filesystem scopes, command
allow/deny lists, environment filtering, Git limits, external-tool allows).
Instructions that say "you may write anywhere" or "never touch the network" are
advice, not enforcement — configure a policy instead. See
[Policies](policies.md) and [section 9](#9-policies-vs-role-instructions).

## 4. Writing the role description

The `description` field is load-bearing: the Planner receives a **concise catalog**
of the workflow team's roles — id, name, execution class, description — and nothing
else from the role definition. The description must therefore answer:

**"When should the Planner use this role?"**

Good:

```text
Designs and validates relational database schemas, migrations, indexes and
integrity constraints.
```

Bad:

```text
Database expert.
```

Bad:

```text
Helps with databases and backend tasks.
```

The first is discriminative: the Planner can tell a schema task from a middleware
task. The second carries no decision rule; the third overlaps Worker and Backend
Engineer. Keep descriptions to one or two concise sentences and free of persona
language.

## 5. Writing role instructions

Reusable template:

```text
Purpose:
...

Responsibilities:
- ...
- ...
- ...

Scope:
- ...

Inputs to consider:
- ...

Constraints:
- ...
- ...

Expected output:
- ...
- ...

Decision rules:
- ...
```

Instructions are injected into the mission next to Factory's own constraints, and
Factory's execution constraints (worktree isolation, `.factory` state protection)
always take precedence over role text.

Keep instructions concise enough to remain useful. Validation allows up to
16,384 characters; that is a ceiling, not a target. Overly long instructions:

- consume context the agent needs for the actual repository and task;
- create internal conflicts the agent resolves arbitrarily;
- reduce compliance with the few rules that matter;
- duplicate Factory's runtime rules (retry limits, output contracts, invariants),
  which Factory enforces regardless.

A strong instruction block usually fits in 15–40 lines.

## 6. Role quality checklist

A strong role answers **yes** to most of these:

- [ ] Does it own a distinct responsibility?
- [ ] Can the Planner know when to use it (from the description alone)?
- [ ] Is its scope narrower than Worker?
- [ ] Does it have a clear, behavior-based execution class?
- [ ] Does it produce useful output for another task or reviewer?
- [ ] Does it avoid duplicating an existing role?
- [ ] Are its constraints explicit?
- [ ] Is its description concise?
- [ ] Can success/failure be reviewed objectively?
- [ ] Would this role still make sense if another coding agent performed it?

The last question is essential: roles must stay **agent-independent**. Roles
describe responsibilities; agents are the executables that perform them. If a role
only makes sense for one specific CLI, it encodes an agent, not a responsibility.

## 7. Role granularity

```text
Too broad:   Software Engineer
Good:        Database Engineer
Too narrow:  PostgreSQL JSONB Index Optimizer For User Search
```

- **Too broad** — indistinguishable from Worker; the Planner has no reason to
  select it.
- **Good** — a reusable engineering responsibility with a clear boundary and
  output.
- **Too narrow** — that is task granularity, not role granularity. Put it in a
  task objective ("optimize the JSONB index used by user search") and let a
  Database Engineer task carry it.

Roles should represent reusable engineering responsibilities that recur across many
tasks and workflows, not individual tasks.

## 8. Role composition

Specialized roles become valuable when composed in a DAG. The Planner places each
role where its input/output contract fits:

```text
Feature:                       Database migration:
Architect                      Database Engineer
   ↓                              ↓
Backend Engineer               Migration Reviewer
   ↓                              ↓
Test Engineer                  Reviewer
   ↓
Reviewer

Security-sensitive API:        Performance work:
Architect                      Worker
   ↓                              ↓
Backend Engineer               Performance Analyst
   ↓                              ↓
Test Engineer                  Reviewer
   ↓
Security Auditor
   ↓
Reviewer
```

Do not read these as templates: **not every workflow needs every role**. The
Planner is explicitly instructed to keep plans proportional to the objective — a
trivial fix planned with a fully staffed team should still come out as
Worker → built-in review. Composition is something you enable by selecting roles
into the workflow team (or a recipe), and the Planner decides where they materially
help.

## 9. Policies vs role instructions

One mental model for the whole system:

| Concept | Question it answers |
| --- | --- |
| **Role instructions** | What should this role do? |
| **Policy** | What is this role allowed to do? |
| **Agent** | Which executable actually performs the role? |
| **Recipe** | Which roles, policies, routing, and concurrency should this kind of workflow prefer? |
| **Task** | What specific work must happen now? |

Instructions are prompt text; they never grant permissions. Enforcement lives in
the Policy Engine and Factory's safety invariants, applied before launch and
against recorded evidence. When you catch yourself writing "you are not allowed
to …" in role instructions about files, commands, or the network, move it into a
policy and keep the instruction about engineering behavior. See
[Policies](policies.md).

## 10. Example roles

Five complete, ready-to-adapt examples. Each is a `[roles.<slug>]` table in
`.factory/config.toml` plus an assignment and a policy preset.

### Database Engineer

- **id:** `database_engineer`
- **name:** Database Engineer
- **description:** Designs and validates relational schema changes, migrations,
  indexes and integrity constraints.
- **execution_class:** `execution`
- **recommended policy preset:** `implementation`
- **Planner should use it when** the task changes schemas, migrations, indexes,
  constraints, or data preservation.
- **Planner should NOT use it when** the task only reads data through existing
  APIs, or when only an advisory schema decision is needed (use an advisory role
  or Architect instead).

```toml
[roles.database_engineer]
name = "Database Engineer"
description = "Designs and validates relational schema changes, migrations, indexes and integrity constraints."
execution_class = "execution"
instructions = """
Purpose: own relational schema and migration work for assigned tasks.
Responsibilities:
- design the schema changes required by the task;
- create reversible migrations where practical;
- verify constraints, indexes, and data preservation;
- run relevant database tests and report results exactly as observed.
Scope:
- schema, migrations, indexes, constraints, and queries touched by the task.
Inputs to consider:
- task objective and acceptance criteria;
- existing migrations and repository context;
- upstream architecture artifacts when present.
Constraints:
- do not redesign unrelated application code;
- do not invent test results or coverage figures;
- escalate data-preservation uncertainty explicitly.
Expected output:
- implementation with migration files;
- verification evidence;
- a concise note on compatibility and rollback risks.
"""

[[role_assignments]]
role = "database_engineer"
agent = "codex"
preferred = true

[policies.roles.database_engineer]
preset = "implementation"
```

### API Designer

- **id:** `api_designer`
- **name:** API Designer
- **description:** Designs API contracts, endpoints, request/response shapes and
  compatibility rules before implementation.
- **execution_class:** `advisory` — the valuable output is a design artifact
  (routes, schemas, error model, compatibility constraints) that implementation
  tasks consume; the role itself needs no file changes.
- **recommended policy preset:** `read_only`
- **Planner should use it when** a new or changed API surface affects several
  consumers or needs explicit compatibility decisions first.
- **Planner should NOT use it when** the endpoint shape is obvious from the
  objective or an existing convention.

```toml
[roles.api_designer]
name = "API Designer"
description = "Designs API contracts, endpoints, request/response shapes and compatibility rules before implementation."
execution_class = "advisory"
instructions = """
Purpose: produce an implementation-ready API design for assigned tasks.
Responsibilities:
- define endpoints, request/response shapes, and error model;
- state compatibility and versioning constraints;
- identify affected consumers and migration considerations.
Scope:
- the API surface named by the task; not its implementation.
Inputs to consider:
- task objective and acceptance criteria;
- existing routes and conventions in the repository context.
Constraints:
- do not modify production files;
- prefer existing project conventions;
- mark open questions explicitly instead of guessing.
Expected output:
- decisions with rationale;
- affected components and interfaces;
- constraints downstream tasks must honor.
"""

[policies.roles.api_designer]
preset = "read_only"
```

### Performance Analyst

- **id:** `performance_analyst`
- **name:** Performance Analyst
- **description:** Reviews performance-sensitive changes for regressions in
  latency, throughput and resource usage.
- **execution_class:** `review` — it evaluates the diff and evidence of an
  implementation and returns `approve` or `request_changes` with findings, exactly
  like any specialized reviewer.
- **recommended policy preset:** `review`
- **Planner should use it when** the change touches hot paths, queries, caching,
  or resource limits.
- **Planner should NOT use it when** the change has no plausible performance
  dimension (documentation, pure renaming, config comments).

```toml
[roles.performance_analyst]
name = "Performance Analyst"
description = "Reviews performance-sensitive changes for regressions in latency, throughput and resource usage."
execution_class = "review"
instructions = """
Purpose: evaluate whether the change hurts performance.
Responsibilities:
- inspect the diff for algorithmic regressions, N+1 access, and lock/contention changes;
- check query and cache behavior where relevant;
- verify claims in the evidence against what the tests actually measured.
Inputs to consider:
- implementation diff and producer evidence;
- acceptance criteria;
- upstream architecture artifacts when present.
Constraints:
- report only findings supported by the diff or evidence;
- do not invent benchmark numbers;
- keep severity proportional (low, medium, high, critical).
Expected output:
- approve, or request_changes with concrete findings (severity, summary, evidence).
"""

[policies.roles.performance_analyst]
preset = "review"
```

### Accessibility Reviewer

- **id:** `accessibility_reviewer`
- **name:** Accessibility Reviewer
- **description:** Reviews UI changes for accessibility regressions: semantics,
  keyboard interaction, contrast and focus management.
- **execution_class:** `review`
- **recommended policy preset:** `review`
- **Planner should use it when** the change modifies user-facing UI or
  interaction flows.
- **Planner should NOT use it when** the change is backend-only or has no UI
  surface.

```toml
[roles.accessibility_reviewer]
name = "Accessibility Reviewer"
description = "Reviews UI changes for accessibility regressions: semantics, keyboard interaction, contrast and focus management."
execution_class = "review"
instructions = """
Purpose: evaluate whether the UI change harms accessibility.
Responsibilities:
- check semantic markup, labels, and roles for changed components;
- check keyboard reachability and visible focus;
- check contrast and text sizing where the diff touches styles;
- flag removed or weakened accessibility attributes.
Inputs to consider:
- implementation diff and producer evidence;
- acceptance criteria;
- repository context for the affected components.
Constraints:
- report findings with concrete evidence (file and element);
- do not require unrelated retrofits;
- do not edit files.
Expected output:
- approve, or request_changes with concrete findings (severity, summary, evidence).
"""

[policies.roles.accessibility_reviewer]
preset = "review"
```

### Migration Specialist

- **id:** `migration_specialist`
- **name:** Migration Specialist
- **description:** Plans and executes data migrations: sequencing, backfills,
  compatibility windows and rollback requirements.
- **execution_class:** `execution` — chosen because the valuable output is a
  migration that actually runs: migration files, backfill steps, and verification
  evidence produced in a worktree (`implement` / `verify`). If your project only
  ever needs migration *risk assessments* without writing migration code, define
  the role as `advisory` instead — same boundary, different output contract.
- **recommended policy preset:** `implementation`
- **Planner should use it when** the task involves live data movement, backfills,
  deploy ordering, or rollback constraints.
- **Planner should NOT use it when** the schema change is a trivial additive
  column a Database Engineer or Worker can handle without migration sequencing.

```toml
[roles.migration_specialist]
name = "Migration Specialist"
description = "Plans and executes data migrations: sequencing, backfills, compatibility windows and rollback requirements."
execution_class = "execution"
instructions = """
Purpose: own data migration execution for assigned tasks.
Responsibilities:
- sequence migration steps so old and new code stay compatible during rollout;
- write backfills and verify them on the task's test data;
- document the rollback path for every step.
Scope:
- migration scripts and the data-touching code the task names.
Inputs to consider:
- task objective and acceptance criteria;
- existing migrations and schema in repository context;
- upstream architecture or database design artifacts.
Constraints:
- never drop or rewrite data without a stated rollback path;
- do not modify unrelated application code;
- report verification results exactly as observed.
Expected output:
- migration implementation;
- verification evidence;
- rollout and rollback notes.
"""

[policies.roles.migration_specialist]
preset = "implementation"
```

## 11. Domain-specific role suggestions

Suggestions, not built-ins. Each project should create only the ones with real,
recurring work behind them:

| Domain | Candidate roles |
| --- | --- |
| Frontend | Frontend Engineer, Accessibility Reviewer, Design System Maintainer |
| Backend | Backend Engineer, API Designer, Database Engineer |
| Infrastructure | DevOps Engineer, Release Engineer, Migration Specialist |
| Quality | Performance Analyst, Compatibility Reviewer, Localization Reviewer |
| Data / AI | Data Engineer, ML Evaluation Engineer, Prompt/Agent Workflow Reviewer |

Apply the [checklist](#6-role-quality-checklist) before creating any of these; a
suggestion is not a reason.

## 12. Anti-patterns

1. **Persona prompts.** *"You are a brilliant 10x engineer."* → Describe the
   responsibility, scope, and output instead.
2. **Mega-roles.** *"React Tailwind TypeScript Node Docker PostgreSQL Expert"* →
   Split into focused roles (Frontend Engineer, Backend Engineer, Database
   Engineer) and let the Planner compose them.
3. **Duplicate roles.** *"Implementer"* alongside *"Coder"* and *"Builder"* →
   Keep one; duplicates split routing history and confuse the Planner.
4. **Agent-specific roles.** *"worker_codex"* → Assign multiple agents to
   `worker`.
5. **No output contract.** *"Improve things where needed."* → State the expected
   artifact, evidence, or decision.
6. **"Think harder" roles.** *"Deep Thinker"* whose only purpose is more
   deliberation → Factory has no such operation; use advisory research or
   architecture tasks when more context is what's actually missing.
7. **Permission escalation.** *"You may push branches and edit .factory."* →
   Instructions never grant permissions; Factory's invariants deny push-class Git
   operations and `.factory` writes regardless. Configure policies for legitimate
   access.
8. **Duplicating Factory invariants.** Restating retry limits, worktree rules, or
   output JSON schemas in role text → Factory enforces these anyway; the copy
   drifts and wastes context.
9. **Too many roles in every recipe.** Selecting ten specialized roles for every
   workflow → Plans bloat or the Planner ignores them; keep recipes proportional
   and let specialized roles stay optional.
10. **Extremely long instructions.** Thousands of lines of rules → Compress to
    the 15–40 lines that change behavior; move permissions into policies.

## 13. Improving a role over time

Use Factory's own measurements as a feedback loop:

```text
Create role
   ↓
Run real workflows
   ↓
Inspect task/review results (Workflow Inspector, artifacts, evidence)
   ↓
Check Agent Performance (Performance view: first-pass approval, retries,
durations, role/operation breakdowns)
   ↓
Identify recurring failure or rework patterns
   ↓
Adjust role description/instructions
   ↓
Run again
```

Ground rules:

- Read the role/operation breakdowns in [Evaluations](evaluations.md) to see how
  attempts attributed to the role actually perform.
- Repeated `request_changes` findings with the same shape usually indicate a
  missing constraint or output detail in instructions.
- A role the Planner never selects has a description problem (or is unnecessary).
- **Do not automatically modify roles from evaluation data.** The user remains in
  control; evaluation should inform role design, not mutate it.

## 14. Advanced design: separate producer and reviewer roles

The role that produces a change should not always be the same responsibility that
validates a specialized property of it. Splitting is useful when:

```text
Database Engineer   vs   Database Reviewer
Frontend Engineer   vs   Accessibility Reviewer
Worker              vs   Performance Analyst
```

Reasons:

- specialized reviews receive the diff and evidence of the implementation they
  depend on, and return structured findings — a different job than producing the
  change;
- `request_changes` from a specialized review routes back into bounded rework of
  the implementation task, keeping the DAG acyclic;
- producer and reviewer can be filled by different agents, which is how you get an
  independent check.

Do **not** create a reviewer counterpart for every role by default. Reviews add
latency and cost; add them where the property under review (security,
accessibility, performance, data integrity) is expensive when wrong.

## 15. Naming conventions

```text
Display name:  Database Engineer
Role id:       database_engineer
```

- Prefer responsibility-based names; avoid `senior_database_guru`,
  `codex_backend_worker`, `worker2`, `super_agent`.
- Ids are lowercase slugs derived from the name; the slug is editable until first
  save.
- **Keep ids stable once workflows depend on them.** Renaming the display name is
  safe; changing the slug orphans historical references (teams, attempts,
  performance breakdowns). Deleting a role used by an active workflow is rejected.

See [roles.md — Custom roles](roles.md#custom-roles) for the exact rules.

## 16. Quick role creation worksheet

Fill this in before touching configuration:

```text
Role name:
Role id:

This role owns:
...

This role does NOT own:
...

Planner should use it when:
...

Execution class:            (planning | advisory | execution | review | post_process)

Inputs:
...

Expected output:
...

Constraints:
...

Recommended policy preset:  (read_only | implementation | documentation | review | custom)

Success looks like:
...
```

Then generate the instruction block from your answers:

```text
Purpose: <one sentence from "this role owns">

Responsibilities:
- <three to six concrete verbs from the ownership statement>

Scope:
- <from "this role does NOT own", phrased as exclusions>

Inputs to consider:
- <from Inputs>

Constraints:
- <from Constraints>

Expected output:
- <from Expected output>

Decision rules:
- <from "Planner should use it when", phrased for the agent>
```

The description field gets its own sentence, answered from the worksheet:
"When should the Planner use this role?" If the worksheet resists completion —
vague ownership, no output, no use trigger — that is the role telling you it is
not ready.
