# Dynamic scheduler (P08)

The existing `Factory::execute_active_run` is the sole execution engine. SQLite remains authoritative. A scoped Rust coordinator dispatches the existing implementation/advisory/review operations on separate connections, with one frozen `Agents` configuration snapshot per owner. There is no external queue or browser-owned scheduler.

## Eligibility and dispatch

A short `BEGIN IMMEDIATE` transaction checks the active run, exact plan revision, durable scheduler owner, Ready state and every prerequisite. Prerequisites satisfy dispatch only after Completed (or historical Superseded) state. Implementation tasks become Completed only after reviewed publication. Process exit and Reviewer approval alone do not release dependencies.

A partial unique index permits one unfinished owner per run and one unfinished dispatch per task. Claims survive connection loss; they have no automatic expiry/steal policy. Worker execution, review, Git and checks run outside database transactions. WAL and the existing bounded five-second SQLite busy timeout remain in force.

The coordinator reads effective scheduler configuration after acquiring durable ownership. A concurrent pre-execution policy change therefore either commits before ownership and is observed, or is refused after ownership. An ownership guard closes setup failures before any agent launch; it does not silently steal an uncertain owner.

The coordinator keeps a task permit through Worker, review and queued integration, preventing freshness review/check processes from exceeding the active-task budget. All routing modes, pins and fallback paths use atomic per-agent reservations. Worker reservations are released before requesting a Reviewer slot, including when one configured CLI serves both roles.

## Limits and fairness

`[scheduler]` configuration is snapshotted per new V2 conversation run: `max_parallel_tasks=4`, `max_project_tasks=4`, `max_replans=2`, `max_total_tasks=128`, `max_duration_secs=3600` by default. Run width is capped by the existing recipe/project runtime preference. Widths are 1-32, replans 0-8, historical growth 1-512 tasks and duration 1-86400 seconds. Each revision has at most the existing 50 tasks. Existing per-agent `max_concurrency` remains authoritative (default one).

One application `RuntimeBudget` shares `TaskBudget` across registered projects; default global task cap is eight. `ProjectLimits.concurrent_tasks` and `factory start --max-active-tasks N` configure the shared 1-32 application cap; `RuntimeBudget::with_task_limit` supports embedded callers. Project contexts share agent reservations across their missions and conversation invocations. Agent names are scoped to their project configuration; the application task cap is shared across all projects. CPU/RAM quotas and OS sandboxing are not claimed.

Ready tasks use stable plan position/ID order. Capacity waiters rotate FIFO across `(canonical project identity, run ID)` keys; a saturated project's waiter does not block an eligible different project. Absent waiters expire after two seconds; active permits and their restrictive project limits do not expire. This is cooperative fairness, not a real-time deadline guarantee. Slow integration retains its permit deliberately. Dependencies, width one, busy agents or project limits provide sequential fallback.

## Compatibility and observation

Old/V1 runs without a P08 policy use the same coordinator at width one with no autonomous replans. Their timestamps and prior ordering are not reinterpreted. New P07 chat/form/visual missions opt into P08 defaults. Authenticated scoped `GET/PUT /api/projects/:projectId/runs/:id/scheduler` exposes the state and pre-execution configuration; configuration is immutable after dispatch ownership starts.

Legacy runs retain V1's absence of a workflow-wide deadline. Existing per-invocation timeouts and explicit cancellation still apply. A persisted P08 policy enables its validated workflow deadline; `deadlineSeconds` in scheduler state/events is the effective value, or null for legacy compatibility. The legacy policy template's numeric duration is not an enforced deadline.

Scheduler decisions are schema-versioned durable events with per-project SQLite ordering. P07 snapshots expose actual active stages, immutable revision history and publication records. A task awaiting an agent is labelled separately from a running AgentSession. Closing a presentation tab has no cancellation or suspension semantics.

## Repository identity

P09 adds a separate immutable registered project ID for profile and handoff audit records. The existing canonical-root capacity key and ownership validator retain their original purpose.

The P03 manager binds its original directory and SQLite file identities to an immutable `ProjectExecution` validator. Parallel connection creation, dispatch loops, Worker launch/evidence and integration validation/publication recheck that identity. Replacing a path or database blocks execution and requires explicit relinking; the browser cannot change the callback. This reuses P03 identity rules and does not eliminate external filesystem TOCTOU races or provide an OS sandbox.

## Default team and prerequisite handoff

P09 extends agent resolution with project-owned revisioned profiles. Enabled automatic-delegation profiles contribute to role pools; explicit teams and task pins remain authoritative. Selection resolves the trusted installation, requested model, effective policy and existing capacity. Snapshot insertion rejects a conflicting profile revision before process launch. Active attempts retain their selected configuration; new attempts reload current eligibility. A handoff uses the existing durable task owner and a new attempt at a verified boundary. See [profiles](agent-profile-architecture.md) and [handoffs](agent-handoff.md).

A new V2 conversation mission without an explicit team or recipe snapshots every already assigned Worker and Reviewer, keeping the preferred member first. Explicit teams, recipe selections and V1 defaults are preserved. Per-agent eligibility, availability and effective policies remain dispatch gates. This uses existing configured assignments; it neither creates profiles nor grants permissions.

Dependent P08 invocations receive bounded `ASF_DEPENDENCY_FACTS` from the same project database: predecessor IDs/states, actual review decisions and published candidate commits. This establishes dispatch prerequisites without requiring a Worker to access ASF internals through a source checkout. The facts never claim that the new task or its tests succeeded.
