# P14 qualification test matrix

Mapping is not a PASS claim. Each result must identify its exact source, native
environment, command, timestamp and evidence hash. Missing receipts remain
UNVERIFIED; unavailable authorization/hardware is BLOCKED. Historical tests remain
historical. The authoritative G01-G25 ledger and final delivery record dispositions.

## BUILD

| ID | Required behavior | Candidate evidence / procedure |
| --- | --- | --- |
| BUILD-01 | Clean Rust production build | run.py build from the verified clean checkout and empty release target |
| BUILD-02 | Clean frontend build | npm ci and run.py frontend in the clean checkout |
| BUILD-03 | Locked dependencies | cargo --locked, npm ci, lockfile hashes |
| BUILD-04 | Embedded asset integrity | Rust embedded tests and smoke.py verifies every served asset hash |
| BUILD-05 | No developer checkout dependency | Standalone package starts outside checkout; inspect native library dependencies |
| BUILD-06 | Version metadata | package.py compares actual CLI version with workspace version |
| BUILD-07 | Repeat consistency | Repeat optimized build and archive hashes under equivalent inputs |
| BUILD-08 | Native architecture | host triple and actual PE/ELF/Mach-O inspection |
| BUILD-09 | Artifact hashes | Independent release-checksums verification plus per-file manifests |
| BUILD-10 | Packaged launch | smoke.py and production browser against unpacked candidate |

## INST

| ID | Required behavior | Candidate evidence / procedure |
| --- | --- | --- |
| INST-01 | Windows fresh install | Native portable/local-download installer campaign |
| INST-02 | Linux native install | Native GNU tarball, non-admin smoke and runtime dependencies |
| INST-03 | macOS ARM64 install | BLOCKED: no native local machine; prepared CI is not PASS |
| INST-04 | macOS x86_64 install | BLOCKED: no native local machine; prepared CI is not PASS |
| INST-05 | Spaces in path | smoke.py uses a new installation path containing spaces |
| INST-06 | Unicode path | smoke.py uses actual Unicode project/application paths |
| INST-07 | Non-admin launch | Observed token/UID and native packaged startup |
| INST-08 | Port collision | Second packaged server must fail before state mutation |
| INST-09 | Upgrade preserves data | Actual previous P13 preview and current candidate on the same disposable project |
| INST-10 | Uninstall preserves repositories | Remove only verified portable application tree; independently inspect repository/database |

## DB

| ID | Required behavior | Candidate evidence / procedure |
| --- | --- | --- |
| DB-01 | Fresh schema | factory-db and native package initialization |
| DB-02 | Supported prior schema | Authentic V1 schemas/migrations plus actual P13-preview upgrade |
| DB-03 | Backup | migration_backup tests and preserved pre-upgrade snapshots |
| DB-04 | Interrupted migration | Transaction rollback/interruption fixtures; no reinitialization workaround |
| DB-05 | WAL recovery | Native SQLite WAL/restart/backup tests and package quick_check |
| DB-06 | Corruption | Corruption refusal fixtures and unchanged original data |
| DB-07 | Concurrent access | DB/API CAS, busy handling and concurrent readers/writers |
| DB-08 | No schema downgrade | Unknown/newer/noncontiguous schema rejection |
| DB-09 | Project isolation | factory-projects, memory/conversation/files API cross-project tests |
| DB-10 | Checkpoint durability | DB/runtime lifecycle checkpoint/restart tests |

## RUN

