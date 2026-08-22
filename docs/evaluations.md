# Evaluations & agent performance

Factory measures how each configured coding agent actually performs inside
workflows. Evaluation **observes, measures, compares, and exposes** reliable
performance data. The default scheduler routing stays deterministic and
capacity-aware; projects that opt into `[routing] mode = "performance"` let
the scheduler consume these measurements — through the same read-only
functions, never re-derived formulas (see [routing.md](routing.md)).

Everything is derived from the immutable workflow history stored locally in
`.factory` (SQLite). Nothing is sent to any external service, and no
telemetry exists. No LLM ever scores another LLM's output, and no token or
cost numbers are invented when the underlying CLI does not provide them.

## Evaluation architecture

- **`factory-eval`** (crates/factory-eval) is the evaluation engine. It reads
  `task_attempts`, `agent_sessions` (a lean timing-only projection), and
  `integration_outcomes`, reduces them to one record per task, and computes
  all metrics. It computes; it never schedules.
- **`factory-db`** owns the persistence. Migration V13 adds the
  `integration_outcomes` table because integration quality (clean vs
  rebased vs conflicted landings) was not derivable from existing rows.
  Attempt history itself is *not* duplicated into any event store.
- **`factory-api`** exposes two read-only semantic endpoints (below).
- **Dashboard** adds a Performance view and a compact Agent Inspector block.
- **Routing consumers**: `factory_eval::performance(db, agent, role,
  operation, language, now)` answers the routing question
  `performance(agent, role, operation, language?)`, and
  `factory_eval::resolve_performance(...)` walks the
  role/operation/language hierarchy to the most specific *reliable* slice.
  The performance router calls these; the metric formulas live only here.

## Data sources

| Source | Used for |
| --- | --- |
| `task_attempts` | outcomes, retries, attempt counts, timestamps, changed files, review decisions, error strings |
| `agent_sessions` | execution vs review durations per attempt (lean query, no output text) |
| `integration_outcomes` (V13) | clean/rebased/conflict integration results |
| `tasks`, `runs` | task/run context for seeding tests |

## Outcome classification

One centralized evaluator (`factory_eval::outcome`) classifies every
attempt; every metric uses it. Outcome categories:

| Outcome | Meaning |
| --- | --- |
| `approved` | attempt accepted by review |
| `changes_requested` | review requested changes; task ended without a later approval |
| `agent_failed` | agent process failed (non-zero exit, crash, spawn error) |
| `integration_conflict` | the integration rebase conflicted |
| `cancelled` | user cancelled the workflow mid-attempt |
| `interrupted` | a Factory restart interrupted the attempt |
| `policy_blocked` | the policy engine rejected the attempt's evidence |
| `configuration_error` | agent executable/invocation misconfigured |
| `in_progress` | running or under review |

### Task-level rules

- **Attribution**: a task belongs to the agent of its **first attempt**.
  A rescue by a different agent on retry #2 counts as eventual success (not
  first-pass) for the original agent.
- If **any** attempt is `approved`, the task outcome is `approved`
  (eventual success), regardless of earlier failures or rework.
- Otherwise a recorded integration conflict wins over an attempt stuck in
  review → `integration_conflict`.
- Otherwise the **last** attempt's classification is the task outcome.

### Exclusions (agent-quality denominators)

Only tasks whose terminal outcome is **agent-attributable** — `approved`,
`changes_requested`, `agent_failed` — enter quality-rate denominators
("qualifying tasks"). The following are counted separately and **excluded**:

- user cancellation (`cancelled`);
- Factory restart interruption (`interrupted`);
- policy rejection (`policy_blocked`, error string `blocked by policy: …`);
- configuration errors (`configuration_error`).

Configuration errors are recognized by the durable error-string prefixes of
`factory_agent::AgentError` / `AgentResolutionError` (e.g. `` executable `x`
was not found in the PATH…``, "has no non-interactive…"). These prefixes are
part of this contract; changing those Display strings requires updating
`factory-eval`.

Integration conflicts are excluded too: a Git conflict usually reflects
concurrent work, not agent quality, and is reported as its own metric.

## Metrics and exact formulas

For each agent over its attributed tasks in the window, with
`Q` = qualifying tasks:

| Metric | Formula |
| --- | --- |
| tasks attempted | number of attributed tasks (any status) |
| attempts | total attempts across those tasks |
| **first-pass approval** | tasks whose attempt #1 was approved ÷ `Q` |
| eventual approval | tasks eventually approved ÷ `Q` |
| request-changes rate | tasks with ≥1 `changes_requested` attempt ÷ `Q` |
| retry rate | tasks needing >1 attempt ÷ `Q` |
| terminal failure rate | tasks ending `agent_failed` ÷ `Q` |
| avg attempts per task | attempts ÷ tasks (all attributed tasks) |
| avg attempts per successful | mean attempts over eventually-approved tasks |
| median / p95 execution | per-task execution duration (below) |

