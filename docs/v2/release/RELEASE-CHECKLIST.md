# Mandatory release decision

Run `python -B scripts/v2_p14/gates.py INPUT --evidence-root ROOT --output REPORT`.
Evidence must exist, match its SHA-256 and identify the exact candidate source.
The evaluator is an auditable checklist, not an authorization service.

| Gate | Required evidence |
| --- | --- |
| G01 | Clean integrated source, verified ancestry, frozen inputs |
| G02 | Locked full serial and normal-threaded Rust suites |
| G03 | Full frontend formatting/lint/types/tests/build |
| G04 | Actual authorization/security regressions |
| G05 | Native Windows x86_64 runtime/package/install |
| G06 | Native Linux x86_64 runtime/package/install |
| G07 | Native macOS ARM64 runtime/package/install |
| G08 | Native macOS x86_64 runtime/package/install |
| G09 | Required production artifacts; missing targets explicit |
| G10 | Fresh isolated installation/launch/persistence |
| G11 | Actual prior-version upgrade, backup and migration recovery |
| G12 | V1 entry, workflows, CLI/API/database compatibility |
| G13 | Genuine Planner/Worker/Reviewer on the candidate, nonempty code, independent tests/Git |
| G14 | Parallel claims/worktrees, repair and interruption recovery |
| G15 | Browser/session leases, controlled process suspension, durable manual resume |
| G16 | Memory revisions, provenance, eligibility/deletion, actual invocation audit |
| G17 | Git HEAD/index/worktree integrity, stale hunks and protected worktrees |
| G18 | GitHub authorization/idempotency plus explicitly authorized live delivery qualification |
| G19 | Keyboard/both-theme accessibility, native AT and input limitations resolved or approved |
| G20 | Controlled release-build performance and stability, 10,000 memories / 6,000-file staging |
| G21 | Dated vulnerability/license/supply-chain disposition |
| G22 | Native architecture and archive/file hashes verified after packaging |
| G23 | Matching remote four-target CI run/job evidence |
| G24 | Accurate installation, upgrade, security, user and release documentation |
| G25 | No unresolved critical/high blockers |

GO also requires explicit authorization for the exact release source. BLOCKED,
UNVERIFIED, FAIL and a mandatory NOT_APPLICABLE all prevent GO. An unsigned/unbuilt
target must not be described as signed/built. A scope reduction is a product
decision requiring explicit approval, not a checklist workaround.

No source or artifact is pushed, tagged, released, uploaded or deployed by the local
evaluator. Current source publication and remote qualification remain blocked.