| ID | Required behavior | Candidate evidence / procedure |
| --- | --- | --- |
| RUN-01 | Project registration | Native package and multi_project API tests |
| RUN-02 | Real file editing | P05 CAS and production CodeMirror save journey |
| RUN-03 | LSP initialization | Eight optional native IDE/protocol tests with real approved tools; separate from default suite |
| RUN-04 | Real terminal | Runtime PTY/ConPTY and IDE/browser tests |
| RUN-05 | Debug adapter | Real approved DAP tests with target processes |
| RUN-06 | Extensions | Declarative activation/permission tests and UI |
| RUN-07 | Conversation persistence | factory-conversation and API/browser tests |
| RUN-08 | Genuine Worker | Fresh genuine model journey on release candidate |
| RUN-09 | Genuine Reviewer | Actual captured nonempty diff and recorded decision |
| RUN-10 | Integration | Authoritative coordinator plus independent Git/test inspection |
| RUN-11 | Parallel scheduling | dynamic_scheduler, stress and genuine historical evidence identified separately |
| RUN-12 | Autonomous repair | Repair/recovery fixtures and retained genuine R1 evidence |
| RUN-13 | Safe handoff | Profile/handoff/runtime tests and controlled browser journey |
| RUN-14 | Suspension | Lifecycle tests and representative default real timing |
| RUN-15 | Manual resume | Checkpoint validation, remaining attempts and no duplicate integration |
| RUN-16 | Memory retrieval | Memory tests plus real invocation revision audit |
| RUN-17 | Git staging | Actual file/hunk staging with stale guards |
| RUN-18 | Local commit | Exact index commit and independent content/hash inspection |
| RUN-19 | Notifications | Genuine runtime events, dedup/read/dismiss persistence |
| RUN-20 | Workspace restore | Versioned layout and production reload without side-effect replay |

## SEC

| ID | Required behavior | Candidate evidence / procedure |
| --- | --- | --- |
| SEC-01 | Local server authorization | Actual API security tests and packaged unauthenticated denial |
| SEC-02 | Origin denial | Actual Host/Origin/DNS-rebinding boundary tests |
| SEC-03 | CSRF denial | Actual mutation tests and packaged omission refusal |
| SEC-04 | Project isolation | Multi-project filesystem/memory/conversation/Git tests |
| SEC-05 | Protected filesystem | P05 path, reparse/symlink, hardlink and protected-root tests |
| SEC-06 | Agent permissions | factory-policy/agent/core effective deny precedence tests |
| SEC-07 | Reviewer restrictions | Read-only profile/actual captured evidence and native review |
| SEC-08 | Unauthorized Git mutation | Protected worktree/ref, literal paths and stale-index tests |
| SEC-09 | Unauthorized GitHub publication | Production provider-boundary authorization tests |
| SEC-10 | Memory injection | Source validation and memory cannot alter effective policy |
| SEC-11 | Extension permissions | Actual declarative extension permission boundary |
| SEC-12 | Process ownership | Owned process metadata, group/job cleanup and interruption tests |
| SEC-13 | Secret redaction | Known-secret fixtures, generic notifications, artifact inventory |
| SEC-14 | Deep links | Project-scoped resource resolution and UI stale-target tests |
| SEC-15 | Lease ownership | Authenticated identity/client/generation/sequence lifecycle tests |
| SEC-16 | Safe resume | Checkpoint identity/Git/policy/idempotency tests |
| SEC-17 | Stale writes | Actual expected file revision conflict |
| SEC-18 | Duplicate integration | Claims/journals/CAS and repeated-request regressions |
| SEC-19 | Remote uncertainty | Actual provider-contract timeout/reconciliation cases; no blind retry |
| SEC-20 | Artifact privacy | Allowlisted package inventory, dependency notices, no credentials/source runtime state |

## PERF

