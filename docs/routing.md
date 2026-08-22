# Intelligent Agent Routing

Factory routes every dispatch — worker, reviewer, advisory, or any custom role —
to one of the agents assigned to that role. Routing is **deterministic, local,
explainable, policy-aware, and safe when performance data is thin**. There is no
LLM in the routing path and no randomness: the same candidate set, durable
performance data, capacity state, and task context always produce the same
choice.

See also: [architecture.md](architecture.md) for where routing sits in the
runtime, [evaluations.md](evaluations.md) for the metric definitions the router
consumes, [policies.md](policies.md) for the Policy Engine.

## Configuration

```toml
[routing]
mode = "round_robin"   # round_robin | performance | manual
exploration = true     # bounded cold-start data gathering (performance mode)
```

| Mode | Behavior |
| --- | --- |
| `round_robin` | The original behavior and the **default**: least-loaded pool member, round-robin tie-breaking. Existing projects keep this until they opt in. |
| `performance` | Deterministic score from reliable `factory-eval` history (see below). Falls back to round-robin whenever evidence is insufficient. |
| `manual` | Each task's pinned agent (`PUT /api/tasks/:id/routing` or the Task Inspector), else the role's preferred assignment. |

A per-task agent pin is honored in **every** mode; it never routes around
policies or role assignments (see [Manual override](#manual-override)).

## Candidate filtering comes first

Before any scoring, a candidate must:

1. **belong to the task's role** in the workflow's team snapshot
   (`WorkflowTeam::agents_for_role`);
2. **pass the Policy Engine** — `validate_executable(effective_policy(role, agent), operation)`
   must accept the operation;
3. **be resolvable** — installed, configured, and usable for automated
   invocation (`Agents::command_agent_for`);
4. **have capacity** — in-flight work below the agent's `max_concurrency`
   (reserved atomically at dispatch, see below).

Performance can rank only eligible survivors; it can never resurrect an
ineligible agent. The routing order is always:

```text
role/team eligibility → policy eligibility → availability/config → capacity → performance
```

## The routing score

Only candidates with a **reliable** performance slice are ranked (see
[Confidence](#confidence)). For them:

```text
score = 0.55 · quality      Wilson-lower-bound mean of first-pass and eventual approval
       + 0.20 · rework      1 − Wilson upper bound of the retry rate
       + 0.10 · speed       (slowest reliable median ÷ own median), clamped to [0, 1];
                            0.5 (neutral) when the duration sample is unreliable
       + 0.15 · capacity    free fraction of the agent's max_concurrency
       + 0.02              preferred-agent bonus (scored candidates only)
       − 0.05              retry penalty: this agent already failed or was asked
                            for changes on THIS task
```

Design notes:

* **Quality dominates speed.** With 55% vs 10% weight, a fast but unreliable
  agent cannot out-rank a slower high-quality one.
* **Confidence is rewarded** (see below): 97% with n=10 does not beat 94% with
  n=200.
* **Integration conflicts stay a separate weak signal** inside `factory-eval`
  and never enter this score.
* The weights are fixed constants (`factory-core/src/routing.rs`), not user
  knobs. There is no product need for per-project tuning yet.

## Performance hierarchy

Evidence is resolved per agent from the most specific reliable slice to the
least (`factory_eval::resolve_performance`):

```text
agent + role + operation + language   (when a language is known)
        ↓ if insufficient
agent + role + operation
        ↓
agent + role
        ↓
agent (global)
```

A slice is used only if it meets `factory-eval`'s reliability requirement
(≥ 10 qualifying samples — the same `MIN_RELIABLE_RATE_SAMPLES` the Performance
view uses). A tiny highly-specific sample therefore never masks a large
reliable broader one; the router simply walks down to the broader slice.

**Language resolution** is deterministic: the router uses the language of the
files changed by the task's previous attempts (`TaskEvidence.changed_files` →
`detect_languages`), when exactly one language is observed. Fresh tasks have no
evidence yet and route on `role + operation`; retries with Rust evidence route
on the Rust slice. (Routing from the repository context index is a deliberate
future refinement, not in V1.)

## Confidence

Raw success percentages are never used alone. Quality uses the **lower bound of
the Wilson 95% interval**; the retry component uses the **upper bound**. Both
directions are conservative against the agent being scored, so thin samples
shrink the reward automatically:

* 97% approval with n=10 → Wilson lower bound ≈ 0.74;
* 94% approval with n=200 → Wilson lower bound ≈ 0.90 — and the router prefers
  the better-evidenced agent.

The interval math lives in `factory-eval` (`stats.rs`); the scheduler never
re-implements it.

## Insufficient data and fallback

If **no** candidate has a reliable slice, nothing is ranked — the router never
compares `Agent A n=2` against `Agent B n=1`. It falls back to the existing
deterministic capacity-aware round-robin and says so in the decision record.
Cold start is therefore exactly as predictable as before routing existed.

## Capacity and the Parallel Runtime

Performance routing reserves capacity with `AgentCapacity::try_acquire`, which
checks the in-flight count and increments it **under one lock**: two concurrent
dispatches can never both take the final free slot of the same agent, and a
ranking computed from a load snapshot is always guarded by the final
reservation (if the reservation is lost to a race, the next-best eligible
candidate is taken instead).

A saturated historical favorite does not block a strong alternative:

```text
Codex    score 0.91   2/2 busy
OpenCode score 0.86   1/2 busy   → OpenCode runs
```

The 15% capacity component moves near-ties toward free agents, the 0-free-slot
candidates are skipped entirely by the reservation loop, and if every ranked
candidate is saturated the router falls back to capacity-aware selection
rather than queueing forever. `max_concurrency` (default 1, max 32) is
enforced by these reservations in performance mode; round-robin mode keeps the
previous meter-only behavior.

## Preferred agents

`preferred = true` on a role assignment keeps its meaning: the preferred agent
(the flagged one, or the first declared when none is flagged) receives a
**+0.02 bonus**. The bonus:

* breaks near-ties and identical-quality ties deterministically;
* never overrides policy or capacity (those filter first);
* never overrides a real performance gap (0.02 ≪ typical quality differences).

## Exploration (cold start)

Without exploration, a well-known agent would monopolize all future evidence
and under-sampled teammates would never become rankable. With
`exploration = true` (the default), every 5th dispatch (driven by the durable
run attempt count, `EXPLORATION_INTERVAL`) is routed to the least-observed
unranked eligible candidate with free capacity — but only when at least one
other candidate *is* reliably ranked. It is a fixed deterministic rule, not
bandit or reinforcement-learning infrastructure, and it can be disabled
entirely.

## Manual override

The Task Inspector (or `PUT /api/tasks/:id/routing`) can pin a task to a
specific agent. The pin:

* is validated up front against the role's assignments (unknown or unassigned
  agents are rejected with a clear error) and re-validated at `prepare_start`;
* still cannot bypass policies, role assignment, or availability/config — a
  pin that fails those gates **blocks the task with a clear error** rather than
  silently choosing another agent;
* is persisted with the task (`tasks.agent_override`) and survives restarts
  and plan edits;
* does not apply to the built-in final review (the pin selects the task's own
  role, not its reviewer);
* can be set only while the task has not started (pending/ready/blocked/failed).

## Retries

Retries may route to a different agent. Each attempt is routed independently
under the current mode, and candidates that already failed or received
`request_changes` on this specific task carry a −0.05 penalty, so a near-tie
moves to the next agent while a genuinely better agent can legitimately be
retried. Bounded retry limits (`MAX_TASK_ATTEMPTS`), attempt attribution, and
the no-duplicate-concurrent-attempt guarantee are unchanged; retry history is
never reset.

## Degradation protection

Routing uses **all-time reliable data** (the same windows and confidence as the
Performance view). One recent failure barely moves a Wilson lower bound with
n ≥ 10, so the router does not flip every task after a single bad run, and it
never disables an agent. Recent trends are visible in the Performance view but
intentionally have no routing influence in V1.

## Routing decision records

Every dispatch persists one compact `RoutingDecision` row
(`routing_decisions` table, exposed at `GET /api/tasks/:id/routing-decisions`):

```text
task_id, attempt_id, mode, selected_agent, role, operation, language,
candidate_scores (agent, score | null, reliable, note), reason, created_at
```

The note names the evidence slice each score came from (for example
`role+operation+language slice, n=42` or `insufficient data (n=3 of 10)`).
Records are small and bounded — no evaluation snapshots are stored. A worker
attempt plus its built-in review produces two records, distinguished by
role/operation.

## Routing preview

`GET /api/tasks/:id/routing-preview` (Task Inspector) shows the current mode,
likely candidate, reason, and per-candidate scores. It is informational:
actual selection happens at dispatch time because capacity and history change.

## Determinism

Given the same candidate set, durable performance data, capacity state, and
task context, the router picks the same agent. Tie-breaking is stable and
documented:

```text
score (descending) → preferred → configured pool order → agent name
```

HashMap iteration order is never consulted; scores are compared with
`total_cmp`.

## Reviewer and custom roles

The router is generic over roles: reviewer pools (including the built-in final
review), `security_auditor`, `test_engineer`, `researcher`, and custom roles
all route on their own role/operation-specific history automatically. Planner
selection remains explicit (the team's planner); there are no planner
ensembles.

## Limitations and non-goals

* Routing selects **configured external coding agents**, never underlying
  models; no model switching, cost/token optimization, or provider pricing.
* No LLM router, no reinforcement learning, no cloud routing service.
* No automatic agent disabling and no cross-project global telemetry — each
  project's history stays in its own `.factory` database.
* Language hints come from prior attempt evidence (retries) in V1; first
  attempts route on role + operation.
* The evaluation read is O(slices × history) per dispatch — fine for local
  SQLite histories at Factory's scale, and a natural place for caching if
  large histories ever make it matter.
