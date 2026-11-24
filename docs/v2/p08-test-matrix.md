# P08 test matrix

This matrix names executable evidence, not unexecuted pass claims. Exact commands, source/binary hashes, counts and retained failures are recorded in `deliveries/V2-P08.md` and `evidence/p08/`. Controlled subprocess fixtures are intentionally distinct from real coding agents.

## Deterministic scheduler, Git and recovery

Rust integration suite: `crates/factory-core/tests/dynamic_scheduler.rs`. It uses real Git repositories/worktrees, Python-controlled child processes, actual agent routing/policies, Reviewer invocations, SQLite claims, Git publication and process supervision. It does not mock these boundaries.

| Requirements | Tests / evidence |
| --- | --- |
| SCH-01, SCH-02, SCH-03, GIT-01, GIT-02, GIT-03 | `siblings_overlap_and_dependent_base_contains_both_published_commits`: overlapping intervals, distinct worktrees, independently inspect both predecessor files in dependent base SHA |
| SCH-04, SCH-06 | `width_one_and_agent_one_are_real_sequential_fallbacks`: four ready tasks; run width one and agent capacity one |
| SCH-05, SEC-01, GIT-10 | `global_capacity_is_shared_across_projects_and_numeric_collisions`: two actual project databases with run ID 1, one shared budget, eight completed tasks |
| SCH-07, REL-09, REL-10 | DB `durable_owner_and_task_claim_have_one_winner_across_connections`: 12 competing SQLite connections; `duplicate_scheduler_control_cannot_fail_the_actual_owner` |
| SCH-08 | `execution::tests::waiting_runs_take_turns_without_a_saturated_project_blocking_others`, plus cross-project completion above; cooperative FIFO fairness, no real-time starvation bound claimed |
| SCH-09 | Ready sibling dispatch and capacity tests; failed-task recovery drains unaffected owned work before the revision barrier |
| SCH-10, REL-03 | Existing `workflow_runtime::a_broken_worker_executable_blocks_the_workflow_without_consuming_retries`, task-permit RAII and process-launch regressions |
| GIT-04, GIT-08 | Durable single owner/lane, per-attempt unique publication, compare-and-swap ref updates; stress verifies one published record per completed attempt |
| GIT-05, GIT-06, GIT-07 | `integration_conflict_preserves_outputs_and_is_not_silent_overwrite`: real Git rebase conflict, one published side, retained conflicting candidate and source attempt |
| Reviewer/check integrity | `reviewer_cannot_change_the_captured_source_and_approve_it`, `checks_cannot_publish_dirty_or_committed_changes_after_review`: real child-process mutations refused before publication |
| Agent permit order | `one_agent_shared_between_worker_and_reviewer_does_not_deadlock`: same configured agent, capacity one, sequential role handoff |
| GIT-09 | Parallel/dependency test verifies primary HEAD, branch and user checkout remain unchanged; existing Git/Policy regressions |
| RPL-01, RPL-02, RPL-05, RPL-08, RPL-09 | `obstacle_invokes_real_configured_planner_and_preserves_completed_rows`: controlled configured Planner process, new revision/new task, superseded attempt history |
| RPL-03, RPL-06, RPL-07 | DB `active_barrier_cycles_and_foreign_owners_cannot_change_the_plan`, `revision_dependencies_and_owner_gate_dispatch` |
| RPL-04 | DB `replan_compare_and_swap_preserves_completed_history_and_attempts`: exact completed Task equality including timestamps/dependencies, attempt retention, two queryable revisions |
| RPL-10, REL-08 | Scheduler policy validation, growth/deadline bounds, zero-replan conflict failure and bounded original per-task attempt tests |
| REL-01, REL-02 | `cancellation_drains_parallel_processes_and_releases_capacity`: two long-running real child processes, terminal session statuses and zero claims/permits after return |
| REL-04, REL-06 | `interrupted_attempts_are_not_blindly_replayed`, DB `restart_releases_claims_without_automatically_replaying_work`; uncertain process ownership is retained for inspection |
| REL-05 | `crash_after_ref_publication_reconciles_without_replaying_git_or_worker`: actual ref published before database finalization, restart records it and releases dependencies without rerunning work |
| REL-07 | `deterministic_policy_denial_does_not_invoke_recovery_planner`: one blocked attempt, no Reviewer or Planner loop |
| Legacy/deadline compatibility | `legacy_runs_have_no_new_deadline_and_explicit_deadlines_drain_processes`: legacy execution and null effective deadline; real long-running P08 child terminated at an explicit deadline, zero claims/permits afterward |
| Setup ownership | `policy_setup_failure_releases_ownership_without_launching_agents`: corrupt actual SQLite policy row, one auditable owner closed, no AgentSession launch |
| Stress 2/4/8 | `bounded_stress_two_four_eight_has_no_duplicate_attempts_or_capacity_leaks`: two iterations per width, more ready tasks than permits, actual overlap, unique attempts/publications, zero remaining claims/capacity |