| ID | Required behavior | Candidate evidence / procedure |
| --- | --- | --- |
| PERF-01 | Startup | Controlled candidate cold/warm startup samples |
| PERF-02 | Workspace restoration | Production browser through durable restored presentation |
| PERF-03 | Project switch | Correct project/resource identity and latency |
| PERF-04 | Editor load | Actual CodeMirror tab timings and 100 open/close cycles |
| PERF-05 | Git status | Optimized 100/1000/5000/6000-file fixtures |
| PERF-06 | Git diff | Bounded actual text diff samples |
| PERF-07 | Hunk staging | Exact displayed-version selected hunk timing |
| PERF-08 | Large commit | Repeated 6000-file stage/commit with independent contents |
| PERF-09 | Memory creation | Validated ordinary API creates and multiple revisions |
| PERF-10 | Memory retrieval | Bounded search/context samples at increasing collection sizes |
| PERF-11 | Memory rebuild | Derived index rebuild with authoritative records intact |
| PERF-12 | 10000-memory fixture | Original P11 four-writer workload and longer ordinary payload; no reduced count |
| PERF-13 | Scheduling | Actual bounded 2/4/8 claims/capacity fixture |
| PERF-14 | Suspension | Measured process quiescence/checkpoint latency |
| PERF-15 | Resume | Authorized remaining-work continuation latency |
| PERF-16 | Browser heap | Method-labelled retained/GC heap and DOM measurements |
| PERF-17 | Process stability | Repeated tool/terminal lifecycle and owned-process observations |
| PERF-18 | Test memory | Whole unchanged test suite, bounded workers, measured process tree; no unrestricted claim |

## A11Y

| ID | Required behavior | Candidate evidence / procedure |
| --- | --- | --- |
| A11Y-01 | Keyboard core journey | Production keyboard-only navigation/edit/save/settings/notifications |
| A11Y-02 | Dialog focus | Trap/restoration component and browser tests |
| A11Y-03 | Editor keyboard | CodeMirror commands/Tab mode and composition guard |
| A11Y-04 | Terminal keyboard | Focused terminal shortcuts and actual PTY input |
| A11Y-05 | Native screen reader | Actual native AT output required; detection/Axe alone insufficient |
| A11Y-06 | Announcements | Real save/error/mission/notification live-region tests |
| A11Y-07 | Both themes | Axe plus actual contrast/visual inspection |
| A11Y-08 | Narrow desktop | 1920x1080 through 1024x768 production captures |
| A11Y-09 | Reduced motion | System preference and production browser checks |
| A11Y-10 | IME composition | Synthetic guard tests distinguished from native installed IME journey |

## REL

| ID | Required behavior | Candidate evidence / procedure |
| --- | --- | --- |
| REL-01 | Source identified | Frozen source, ancestry, fingerprints and artifact bindings |
| REL-02 | Exact remote CI | BLOCKED until source published with explicit authorization and matching jobs pass |
| REL-03 | Architecture | Inspect actual produced executable, not archive name |
| REL-04 | Hash integrity | Archive/per-file hashes independently verified |
| REL-05 | Package installation | Actual unpacked/installed candidate journey |
| REL-06 | Upgrade | Actual prior preview and authentic migrations |
| REL-07 | Vulnerabilities | Dated RustSec/npm results with reachability/severity disposition |
| REL-08 | Native platforms | Every mandatory target; blocked/unverified never PASS |
| REL-09 | Release notes | Implementation and actual qualification claims separated |
| REL-10 | Publication boundary | No push/tag/PR/release/deploy; explicit handoff only |

## Reproduction

Use `scripts/v2_p14/README.md` for frozen-source commands. The unchanged default
Rust suite retains optional native-tool/diagnostic ignores; real-tool execution is
reported separately. Frontend tests retain the configured two workers. No test is
removed to make a platform fit. Failed/changed-source attempts are retained, and
only unchanged-source successful receipts can qualify the candidate.

## Frozen candidate execution summary

Candidate `the historical private baseline`. These execution summaries scope the required cases above. The receipt index retains environment, source, command and evidence hashes. The gate ledger determines the final aggregate decision.

