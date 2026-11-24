# P13 test matrix

Every row identifies a real implementation boundary. Final run identities, outcomes, counts, hashes and unavailable cases belong in V2-P13 (historical record held privately) and `evidence/p13/manifest.json`. A test definition is not a recorded pass. Controlled subprocesses, retained historical model evidence and fresh genuine model requests remain distinct.

| Requirements | Automated evidence / actual boundary |
| --- | --- |
| SET-01,04,05,06,08; SEC-01,08,18 | `factory-api::preferences::tests::preferences_persist_reject_stale_and_invalid_and_preserve_workspaces`; actual legacy WorkspaceStore migration, reopened database, CAS, invalid/corrupt payloads |
| SET-02,03,07,10; SEC-02,07,08 | `factory-api/tests/product.rs::project_settings_are_an_authoritative_read_only_projection_without_credentials`; actual project configurations, distinct registered contexts, denied mutation and synthetic secret exclusion |
| SET-09; CMD-01,02 | `v2/product/product.test.tsx` settings search/focus; `commands.test.ts` real metadata matching; production browser Settings/palette journey |
| NOTIF-01,02,03,04,05,06 | `factory-db::notifications::tests::actual_transitions_are_durable_deduplicated_private_and_revision_checked`; actual run/scheduler transitions, database reopen, read/dismiss CAS |
| NOTIF retention / PERF-03 | `retention_and_paging_do_not_delete_runtime_evidence`: 1,010 source events, 1,000 retained notifications, bounded pagination, all source events preserved |
| Save conflict / important service failures | `typed_service_failures_are_private_bounded_and_deduplicated`; authenticated `a_real_save_conflict_creates_one_private_notification_without_overwriting_source` invokes P05 against an externally changed real file |
| NOTIF-07,08; SEC-03,04,05,06,11 | Authenticated `real_notification_events_and_read_state_remain_project_scoped_and_non_executing`; project-ID collision, CSRF denial, missing target, unchanged AgentSessions; frontend rejects foreign-project response before rendering |
| NOTIF-09,10; SEC-09 | Generic OS-notification payload and explicit settings permission action; component/browser checks distinguish denied/unsupported permission from in-app history. Native OS delivery remains separately qualified. |
| CMD-03,04,05,06,07,08 | `commands.test.ts`: disabled context, conflict detection, exact modifiers, IME/default-prevented events, terminal precedence; production keyboard palette/focus/quick-open/save journey |
| CMD-09,10; SEC-05,06,12,13,14,16,17 | Commands call existing document, project, lifecycle, agent, memory and Git services. Full actual API/policy/MCP/filesystem/GitHub regression suites remain required; command labels are not authorization. |
| UX-01,02,03,07,08,09,10 | `workspace.test.ts`, document/cache/stream regressions; P13 browser actual project switch, four presets, restored preferences/read state and unchanged task sessions/Git HEAD after reload; retained P04/P10 journeys |
| UX-04,05,06 | Actual mission/review/source links and existing P05 root/path/revision navigation; P11 provenance UI replay and P12 real Review inspection; missing resources fail rather than matching by title |
| A11Y-01,02,03,04,07,08,09,10 | P02 native dialog/menu tests; production keyboard palette, focus restore, labelled settings, editor save, Source Control, notification links and existing keyboard docking actions |
| A11Y-05,06 | P13 production browser Axe audits in both themes and reduced-motion mode; five viewport screenshots. Automated findings and manual visual inspection are recorded separately from native AT. |
| PERF-01,02,04,07,08 | ResourceCache/SessionStreams ownership regressions; bounded command search, shared scoped preferences/notification reads; repeated panel navigation/browser resource samples |
| PERF-05,06 | Real `performance.mjs` API/native-Git before/after fixtures, `EXPLAIN QUERY PLAN`, exact-index/unstaged independent checks; no memory extraction on Git status/staging/diff |
| P11 performance invariants | `factory-db::memory::performance_tests::derived_index_upgrade_retains_records_revisions_order_and_deletion_semantics`; full 26-test memory service suite, including forged/protected sources, concurrency, deletion, extraction policy, index rebuild and context eligibility |
| P12 performance invariants | `factory-git/tests/source_control.rs::large_filter_batches_inspect_every_path_and_preserve_the_index_on_refusal`; full native Git suite including stale hunks/index, active helpers, Unicode/ref ownership and protected worktrees |
| Journeys A,G,H,I | P13 production browser with real registration, CodeMirror, Git/hunk/local commit, authoritative settings, actual refused Planner notification, cross-project isolation and reload |
| Journey B / actual code review | `scripts/v2_p13/mission.mjs --model`; genuine configured Planner/Worker/read-only Reviewer, captured patch, actual integration, independent checkout tests and coverage oracle; no remote mutation |
| Journeys C,D,E,F | Current retained P08 parallel/conflict fixtures and original genuine P09/P10/P11 records inspected with the P13 binary; actual controlled handoff/lifecycle regressions, original model evidence hashes and new invocation-memory audit kept distinct |
| V1 and P03–P12 | Full serial and normal-threaded Rust; complete frontend; retained P03–P07 browser suites; P09/P10/P11/P12 current APIs/UI and frozen V1 database contracts |

## Execution and evidence discipline

Use fresh output directories under `.factory/v2/p13/`. The validation runner fingerprints crates, dashboard source, Cargo manifests and workflows before/after and rejects changed-source runs as qualification. Copy an embedded executable to a unique name; never rebuild a running Windows binary. Store its SHA-256 and build source separately from the browser/benchmark driver commit. Preserve failed attempts.

The resource-runner investigation executes the whole frontend suite at measured worker counts; a low-memory refusal is explicitly unexecuted. Native IDE tests use the approved local `ASF_IDE_TEST_TOOLS` profile and are counted separately from normally ignored optional cases. Remote CI and Linux/macOS are not inferred from Windows tests. Live GitHub writes remain unauthorized and unexecuted; deterministic production-provider tests do not become genuine remote PR evidence.
