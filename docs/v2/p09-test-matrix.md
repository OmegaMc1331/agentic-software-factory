# P09 qualification matrix

The delivery report and evidence manifest distinguish controlled deterministic fixtures from actual model-driven acceptance. Passing a process fixture does not qualify a provider.

| Contract | Deterministic evidence | Native acceptance |
| --- | --- | --- |
| Durable CRUD, stable IDs, archive, legacy compatibility, revisions | `factory-core/tests/agent_profiles.rs`: profiles/revisions, concurrent edits; API profile history tests | Workspace create/reload/disable/history journey |
| Safe provider/model selection | Core invalid inputs and structured argv tests; API rejects command/env injection and redacts provider metadata | Actual native CLI header and immutable session snapshots |
| Project isolation, auth, CSRF, Origin | `factory-api/tests/agent_profiles.rs` plus P03 security suites | Disposable independent projects |
| Policy intersections and reviewer read-only | Core project/role/installation denial and read-only replacement tests; existing policy/MCP suites | Rejected read-only handoff with no permission change |
| Active snapshot immutability and stale launch rejection | Active edit test, changed handoff revision test, database snapshot CAS | Actual selected model/mode retained in session history |
| Pending handoff, idempotency, conflicting clients | Pending reopen and conflicting request tests | Pending task assignment in production workspace |
| Running handoff and correct context | Controlled Worker exit, preserved aggregate patch, separate replacement attempt and Reviewer | Genuine Worker-to-Worker handoff |
| Parallel custom identities and capacity | Distinct custom profile parallel fixture; existing P08 global/project/agent capacity tests | Real overlapping native Worker intervals |
| Replanning retains profile eligibility and scope | `replan_uses_eligible_profile_snapshots_without_widening_source_scopes`: actual controlled process failure, Planner revision, profiles and review | P08/P08-R1 native replanning evidence retained; P09 native mission does not repeat it |
| Review/integration boundary | Review-time rejection; original P08 serialization and conflict tests | Native reviews, verified integration branch and fresh checkout |
| Restart and uncertain side effects | Handoff interruption reconciliation, P08 crash/claims tests | P08-R1 retained live interruption qualification; P09-specific pending recovery |
| Quota and permission refusal | Existing authorization/resource-exhaustion and P07-R1 permission tests; no automatic model fallback | Stop on any actual provider refusal; record unavailable evidence accurately |
| Frontend behavior | `src/v2/agents/agents.test.tsx`: stale drafts, selected configuration, idempotent retry, pending state, historical models | Production browser journeys and inspected screenshots |

Full workspace gates, P03-P08 and V1 regressions remain required. Platform and exact-commit remote CI claims require their own execution evidence. No P10 suspension qualification is inferred.

## Recorded local qualification

The delivery (historical record held privately) and evidence manifest (historical record held privately) record 933 passing Rust tests in both full modes, 19 focused Core profile tests, four API suites, 515 frontend tests in both worker configurations, eight explicit IDE-tool integrations, 104 existing browser regressions and nine P09 replay checks.

Native scenario A is the actual profile-A source implementation; scenario B is verified Codex low/medium configuration separation; scenario C is the session-2 to session-4 handoff and substantive replacement correction; scenario D is two retained read-only refusals without a process; scenario E is sessions 2 and 3 overlapping for 39,470 ms in separate worktrees. All are one bounded real mission, with three independent native review invocations, serialized integration and fresh verification. Cross-provider execution remains unqualified.