**First-pass approval** means *accepted on attempt #1 without implementation
rework*. It is deliberately distinct from eventual approval: an agent that
succeeds only after three attempts must not look identical to one that
succeeds immediately.

### Durations

- **Execution duration** per attempt = sum of `agent_sessions.duration_ms`
  for the attempt's worker-role sessions (automated mode). Waiting for
  scheduler capacity is never included.
- **Review duration** = sum of durations of `review`-operation sessions
  attached to a *non-review* attempt (the built-in review of someone else's
  work). A `review`-operation session on a review task **is** that agent's
  execution.
- **Total task duration** = wall clock from the first attempt's start to the
  last attempt's finish (includes queueing between retries).
- Task execution/review durations sum across the task's attempts.
- Legacy attempts without session timers fall back to attempt wall time;
  these samples are counted in `approximateSamples` so the dashboard can
  disclose the approximation.
- Median of an even sample is the mean of the two central values; p95 uses
  the nearest-rank method (`ceil(0.95·n)`).

### Integration metrics

`integration_outcomes` rows are written for every approved-attempt
integration: `clean` (fast-forward), `rebased` (stale base, rebase
succeeded), `conflict` (rebase failed; the run stops exactly as before).

- clean integration rate = clean ÷ (clean + rebased + conflict)
- integration conflict rate = conflict ÷ (clean + rebased + conflict)
- rebased count is the stale-base signal

Conflicts are never counted as agent failures.

## Sample size and confidence

- Every rate carries its sample count (`total`).
- Rates carry a **Wilson 95% interval** (z = 1.96).
- A rate is `reliable` only with **≥ 10 samples**; below that the dashboard
  renders "Insufficient data (n=…)" instead of a percentage. n = 2 never
  produces a ranking-ready number.
- Duration medians are `reliable` with ≥ 5 samples; sample counts are always
  exposed.
- No single opaque "agent score" exists. If one is ever introduced it must
  be deterministic, documented, expose its components, and carry sample
  confidence.

## Time windows

`All time` · `Last 30 days` · `Last 7 days` (API: `all`, `30d`, `7d`).

A task belongs to a window when its **latest attempt started** inside it;
the task's **full attempt history** is then evaluated, so first-pass vs
eventual distinctions stay correct across window boundaries. Trends
(`Recent 10` / `Recent 25` tasks, and `Last 7 days` vs `Previous 7 days`)
are computed over the agent's full history for the applied
role/operation/language filters, ignoring the window filter, because the
week-over-week comparison inherently spans windows. There is no forecasting
and no general analytics query language.

## Role, operation, and language breakdowns

Performance is never only global. Every agent has breakdowns by:

- **role** (e.g. `worker`, `security_auditor`),
- **operation** (`implement`, `review`, `advisory`, `verify`, `post_process`),
- **language**.

Languages are derived deterministically from the extensions of files in
`TaskEvidence.changed_files` (`.rs` → Rust, `.ts/.tsx` → TypeScript, `.py` →
Python, …; full map in `factory_eval::language`). Tasks may be
multi-language and then count in every matching bucket. No label is forced
when evidence is insufficient (config files, lockfiles, dotfiles, or no
changed files).

## API

Read-only and semantic; no arbitrary SQL surface:

- `GET /api/performance/agents?window=7d&role=worker&operation=implement&language=rust`
  — compact summary per agent plus observed facet values.
- `GET /api/performance/agents/:agent?…` — full detail: outcome counts,
  durations, integration stats, role/operation/language breakdowns, trend,
  rework/failure reasons. 404 when the agent has no attributed history.

## Dashboard

- **Performance** view (`#/performance`): overview table (Agent, Tasks,
  1st-pass, Avg attempts, Median execution, Terminal failures) with
  window/role/operation/language filters; clicking an agent shows the
  detail (breakdowns, durations, integration, trend, reasons). Compact and
  analytical — no gamified leaderboard.
- **Agent Inspector** (Agent Graph → agent node → Overview): a small
  Performance block (Tasks, First-pass approval, Median execution, Average
  attempts) with a "View details" link. Graph nodes themselves stay
  uncluttered; agents without history simply omit the block.

## How evaluation feeds routing

The breakdowns answer, per agent and per slice: is the first-pass rate
reliable here, at what attempt cost, at what duration?
`performance(db, agent, role, operation, language, now)` returns exactly
those metrics for any (agent, role, operation, language) tuple, and the
router consumes them with Wilson-bounded confidence (a slice is used only
when it meets `MIN_RELIABLE_RATE_SAMPLES`). The agent detail endpoint also
reports whether each agent's metrics currently feed routing
(`routing.usedForRouting`), so the Performance view is the single observable
source of what the router sees.

## What is intentionally not implemented

LLM-as-judge scoring, downloaded model benchmarks, cost optimization,
token-based routing, automatic agent disabling, agent ranking on tiny
samples, cloud telemetry, and single opaque scores. (Deterministic
performance *routing* is implemented — see [routing.md](routing.md) — but
only from reliable local history.)
