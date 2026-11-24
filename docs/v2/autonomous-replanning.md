# Autonomous replanning

P08 extends the existing configured Planner, plan resolver and durable `plan_revisions`; it does not introduce a second task model. Recoverable exhausted attempts, review rejection, failed candidate validation, integration conflict or an unfinished DAG with no ready task can trigger replanning. Policy/approval/host denials, malformed permission reports and unavailable executables stop rather than entering a repair loop.

The coordinator stops new dispatch and drains active work. Unaffected successful tasks may finish review and publication before the barrier. No active, reviewing, AwaitingIntegration or Integrating task can be superseded. The owner invokes the actual Planner with the original objective, completed anchors, current approved commit, bounded attempts/patches and the observed obstacle. Evidence is untrusted data, not permission. Truncation is labelled.

The Planner returns the normal finite plan schema for unfinished work. New IDs are unique nonnumeric labels; numeric dependency references may name only completed anchors. Unknown fields, arbitrary commands/policies, unknown references, cycles, incompatible roles/operations and task/growth limits are rejected. Effective project/recipe/agent policy checks run before commit.

An immediate SQLite transaction rechecks owner, drained claims and expected revision. It adds new task IDs and marks replaced unfinished rows Superseded. Completed and already superseded rows, timestamps, dependencies, attempts, reviews and prior revision snapshots remain unchanged. Superseded attempted work retains its source commit and worktree. The original mission objective is retained even if Planner prose differs.

Each revision records reason, source Replan, real Planner AgentSession, timestamp and complete resulting snapshot. P07 Progress shows previous and revised graphs as keyboard-readable task/dependency lists; the existing workflow DAG remains available. A concise runtime-sourced conversation notice records significant revision changes without impersonating a model response.

Replans, task growth, per-task attempts and execution duration are bounded. Invalid output is not applied; exhausted recovery is explicit. No fixed alternate DAG is represented as autonomous intelligence. Controlled agents test deterministic contracts separately from real-model qualification.
