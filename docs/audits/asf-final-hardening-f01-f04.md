# ASF Final Hardening Report — F01 through F04

Date: 2026-10-26
Base commit: `2303535d7c7a0606f955469310d68d14b714f9f6` (the audited revision
after ASF-N01..N08; all work builds on it). Final revision: `2a56af5`.
Validation platform: Windows 11 x64 local (rustc 1.99.1 / cargo 1.99.1);
CI (Linux, Windows, macOS) — see §E.

## A. Environment

- Windows 11 (10.0.26200) x64, Git Bash, rustc 1.99.1 (58cf8f9c3).
- New dependency: `windows-sys 0.61` (target-gated to Windows, features
  `Win32_Foundation`, `Win32_Security`, `Win32_System_JobObjects`,
  `Win32_System_Threading`) in `factory-proc` for Job Object supervision.
  Already present in the workspace lock via other crates; no other
  dependencies added. No migrations. No API contract changes.
- `factory-context` gained a dependency on the existing `factory-proc`
  supervisor for bounded ripgrep execution.

## B. Issue matrix

### ASF-F01 — Symlink/repository-boundary safety in content search — CONFIRMED AND FIXED

- **Reproduction.** The fallback scanner decided traversal on
  `Path::is_dir()` and read files with `std::fs::read` — both follow
  symlinks. A repository link to an external directory was traversed, an
  external file's content entered results, and a `nested/back -> ../` cycle
  looped the walk forever (proven by a std-level probe and the bounded-time
  cycle test).
- **Fix.** `fallback_paths` now decides on `DirEntry::file_type()`:
  symlinks (and Windows junctions — std reports both as `is_symlink`) are
  never traversed and never read; non-file/non-dir special entries are
  skipped. Every path produced by the ripgrep subprocess is revalidated
  (`accept_candidate`): must `strip_prefix` the canonical root, must not be
  a symlink per non-following `symlink_metadata`, must pass the ignore
  policy, and must be within `MAX_INDEX_FILE_READ_BYTES` before any read.
  Policy (Test E): internal symlinks are skipped too — one policy for the
  index and both search paths.
- **Tests** (`crates/factory-context/tests/search_boundaries.rs`): external
  file link, external directory link, cycle (bounded-time proof), ordinary
  files with spaces/Unicode, internal link not followed, broken link, and
  the ripgrep-path revalidation test (outside-root and symlink candidates
  from a fake `rg` are rejected). Windows: junctions are exercised via the
  `mklink /J` fallback in `platform_symlink`; where a link type needs
  privileges the test skips with an explicit message, never silently.
- **Documented limit.** A path swapped for a symlink between the
  enumeration check and `File::open` (TOCTOU) is not addressed — closing it
  needs `O_NOFOLLOW`/reopen-by-handle; the static-boundary guarantee the
  finding asked for holds, adversarial races are not claimed.

### ASF-F02 — Slash-containing gitignore patterns — CONFIRMED AND FIXED

- **Reproduction.** `*.env` + `!dir/allowed.env`: the old parser compiled
  the negation as `(?:^|/)dir/allowed\.env$`, suffix-matching
  `nested/dir/allowed.env` and re-including ignored credential-bearing
  files. The scenario test fails against the pre-fix parser (verified by
  stashing the fix: FAILED pre-fix, PASSED post-fix) — and the bug class
  was pinned by the new differential matrix.
- **Fix.** Patterns are now classified per gitignore(5): slash-free
  patterns match basenames at any depth; patterns with an interior or
  leading slash anchor `^…$` to their contributing directory. `**` is
  translated context-aware (`**/a` and `c/**/d` → zero-or-more directories,
  `b/**` → everything inside), and a backslash now escapes the next
  character (`what\?q`, `star\*literal`, `\#`, `\!`, `\ ` are literals) —
  a second real defect the differential matrix exposed.
- **Tests.** `expanded_differential_matrix_matches_git`: 76 fixture paths
  (basename, slash-containing, root-anchored, directory-only, negations,
  parent-directory restrictions, nested `.gitignore` precedence,
  `info/exclude`, escapes, Unicode, `*`/`**`/`?` wildcards) compared with
  real `git check-ignore` under an isolated `core.excludesFile` — 0
  mismatches. `a_ignored_credential_file_cannot_return_through_any_context_path`
  proves an ignored nested `.env` with fake credentials is absent from the
  index, both search paths, resolution, and rendered context while the
  legitimately re-included file remains visible.
