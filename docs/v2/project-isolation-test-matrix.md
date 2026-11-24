# P03 - Isolation and integration evidence matrix

Executed results, counts, retained logs and platform limits belong to V2-P03 (historical record held privately). Tests below use real temporary repositories, SQLite files, supervised child processes and the production routers. Controlled agents are explicitly deterministic test executables, not live provider agents. No test publishes a real GitHub PR.

## Invariants

| Invariant | Automated regression evidence |
| --- | --- |
| I1 authoritative project scope | API `identical_run_ids_are_scoped_and_legacy_default_never_switches`; `project_management_roundtrip_initializes_clones_renames_and_removes_without_deleting_files` |
| I2 numeric IDs are local | API identical runs; `one_tcp_server_scopes_conpty_websockets_sse_resize_reconnect_and_termination` creates session 1 in each DB |
| I3 UI cannot redirect existing operations | API `two_real_controlled_workflows_complete_with_distinct_worktrees_and_evidence`; browser late-A switch/reload case; hook late-result and late-error tests |
| I4 project config and policy isolation | API `configuration_agents_context_and_policies_remain_project_owned` mutates A and compares B config bytes; same `codex` name has different environments |
| I5 cancellation ownership | API `cancelling_a_cannot_cancel_b_and_busy_project_cannot_close` gates real worker children; cancels A and finishes B |
| I6 Git ownership | Actual distinct task worktrees, commits and integration branches in API workflow test; different fixture GitHub remotes in MCP/delivery API test; shared Git/worktree rejection and common lock tests |
| I7 MCP scope | API `mcp_grants_delivery_and_deleted_projects_never_fall_back_to_another_project` checks A grant hash absent in B, B config bytes and tool server list; existing MCP grant/CLI/policy suites |
| I8 no fallback on registry failure | API `no_default_fails_explicitly_and_registry_corruption_cannot_redirect_scoped_requests`; removed-ID and malformed/unknown-ID cases |
| I9 one runtime owner | projects `concurrent_registration_and_open_share_one_runtime`, Windows aliases, `os_lock_contention_in_another_process_does_not_block_b`, cross-manager mutation guard, linked-worktree common Git lock; original CLI second-start regressions |
| I10 unavailable history retained | projects unavailable/relink, corrupt fixture, disappeared authorized root; browser move/relink preserves ID and run 1 |
| I11 browser security preserved | API management auth/Origin/CSRF/trust case; authenticated network terminal + rejected foreign Origin; complete existing security suite |
| I12 non-destructive V1 opening | projects frozen V1 fixture / actual preserved worktree / repeated open and restart; complete frozen DB row/column compatibility suite and migration rollback/backup tests |

## Required scenarios

| Scenario | Evidence / specific boundary |
| --- | --- |
| MP-01 two projects, one server | `FactoryApplication` API tests; real TCP server and native CLI browser harness |
| MP-02 identical run ID | A and B both run 1; per-route objectives, immutable legacy default; unknown and deleted scopes refused |
| MP-03 simultaneous workflows | Two real controlled executable agents in actual isolated Git worktrees, distinct environments/DB writes and commits; gate test keeps both active concurrently |
| MP-04 cancellation | A cancellation while B remains active then completes; close rejects active ownership |
| MP-05 terminal ownership | One TCP listener, real ConPTY/PTY sessions, authenticated WS output/input/resize, reconnect/history, SSE root/header scope, A stop while B responds, eventual map cleanup |
| MP-06 durable registry | Manager restart identity and deterministic listing; native server restart browser harness |
| MP-07 duplicate paths | Dot-relative canonicalization, spaces/Unicode; eight concurrent registrants/opens; native Windows case and junction aliases |
| MP-08 OS lock | Separate child process holding `runtime.lock`; other project opens; release and retry; Git common lock also covers another registered-root/registry owner |
| MP-09 V1 fixture | Frozen P00 SQL fixture opened through manager; runs/tasks/attempts/sessions/artifacts/GitHub linkage, config bytes and real clean worktree retained |
| MP-10 failure isolation | Moved root, corrupt project DB, unavailable startup default and unavailable authorized root; B remains usable; global corrupt registry intentionally fails closed |
| MP-11 frontend | Hook abort/generation/identity tests; real browser A request delay, switch to B, reload, same filename/agent/run ID |
| MP-12 Git/delivery | Separate actual repositories/worktrees/commits, distinct locally configured fake GitHub remotes in scoped delivery reports; unauthorized/not-ready delivery refused; existing fake-remote uncertain-publication regressions |
| MP-13 MCP/policy | Project-local grant/token hash/config byte checks; existing real controlled MCP stdio/HTTP and policy tests unchanged |
| MP-14 restart | Actual native server restart with several registry entries and preserved histories; `restart_reconciles_each_project_without_replaying_uncertain_delivery` seeds explicitly synthetic interrupted DB markers in A and B and checks existing reconciliation; original native CLI crash/recovery test |
| MP-15 fault injection | SQLite registry insert and open-timestamp abort triggers; failed clone; existing/partial Factory initialization refusal; migration rollback with verified backup; OS lock contention; canonical directory replacement and explicit identity-change relink |

`config_parse_failures_never_reflect_source_secrets_in_project_errors_or_listing` also checks that malformed config cannot expose a synthetic secret marker through open, listing or config responses while B remains usable.

Security-specific path tests reject out-of-scope destinations before creation, nested registered-root destinations, malformed IDs, DB hard links, overlapping repositories, Git-common sharing, symlink/junction escape and `.factory` redirection. Context API reuses the unchanged boundary-tested Factory Context engine. P03 does not add a filesystem editor API.

`child_lock_holder` is a child-process fixture test that returns immediately in ordinary discovery and takes the real OS lock only in its explicitly configured child process. It is not an ignored scenario. The existing two ignored diagnostic benchmark tests remain ignored in the ordinary workspace command and are reported separately; no test is disabled by P03.

## Qualification limits

Native execution here is Windows x86_64. Linux/macOS paths, locks, PTY behavior and packaging require exact-commit CI or native qualification; they are not inferred from Windows. The browser harness uses built/embedded production assets and a local server, not mocked HTTP data. Its databases are deliberately seeded controlled fixtures. Full real-provider missions, checkpoint recovery and browser-disappearance suspension belong to P08-P10/P14; P03 does not claim those validations.

Stable alias/replacement checks do not prove immunity to hostile OS-level TOCTOU replacement. Git worktrees and separate DBs are logical ownership boundaries, not an OS sandbox. No ordinary request may select an unregistered context, fall back on a different project, or share a process map keyed only by run/session ID.
