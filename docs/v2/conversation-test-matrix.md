# P07 conversation and mission test matrix

This is a source-to-contract map. Execution counts, failures, actual agent/tool qualification, screenshots and platform status are recorded in V2-P07 (historical record held privately). Test existence is not a claim of a passing remote run. The original controlled agent identifies itself as a protocol fixture; its output is not model-quality evidence.

## Functional contracts

| Scenario | Automated evidence |
| --- | --- |
| CHAT-01 creation/durability | `factory-conversation/tests/conversations.rs::question_is_persisted_without_a_workflow`; API ownership test; browser creation |
| CHAT-02 real response | Browser configured-Codex question, persisted agent-session provenance; separate from controlled-agent tests |
| CHAT-03 history/reload | Browser restart/restoration journey and SQLite message queries |
| CHAT-04 backend restart | Browser real server stop/restart; `interrupted_acceptance_is_reconciled_without_repeating_a_command` |
| CHAT-05 project isolation | `project_histories_and_numeric_run_collisions_remain_independent`; API foreign conversation/history/events/mutation refusal |
| CHAT-06 duplicate request | `duplicate_submission_has_one_command_and_one_executed_mission`; changed-key-payload conflict; API/browser duplicate submission |
| CHAT-07 automatic create/plan/start | Controlled real-process pipeline test; browser real model classification/Planner and native execution outcome recorded separately |
| CHAT-08 actual execution/evidence | Controlled run completes with real worktree, commit, review and task evidence; real-model qualification limitations remain explicit |
| CHAT-09 planning failure | `invalid_planner_output_remains_recorded_without_execution` retains run and rejected sessions |
| CHAT-10 invalid DAG | Same test plus visual-graph cycle validation; no worker invocation |
| CHAT-11 policy rejection | `readonly_policy_blocks_automatic_execution_without_worker_side_effects` |
| CHAT-12 form | `form_and_visual_plan_use_the_same_durable_pipeline`; browser real filesystem workflow through the form |
| CHAT-13 visual plan | Same Rust test verifies exact authored task and absence of Planner replacement; browser graph/task editor |
| CHAT-14 invalid dependency | `validateGraph` cycle/missing-ID tests and Rust DAG validation |
| CHAT-15 progress | Runtime snapshot assertions, task counts and actual-session browser inspection |
| CHAT-16 results | Completed controlled run/evidence assertions; user-facing summary is structured-record-derived |
| CHAT-17 failed tests | `failed_worker_is_not_a_successful_test_summary`; browser controlled nonzero worker exit; no blanket test-success text |
| CHAT-18 cancellation | `cancellation_stops_owned_work_and_preserves_history`; browser independent B/C workflow cancellation |
| CHAT-19 disconnect | Browser reload/switch while controlled workflows own real processes; view closure is navigation only |
| CHAT-20 replay | `event_replay_is_ordered_and_resumes_from_a_durable_cursor`; browser actual SSE cursor records |
| CHAT-21 deduplication | Frontend overlapping/out-of-order/foreign event tests |
| CHAT-22 slow consumer | API `conversation_sse_slow_subscriber_is_bounded_and_replayable`, real SSE body left unconsumed before bounded drain |
| CHAT-23 two windows | Browser second-window submission and first-window history convergence |
| CHAT-24 delayed response | Frontend late-A-response test with separately keyed B store; browser real background project response |
| CHAT-25 file context | Browser saved file reference and actual content; P05 authorization re-used |
| CHAT-26 unsaved context | `unsaved_selection_is_retained_with_explicit_provenance`, Unicode byte selection and disk unchanged assertion; browser CodeMirror selection |
| CHAT-27 protected context | Rust protected/ignored/traversal fixture, API foreign root, browser protected Factory path |
| CHAT-28 no configured agent | Rust and API retained-failure tests; browser configuration-required view |
| CHAT-29 injection | Unknown action/project override schema tests, malicious repository-content fixture, inert HTML/URL frontend rendering |
| CHAT-30 V1 | Existing multi-project/core workflow regressions and original V1 browser journeys on the final executable |

Additional regression: `invalid_authored_role_fails_the_created_run_instead_of_leaving_false_planning` checks an invalid role cannot leave an idle run labelled planning. Frontend form failures retain typed objectives, and a retry-copy button never submits without a separate action.

`classifier_process_failure_is_distinct_from_a_policy_refusal` checks a real controlled classifier exit 17: the durable message identifies an agent-process failure and session, does not mislabel it a policy block and creates no workflow. `a_followup_question_does_not_repeat_a_previous_mission` checks that a later question creates no second run. The recovery browser separately drops an acknowledgment after actual server acceptance and verifies idempotent Retry, Enter/Shift-Enter and unsent draft restoration.

`inspect_existing_project_run_is_read_only_and_never_falls_back_to_another_project` reads a run from a new conversation, preserves its planned state, assigns no command run ownership and refuses the same numeric ID where absent in another project. A frontend test checks scoped progress/review links for such a reference; the browser opens an already completed mission from another conversation and returns to the original selection.

`factory-projects::conversation_service_lease_prevents_premature_runtime_close` checks that observer/service ownership blocks project close until the final lease is released; reopening then acquires the correct runtime lock.

`factory-db::conversations::tests::conversation_writer_waits_before_reading_a_snapshot` holds a separate SQLite write reservation and verifies that a competing conversation transition waits before reading, then completes. The isolated `scripts/v2_p07/wal-reproduction.py` demonstrates the obsolete deferred-snapshot upgrade failure; it is database-mechanism evidence, not an agent or browser pass.

The fourth recovery-browser case gates an actual controlled Planner process and worker process. It asserts persisted planning/active/completed states against the visible Progress view before capturing them; the final capture also waits for the polled workflow list to agree. Earlier rapid-transition images showed processing and are retained as superseded attempts. No production status is overridden for screenshots.

## Isolation invariants

| Invariant | Enforcement and evidence |
| --- | --- |
| C1 one project/conversation | Project-owned service/database plus persisted `project_id`; foreign-ID API refusals |
| C2 selected conversation/session | P04 persisted `ViewState.conversationId`, project-scoped lookup and separate `s_` provenance; workspace sessions are not authorization principals |
| C3 mission ownership | Atomic command/run association in the project's own DB; P03 runtime context |
| C4 distinct IDs | `c_` strings never routed as numeric workflow or AgentSession IDs |
| C5 numeric collisions | Two real fixture databases each execute run 1 independently |
| C6 separate history | Foreign conversation get/messages/list tests and keyed frontend stores |
| C7 separate output/events | Authenticated project/conversation SSE; foreign stream route refused before subscription |
| C8 switching continues | Browser active B/C workflows plus original P03/P04 switch regressions |
| C9 closing view continues | Store unsubscribe does not call cancellation; existing workspace close regressions |
| C10 immutable operation owner | Worker retains its service's project/root/Runtime; no mutable active-project authority |
| C11 late-response safety | Explicit delayed-A frontend response and project-owned stores/keys |
| C12 no duplicate mission | Durable unique key/fingerprint, transactional run association and duplicate API/browser tests |

## Qualification boundaries

Real CLI response, planning and complete mission execution are separate observations. A provider usage limit or CLI read-only refusal is retained as a refusal, not replaced with a passing fixture. A non-mutating real-agent research mission does not qualify the same CLI for code writes. Controlled agents validate orchestration mechanics and actual filesystem effects, not model ability.

Windows native, Linux/macOS, exact-commit remote CI, native assistive technology and final release qualification are separate gates. P08 parallel scheduling, P09 profile transfer and P10 suspension are absent and are not inferred from these tests.