- **Lifecycle.** `ContextEngine` re-collects ignore rules on every
  `refresh()`; a same-instance test proves a live engine applies `.gitignore`
  edits (and their removal) without restart. Factory Core constructs an
  engine per operation, so production was not stale — the boundary now
  guarantees it for directly-held instances too. No filesystem watcher.

### ASF-F03 — Fully bounded process lifecycle — CONFIRMED AND FIXED

- **Reproduction.** The supervisor joined reader threads unbounded after
  the parent exited. A std-level probe proved the hazard on Windows: a
  `start /b` descendant holds the parent's stdout pipe, and
  `Command::output()` blocks indefinitely. The same fixture drove the new
  supervisor tests.
- **Fix** (`factory-proc`): the deadline now bounds the complete
  operation — supervision, draining, termination, and reader shutdown.
  Reader threads are joined by a collector thread observed through a
  channel with `recv_timeout` slices; a stuck drain (descendant holding a
  pipe) escalates: graceful termination, then a forced kill after the
  deadline or a cancellation slice, then a bounded 5 s grace before the
  collector is deliberately detached (documented leak: bounded memory,
  only possible when a process outside the group/job still holds a
  descriptor). Cancellation is honored during the drain wait, not just
  while the parent lives. A deadline exceeded during draining classifies
  as `TimedOut` — never a success with silently incomplete output (a
  classification bug found and fixed during testing). Reader failures and
  panics now surface as `ProcError::Io` instead of silent empty captures.
  Windows termination uses a **Job Object** (`KILL_ON_JOB_CLOSE`), so
  descendants that outlive the parent — which `taskkill /PID` cannot reach
  — die with one `TerminateJobObject` call; Unix keeps process-group
  SIGTERM with SIGKILL escalation.
- **Tests** (`factory-proc`): parent-exits-descendant-holds-stdout / -stderr
  / -both (bounded `TimedOut` + heartbeat proves the descendant died),
  cancellation bounded with pipes held, SIGTERM-ignoring descendant killed
  by escalation (Unix; Windows `TerminateJobObject` cannot be trapped),
  plus all pre-existing supervision tests. GitHub delivery semantics are
  unchanged and re-tested: `git push`/`gh pr create` timeouts still map to
  `Uncertain` → `interrupted` → reconciliation; read-only timeouts remain
  definitive failures; ASF-18 claims and operation guards untouched.

### ASF-F04 — Bounded context search — CONFIRMED AND FIXED

- **Reproduction/fix.** The fallback scanner read whole files before the
  size check; now every read goes through a handle-metadata gate plus a
  `take(limit + 1)` reader, so memory stays bounded even when a file grows
  during the read (a file crossing the limit mid-read is rejected, not
  parsed). Traversal is bounded at `MAX_INDEX_ENTRIES` visited entries and
  1,000 matched paths (bounded partial scans, documented as such). The
  ripgrep path now runs under `factory-proc`: 30 s deadline
  (`FACTORY_RG_TIMEOUT_SECS` override), 1 MiB stdout / 64 KiB stderr
  capture, NUL-separated output (exact `OsString` bytes on Unix), explicit
  exit-code semantics — exit 1 ("no matches") is authoritative and does
  NOT trigger a fallback scan; exit ≥2, spawn failure, timeout, or
  truncated output falls back to the bounded scanner. Fake-`rg` tests
  cover hang, failure, no-match, truncation, and the F01 revalidation
  matrix; every returned path is re-seeeded through the ignore policy and
  boundary checks. Result limits, dedup, and deterministic ordering are
  preserved.

## C. Security evidence

- Boundary: link targets outside the repository are neither traversed nor
  read; cycles terminate; external-tool paths are revalidated against the
  canonical root; oversized candidates are rejected before reading.
- Gitignore: 76/76 differential agreement with real `git check-ignore`
  (isolated config); fake-credential exclusion verified through index,
  both search paths, resolution, and rendered context.
- Ripgrep/fallback consistency: both paths share `IgnoreRules`, the
  non-symlink rule, the size gate, and the boundary revalidation.
