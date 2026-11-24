# P10 test matrix

This matrix distinguishes deterministic regression coverage from real coding acceptance. Final executed counts, binary provenance and remaining qualification limits belong to deliveries/V2-P10.md (historical record held privately) and the hashed evidence manifest. A controlled process is never counted as a model-driven implementation.

| Requirement | Automated coverage / observation |
| --- | --- |
| LEASE-01-04, 10-12 | `factory-api::presence::tests`: registration, ordered renewals, strict owner/client/generation/sequence, expiry/revocation, independent windows, restart invalidation; `factory-api/tests/lifecycle.rs` exercises real authentication, Origin and CSRF middleware. |
| LEASE-05-07 | Server-state grace/reload/project-isolation test; production browser acceptance closes two independent Edge clients in sequence and observes the owning Worker and backend lifecycle. |
| LEASE-08-09 | Frontend presence lifetime tests and production workspace ownership test; there is no visibility/focus/unload suspension handler. Browser acceptance navigates projects and panels while agents remain active. |
| SESSION-01-06 | DB ownership/close/reopen/legacy tests; API exact project/session controls; runtime close-reopen test; presentation navigation remains separate from close controls. |
| SUSP-01, 08, 10 | DB dispatch/scheduler fence, completion precedence and duplicate command tests; runtime replay of an old suspend command during a newer live generation. |
| SUSP-02-04 | Runtime owned-Worker drain, retained partial files and interrupted Reviewer tests. Confirmed cleanup metadata is required before checkpoint creation. |
| SUSP-05 | Integration-validation cancellation preserves a suspended candidate journal; resume revalidates it without rerunning the approved Worker. Uncertain publishing remains recovery-required. |
| SUSP-06 | P08 revision application and invoke admission check the durable lifecycle fence; committed revision is checkpointed. Existing dynamic scheduler revision/barrier regressions remain required. |
| SUSP-07 | Runtime pending-handoff suspension/resume test checks the original Interrupted attempt remains byte-for-byte unchanged and the replacement receives a distinct execution. |
| SUSP-09 | Core checkpoint process-tree verification, DB drained-operation requirements and P08 owned-process interruption tests; cleanup uncertainty cannot commit Suspended. |
| CHK-01-04, 06 | DB checkpoint durability and ownership tests; runtime restart/partial-work validation; actual Git heads, source bases, plan revision and worktree fingerprints. |
| CHK-05, 07 | Runtime owned-database corruption cases mutate schema, project, plan and commit fields; resume refuses without new Worker execution. Changed retained worktree test is independent. |
| CHK-08 | Checkpoints store bounded references/states/hashes, not prompts, raw subprocess output, credentials or inference reasoning. API acceptance artifacts retain selected metadata and hashes. |
| RES-01-06 | Runtime completed predecessor + interrupted dependent mission; existing attempt and integration commit counts are checked. Reviewed candidate resume retains review requirements. |
| RES-07-09 | Runtime missing-provider and newly read-only-policy resume test; current core policy/team/profile/provider checks; P09 pending-target revision validation; runtime handoff test and existing profile/policy regression suites. |
| RES-10-12 | Durable resume command test, runtime repeated cycles and restart test; browser repeats resume requests, reloads history and verifies unique rendered message IDs. |
| MP-01-05 | API same numeric run ID across projects, presence scoped binding tests, legacy unmanaged compatibility; runtime shared P08 budget and process ownership remain authoritative. Corrupt metadata in one project does not block another project's absence handling. |
| Repeated cycles and limits | Runtime four suspensions followed by verified completion; interrupted attempts do not consume code-quality attempt budget, active scheduler duration accumulates and completed integration is not republished. |
| Presence storage/backpressure | Twenty clients renew without growing the live registry; SQLite refusal does not extend in-memory lease validity. Bounded request concurrency, lease/audit/checkpoint counts and sizes. |
| Real scenarios A-E | `scripts/v2_p10/acceptance.mjs --native`: actual form submission/Planner/Worker, two browser clients, final lease expiration, reopen/manual resume, isolated explicit close, abrupt owned-runtime kill, restart reconciliation, genuine review/integration and fresh-checkout independent tests. |
| Model-free end-to-end setup | The same harness without `--native` uses explicitly labelled deterministic Workers, never model acceptance. |
| P03-P09 / V1 | Full Rust/frontend suites, existing production browser regressions, P09 profile/handoff tests and P06 explicit-tool tests. Historical skipped/platform cases stay explicit. |

Failure evidence is retained rather than overwritten. A passing API-only lease test does not prove browser process behavior. Windows execution does not qualify POSIX signal/group handling or macOS sleep/wake. No remote CI is implied by local gates.

Final concurrency regressions cover explicit close during pending resume, a newer suspend defeating an older resume commit, eight simultaneous same-key suspension requests producing one checkpoint/event chain, and bounded no-op/failed-resume command history with a reserved safety-fence slot. The two DB safety cases were reproduced as failing tests before their correction; both logs and final results are retained.