Stress measurements describe controlled fixture scheduling, not model coding throughput. Windows process lifetimes and PID metadata are captured by the supervisor; real acceptance additionally samples simultaneous OS processes.

## Security and existing regressions

| Requirements | Executable coverage |
| --- | --- |
| SEC-02, SEC-03, SEC-04, SEC-05 | Existing core `policy_enforcement` and `authorization` suites, P07-R1 permission API regressions, scoped real acceptance, original P05 managed worktree tests |
| SEC-06 | Existing `factory-mcp` grants/attempt/session revocation suites; every concurrent invocation uses its existing distinct `InvocationScope` |
| SEC-07, SEC-08 | Same effective policy gate for revised tasks, strict recovery schema, role/operation resolver and barrier/CAS rejection tests; unknown commands/policies cannot become an action |
| SEC-09 | Existing P03 scoped event tests and P07 durable conversation/replay/isolation tests |
| SEC-10 | Existing GitHub delivery eligibility, claim/reconciliation and explicit authorization tests; P08 never calls publication |
| Integration inspection | `factory-files::p08_integration_snapshots_are_authorized_distinct_and_read_only`: separate root identity, exact snapshot reads, denied save and process launch, primary unaffected |
| Process identity | `factory-agent::process_identity_is_observed_before_dispatching_mission_input`; existing process cancellation and output capture regressions |
| Frontend | `v2/conversation/scheduler.test.tsx`: actual-stage counts, revision/conflict history, explicit project IDs for task/Planner navigation; existing P03-P07 workspace/editor/IDE regressions |

Native junction/ACL/Unix signal/macOS filesystem coverage is stated by platform in the delivery report. Tests that simulate persisted crash boundaries do not claim exhaustive instruction-level power-failure or arbitrary external-side-effect qualification.

## Real-model acceptance

`scripts/v2_p08/acceptance.mjs` creates isolated repositories outside ASF source, initializes/registers them through supported interfaces and uses the installed real CLI under P07-R1 sandbox profiles. It applies no application patch. The production browser conversation or visual-plan editor submits objectives. Actual Worker/Reviewer sessions, policy audits, diffs, candidate checks, plan revisions, refs, process intervals and independent fresh-checkout tests are retained.

- A/B/F: four independent module improvements, task cap two, real Planner, multiple Workers, dependent combined tests and base-commit ancestry.
- C/D: isolated edits to the same expression, detected integration conflict, preserved outputs, configured Planner revision and reviewed repair.
- E: producer/client compatibility obstacle, actual failed/rejected evidence, bounded repair of unfinished work and combined tests.
- Browser: live progress, worktree read-only inspection, review/publication details, history/replay, themes, project switching and V1 access. Screenshot generation alone is not qualification.

The real acceptance result remains separate from deterministic test success. Never substitute controlled worker output, a UI indicator or a prepared implementation patch for genuine coding evidence.

## Qualification follow-up

`exhausted_cli_capacity_stops_without_retries_or_a_recovery_planner` exercises native-adapter diagnostic handling through a controlled real child process, including concurrent cancellation and zero leaked claims/permits. `authorization::tests::exhausted_native_usage_is_distinct_from_code_and_policy_failures` rejects false classification from custom agents, successful, cancelled and truncated output. `SchedulerStates.test.tsx` renders all three newly exposed persisted states through V1 badges and task graphs; the browser suite additionally exercises the actual Agent Graph with superseded tasks.

DB `late_observer_cannot_regress_a_revision_or_duplicate_its_notice` uses separate SQLite connections to deliver a stale revision and a stale nonterminal status. It verifies the authoritative revision/status survive and that one real replan creates only one notice/event. This complements the P07 delayed-response, replay and event-deduplication regressions.

Real A/B/F and C/D succeeded in the second acceptance run. The semantic compatibility candidate was correctly refused, and a real Planner produced a repair revision; provider usage exhaustion prevented completion of that additional repair. The first run completed a genuine Reviewer-requested repair but failed parallel qualification. These outcomes are retained separately, and the uncompleted semantic repair is not counted as a passing acceptance test.

## P08-R1 acceptance closure — 1 November 2026

The subsequent R1 delivery (historical record held privately) closes the semantic-repair and live-interruption gaps with separate real-agent records. Its R1-01–R1-15 invariant map (historical record held privately) links the retained provider refusal, bounded semantic validation/publication guard, actual Planner revision, genuine Worker/Reviewer, owned-process cancellation, server restart, history and replay evidence to their tests. The original outcomes above remain historical evidence rather than being relabelled. P08 is now complete locally; remote CI, other native platforms and V2 release qualification remain pending.