- Process: deadline bounds the whole operation including pipe drains;
  Windows tree cleanup via Job Objects; Unix group TERM→KILL escalation;
  descendant death proven by heartbeat fixtures; delivery uncertainty
  semantics preserved (22 delivery tests green).
- Streaming redaction (ASF-N01), bounded git/gh capture (N02/N03),
  durable delivery (ASF-18), and the lean evaluation projection (N06) are
  untouched and re-verified by the full workspace suite.

## D. Resource bounds (current constants)

| Bound | Value |
| --- | --- |
| File read (context search/index) | 1 MiB (`MAX_INDEX_FILE_READ_BYTES`), `take()`d |
| Fallback traversal | 50,000 entries (`MAX_INDEX_ENTRIES`) |
| Matched-path storage | 1,000 (`MAX_MATCHED_PATHS`) |
| Ripgrep deadline | 30 s (`FACTORY_RG_TIMEOUT_SECS` override) |
| Ripgrep capture | 1 MiB stdout / 64 KiB stderr |
| Search result count | caller `limit` |
| Post-kill drain grace | 5 s (`HARD_KILL_GRACE`) |
| Supervisor captures | per-caller `Bounds` (unchanged from N02) |

## E. Cross-platform status

- **Windows**: full local validation — all suites, both test-thread modes,
  symlink/junction fixtures, Job Object descendant cleanup, dashboard
  gates — and green CI.
- **Linux/macOS**: green CI on the final revision (run 40350905987,
  commit 2a56af5): `sh` fixtures, process groups, held-pipe descendant
  deadlines, SIGTERM-ignoring descendant escalation, gitignore
  differential against git. Two CI-only fixture issues were found and
  fixed on the way: the search-boundary suite now serializes under the
  env lock (CI runners have no ripgrep, and a concurrently installed fake
  rg leaked into parallel plain tests as their tool), and the Unix
  descendant fixture's single quotes prevented $(dirname) substitution.

## F. Regression status

All prior campaigns' suites pass unchanged in the final workspace run
(serial and parallel): factory-policy (62, streaming redaction),
factory-proc (15), factory-git (21 incl. bounded execution),
factory-github (16), factory-context (65 + 15 new boundary/differential
tests), factory-db (36, lean projection), factory-eval (22),
factory-core (all integration suites incl. streaming redaction,
github delivery 22, policy enforcement), factory-api, dashboard (303).
Local gate totals: `cargo fmt --all -- --check` PASS;
`cargo clippy --workspace --all-targets -j 1 -- -D warnings` PASS;
`cargo test --workspace -j 1 -- --test-threads=1` PASS (606 tests);
`cargo test --workspace -j 1` (CI threading) PASS (729 executions);
dashboard format/lint/typecheck PASS, tests 303/303, build PASS. One
environmental hiccup (MSVC LNK1140 PDB-size limit from a 32 GB accumulated
target dir) was resolved by deleting stale PDBs — not a code failure.
GitHub Actions: PASS on the final revision — run 40350905987
(commit 2a56af5): Rust on ubuntu/windows/macos, Dashboard, installer
end-to-end, embedded-dashboard smoke, and release-checksum jobs all
green (after intermediate runs 40342933556 and 40348063974 surfaced the
two fixture issues above).

## G. Remaining risks

| Risk | Severity | Status |
| --- | --- | --- |
| TOCTOU path-swap between non-symlink check and `File::open` in search reads | low | documented, unaddressed (needs `O_NOFOLLOW`/handle reopen); static boundary guarantees hold |
| Deliberately-escaped descendants (explicit `setsid`/job breakaway) can keep pipes open; collector thread detached after grace | low | documented limit; bounded memory; `TimedOut` classification preserves delivery safety |
| Successful commands leave no orphans: job guard drop kills leftover background descendants of Factory-owned commands | informational | intended behavior for git/gh supervision |
| Windows junction behavior depends on std reporting reparse points as symlinks | informational | verified on Windows 11 / current rustc; fixture asserts behavior, not internals |
| Gitignore parity is differential-tested across the supported subset; exotic constructs (e.g. `[a-z]` classes) remain outside the parser | informational | documented subset; differential suite pins what is supported |

No newly discovered defects remain open.