| Cases | Exact candidate observations | Evidence |
| --- | --- | --- |
| BUILD-01–06, BUILD-08–10 | PASS available Windows and Linux builds, version/arch, 68 embedded assets and standalone launch. macOS portion BLOCKED. | Windows package (historical record held privately), Linux package (historical record held privately) |
| BUILD-07 | Same-executable packaging repeat passed on produced targets. Independent Linux repeat-link bytes differ. Windows second clean linker run remains unverified. | Native repeat (historical record held privately), ELF analysis (historical record held privately) |
| INST-01–02, INST-05–08, INST-10 | PASS produced Windows and Linux portable packages, native isolated profiles, path/port checks and user-data-preserving removal. | Windows installer (historical record held privately), Linux installer (historical record held privately) |
| INST-03–04 | BLOCKED macOS machines and exact-source runners. | [Platform scope](release/SUPPORTED-PLATFORMS.md) |
| INST-09, DB-01–10 | PASS authentic V1 migrations, Windows preview projects and restored browser draft. Both available native binaries exercised backup/restore, interruption, WAL, busy and corrupt/unsupported schemas. | Windows database (historical record held privately), Linux database (historical record held privately), prior workspace (historical record held privately) |
| RUN-01–07, RUN-17–20 | PASS browser and installed local operations. Optional native IDE services pass separately. | Browser (historical record held privately), Windows IDE tools (historical record held privately), Linux IDE tools (historical record held privately) |
| RUN-08–10, RUN-16 | PASS genuine Windows Planner/Worker/Reviewer, integration, independent tests, recorded memory revisions. | Genuine coding (historical record held privately) |
| RUN-11–13, SEC-17–18 | PASS native deterministic claims and recovery, actual Git worktree/conflict repair and scripted handoff. | Parallel and conflict (historical record held privately), handoff (historical record held privately) |
| RUN-14–15, SEC-12, SEC-15–16 | PASS real timing and verified process exit, suspension, checkpoint and manual resume. Native restart and ownership fixture tests pass. | Windows leases (historical record held privately), Linux leases (historical record held privately) |
| SEC-01–14, SEC-17–20 | PASS tested Rust guards, process-scoped Reviewer checks, GitHub provider fixtures and produced artifact inventories. Live GitHub write gate is BLOCKED. | Security corrections (historical record held privately), Windows artifact (historical record held privately), Linux artifact (historical record held privately) |
| PERF-01–04, PERF-16–18 | Measured warmed/saved workspace, repeated editor/terminal and bounded Vitest resources. Long-run desktop coverage UNVERIFIED. | [Performance results](release/PERFORMANCE-RESULTS.md) |
| PERF-05–08, PERF-13–15 | Real Git and lease timings recorded. Bulk Git fails thresholds. Scheduler, package lifecycle and short-stress fixtures passed behavior checks. Some per-operation timing distributions remain unqualified. | [Performance results](release/PERFORMANCE-RESULTS.md), gates (historical record held privately) |
| PERF-09–12 | PASS original 10,000-memory workload and read-only SQLite/index verification. | Memory workload (historical record held privately) |
| A11Y-01–04, A11Y-06–09 | PASS targeted production keyboard, multi-engine Axe, theme and responsive checks at recorded scope. Incomplete Axe items and minor visual observations retained. | [Accessibility procedure](release/ACCESSIBILITY-PROCEDURE.md), screenshots (historical record held privately) |
| A11Y-05, A11Y-10 | BLOCKED native AT, UNVERIFIED physical CJK IME. Synthetic composition guard covered. | Native AT tooling (historical record held privately) |
| REL-01, REL-03–06, REL-09–10 | PASS local source identity, actual produced archives, observed available installations and upgrades, documentation, authorization boundary. | [Artifact manifest](release/ARTIFACT-MANIFEST.md), gate decision (historical record held privately) |
| REL-02, REL-08 | BLOCKED exact remote CI and mandatory native macOS targets. | Remote source check (historical record held privately) |
| REL-07 | FAIL mandatory high severity audit threshold. Dated build tool audit reports a `picomatch` advisory. | [Dependency disposition](release/DEPENDENCY-AUDIT.md) |
