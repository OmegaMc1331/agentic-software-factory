# ASF Stabilization Report — N01 through N08

Date: 2026-10-26
Base commit: `1e2b99ea1199be3942d65797744b980d0afbfccd` (the previously
reviewed revision; all work below builds on it)
Validation platform: Windows 11 x64 (local), rustc 1.99.1, cargo 1.99.1.
CI (Linux, Windows, macOS): pending push — this report is written from local
validation; GitHub Actions results were not yet available at authoring time.

## A. Environment

- Rust 1.99.1 (58cf8f9c3 2026-10-13), cargo 1.99.1 (5f94df478 2026-08-27).
- Operating system used for local validation: Windows 11 (10.0.26200) x64,
  Git Bash. Platform-specific paths (process-tree termination via
  `taskkill /T`, `.cmd` fake binaries) were exercised natively; the Unix
  branches are compiled on CI.
- New workspace crate: `factory-proc` (std-only bounded subprocess
  supervisor). No other dependencies were added; `tempfile` moved from a
  dev-dependency to a dependency of `factory-github` (used for the PR body
  temp file). No schema migrations.

## B. Issue matrix

### ASF-N01 — Streaming secret redaction — CONFIRMED AND FIXED

- **Suspected defect:** `Secrets::redact` was applied independently to each
  decoded output chunk before `append_agent_session_output`, so a secret
  split across process write boundaries reached the incremental SQLite rows
  (and therefore the SSE session stream, which replays those rows).
- **Reproduction:** confirmed by code inspection of the callback boundary
  (`factory-core/src/factory.rs`) and by the new gated-writer integration
  test: a fake worker emitting `stream-secret-` and `7f4d2a3b` in two
  separate writes made the partial fragment visible in the persisted row
  before the fix.
- **Root cause:** stateless redaction per chunk.
- **Fix:** `factory_policy::StreamingRedactor` (`crates/factory-policy/src/redact.rs`)
  — per-stream stateful matcher. It redacts complete occurrences
  (leftmost-longest, matching the whole-text function's ordering) and holds
  back exactly the longest suffix that is a proper prefix of some known
  secret, emitting only the provably-safe head. `invoke_with_agent` keeps
  two independent redactors (stdout/stderr), and each stream's held tail is
  flushed after the run — on success, cancellation, and failure paths —
  before session finalization. Final rows keep the whole-text redaction of
  the bounded capture. Retained memory is bounded by the longest configured
  secret (hard-capped at 1 MiB); UTF-8 boundaries are preserved by
  construction (the hold always starts at a character boundary).
- **Files:** `crates/factory-policy/src/redact.rs` (new),
  `crates/factory-policy/src/environment.rs`, `crates/factory-policy/src/lib.rs`,
  `crates/factory-core/src/factory.rs`,
  `crates/factory-core/tests/streaming_redaction.rs` (new).
- **Tests:** 15 redactor unit tests (two-fragment and many-fragment splits,
  exhaustive split points, adjacent and overlapping secrets, prefix-of-
  another secrets, stream start/end, unicode secrets split at character
  boundaries, cancellation-flush, bounded retention under flood,
  deterministic pseudo-random splits, >1 MiB secret, stdout/stderr
  independence); 2 factory-core integration tests observing the persisted
  rows mid-run through a second SQLite connection (hold-back while running)
  and after cancellation.
- **Limitations (documented in `docs/security.md` and `docs/policies.md`):**
  only secrets Factory knows are covered; encoded/transformed secrets are
  not detected; secrets longer than 1 MiB are matched within a chunk and in
  final whole-text redaction, but cross-chunk streaming matching is not
  guaranteed for them; an unterminated prefix flushed at end-of-stream is
  released verbatim (it is not the secret and cannot be completed).

### ASF-N02 — Bounded Git/GitHub subprocess execution — CONFIRMED AND FIXED

- **Suspected defect:** `gh.rs` used `Command::output()` and several
  factory-git paths used `Command::status()` with no deadline, no output
  bound, and no tree termination; a wedged `git`/`gh` could occupy a
  delivery worker forever.
- **Reproduction:** new `factory-proc` primitive tests (never-exiting
  command killed at deadline; endless stdout/stderr flooding flagged and
  capped; descendant process terminated with the tree; slow-but-finishing
  command succeeds; stdin readers get EOF immediately), plus fake-`gh` and
  fake-`git` integration tests (hanging `gh pr create` times out into an
  Uncertain classification; hanging `git` killed; runaway output rejected).
- **Fix:** new leaf crate `factory-proc` (`ProcCommand`, `Bounds`,
  `ProcOutput`, `ProcError`) — structured args, null stdin, piped
  stdout/stderr drained concurrently into bounded head-captures with
  truncation flags, deadline + optional cancellation flag, process-group /
  `taskkill /T` tree termination, guaranteed reaping. All factory-git and
  factory-github execution now routes through it:
  - per-operation deadlines (gh: 30–180 s; git local: 120 s; network git:
    120 s), overridable for tests/ops via `FACTORY_GH_TIMEOUT_SECS` /
    `FACTORY_GIT_TIMEOUT_SECS`;
  - `GIT_TERMINAL_PROMPT=0` on every git/gh invocation (credential helpers
    and askpass flows unaffected — only terminal prompting is disabled);
  - timeout semantics: read-only commands classify as `Network`/`TimedOut`;
    `git push` and `gh pr create` timeouts classify as `Uncertain`, so the
    ASF-18 delivery state machine marks the run `interrupted` with recovery
    required — a timeout can never trigger a second push or PR creation
    (`finish_failed_delivery` also treats `TimedOut` as uncertain);
  - JSON responses are never parsed from truncated output
    (`require_complete_stdout`).
- **Files:** `crates/factory-proc/*` (new), `crates/factory-git/src/lib.rs`,
  `crates/factory-git/src/error.rs`, `crates/factory-git/tests/bounded_execution.rs`
  (new), `crates/factory-github/src/gh.rs`, `crates/factory-github/src/delivery.rs`,
  `crates/factory-github/src/remote.rs`, `crates/factory-github/src/error.rs`,
  `crates/factory-core/src/factory.rs` (uncertain classification),
  `crates/factory-core/tests/github_delivery.rs` (hanging-create test).
- **Limitations:** tree termination is best-effort after a deadline
  (`taskkill /T` on Windows, process-group `SIGTERM` on Unix); a watchdog
  period after `SIGTERM` before escalation is intentionally not
  implemented — `child.kill()` follows immediately.

### ASF-N03 — Truly bounded Git diff capture — CONFIRMED AND FIXED

- **Suspected defect:** `diff_patch()` captured the entire diff into memory
  via `Command::output()` and only then truncated to the review budget.
- **Reproduction:** `crates/factory-git/tests/bounded_execution.rs` — a
  ~300 KB diff against a 10 KB budget returns ≤ budget + marker bytes with
  an explicit `[DIFF TRUNCATED: …]` trailer; small and at-budget diffs
  return complete with no marker; unicode and invalid-UTF-8 content stay
  lossy-safe; git errors surface as failures. (Memory: the capture limit is
  enforced by the factory-proc reader threads, proven by the flood tests in
  factory-proc — a 100 MB diff can retain at most budget + 4 KB.)
- **Fix:** `diff_patch` runs under the bounded supervisor with
  `stdout_limit = max_chars + 4096` slack and keeps the existing
  line-oriented trim; any dropped content (capture truncation or budget
  trim) appends the marker line so a partial patch can never be mistaken
  for the complete change. Truncated patches only feed review contexts,
  never patch application.
- **Files:** `crates/factory-git/src/lib.rs`,
  `crates/factory-git/tests/bounded_execution.rs`.

### ASF-N04 — Unicode-safe gitignore parsing — CONFIRMED AND FIXED

- **Suspected defect:** `IgnoreRules::from_gitignore` iterated
  `raw.as_bytes()` and sliced `&raw[i..i + 1]`, panicking on multi-byte
  characters.
- **Reproduction:** `données/`, `résumé.md`, `日本語/`, `naïve*.rs`,
  `文件/**`, `emoji-😀.txt` — every one of these panicked on the old
  parser (interior byte boundary slicing).
- **Fix:** the parser iterates `char_indices()`/characters; globs, `?`,
  `**`, escapes, anchoring, negation, and directory suffixes behave
  identically for ASCII, and multi-byte characters are escaped per
  character into the compiled regex.
- **Files:** `crates/factory-context/src/ignore.rs`.
- **Tests:** pattern battery above with match assertions, split-position
  parse test, escaped `\#`/`\!` literals, plus the differential suite
  (below) which includes unicode paths.

### ASF-N05 — Large PR body transport — CONFIRMED AND FIXED

- **Suspected defect:** the whole PR body traveled as one argv value
  (`--body <body>`), which Windows command-line limits can truncate.
- **Reproduction/design check:** the new test passes a 120 KB markdown body
  with quotes, backticks, blank lines, and unicode through the full
  delivery path into a fake `gh` that copies the `--body-file` argument to
  disk; the received bytes must equal the sent bytes exactly (after
  `begin_delivery`'s documented trim). Under the old argv transport the
  same body could not be passed on Windows at all.
- **Fix:** bodies are written to an exclusive-create `NamedTempFile`
  (0600 on Unix) consumed by the documented `gh pr create --body-file`
  flag and removed by RAII on success, failure, and timeout. Bodies over
  262,144 characters (GitHub's own cap) are rejected up front with
  `GitHubError::BodyTooLarge` — no silent truncation.
- **Files:** `crates/factory-github/src/gh.rs`,
  `crates/factory-github/src/error.rs`,
  `crates/factory-core/tests/github_delivery.rs`.

### ASF-N06 — Evaluation scalability — CONFIRMED AND FIXED (measured)

- **Suspected risk:** `History::load` selected the full `task_attempts` row
  including the evidence JSON, which carries the multi-kilobyte diff patch
  per attempt, on every evaluation request.
- **Benchmark (Windows 11, debug build, synthetic data with ~24 KB diff
  patches per attempt; `cargo test -p factory-eval --test scalability --
  --ignored --nocapture`). "Raw reads" is the old full-row read path,
  measured inside the same run; "evidence bytes" is what that path
  materializes in memory:**

  | attempts | raw reads (old path) | evidence bytes (old path) | evaluate() before | evaluate() after (lean) | speedup |
  | --- | --- | --- | --- | --- | --- |
  | 100 | 69 ms | 4.6 MB | 66 ms* | 16.6 ms | 4.0× |
  | 1,000 | 1.27 s | 46.3 MB | 912 ms* | 318 ms | 2.9× |
  | 10,000 | 8.35 s | 463.5 MB | 10.60 s* | 1.86 s | 5.7× |
  | 100,000 | 158.82 s | 4,634.9 MB | ≈159 s (reads-bound) | 62.22 s | ≈2.6× |

  (*) before-values at 100/1,000/10,000 from the pre-fix run of the same
  benchmark; at 100,000 the before evaluate() equals the old read cost
  because the read dominates. The decisive effect is memory: the old path
  materializes ~4.6 GB of evidence JSON strings at 100k attempts — beyond
  typical workstation RAM budgets for a dashboard request — while the lean
  projection transfers only identity/classification/timing fields plus the
  changed-file list. All numbers are debug-build; release builds are
  substantially faster on both sides.

- **Fix:** `FactoryDb::list_task_attempts_for_evaluation` — a lean SQL
  projection (identity, classification, timing, error, role/operation, and
  `json_extract(evidence, '$.changedFiles')` for language attribution).
  `History::load` uses it. No metric definitions changed.
- **Correctness:** a factory-db test proves the projection agrees with the
  full rows on every evaluator-consumed field (including unicode
  changed-file lists) and that diff patches are not transferred; the
  factory-eval suite (22 tests over all metrics, windows, facets, routing)
  passes unchanged.
- **Limitations:** evaluation remains a full (lean) history scan per
  request — linear in attempts, now independent of evidence size. No
  materialization cache was added; the measurement above shows it is not
  yet needed.

### ASF-N07 — Git ignore-rule consistency — CONFIRMED AND FIXED

- **Suspected risk:** only the root `.gitignore` was honored; nested
  `.gitignore` files and `.git/info/exclude` were invisible to the Context
  Engine, and when ripgrep was installed, `rg` applied its own (machine
  dependent) ignore rules including the user's global excludes.
- **Intended semantics (decided and documented in
  `docs/architecture.md`):** repository-local rules only — root +
  nested `.gitignore` (deeper overrides shallower, gitignore(5) precedence;
  no re-inclusion under an excluded directory) + `.git/info/exclude`
  (worktree `.git` pointers followed), layered under Factory's built-in
  exclusions, which nothing can negate. Global user excludes are
  deliberately not consulted so indexing is deterministic across machines;
  repositories must express their exclusions in the repository.
- **Fix:** `IgnoreRules::collect` performs a bounded, read-only,
  non-symlink-following walk that gathers every applicable ignore file with
  explicit precedence; `is_ignored` now also applies the
  parent-directory-exclusion rule; ripgrep runs with `--no-ignore` so
  Factory's compiled rules are the single policy for indexing, symbol
  search, content search, and excerpts alike.
- **Evidence:** `crates/factory-context/tests/gitignore_differential.rs`
  compares Factory's decision with real `git check-ignore` (isolated
  `core.excludesFile`) on 20 fixture paths across root/nested/deep
  `.gitignore` files, `info/exclude`, negations, anchored and unicode
  rules — zero mismatches; plus a security test proving an ignored file
  carrying fake credentials never reaches `search_content` or the index,
  and a refresh test proving ignore-rule changes both directions (newly
  ignored files leave the index, newly unignored files return) while
  ASF-R01 content hashes keep working.
- **Known intentional divergence:** Factory's built-in exclusions
  (`.factory`, `target`, `node_modules`, `*.lock`, …) are applied even when
  git itself would not ignore those names — a superset, documented, and
  excluded from the differential comparison.

### ASF-N08 — Git maintenance exit-status handling — CONFIRMED AND FIXED

- **Suspected defect:** `Repo::prune()` ignored `status()`, treating a
  nonzero `git worktree prune` as success.
- **Reproduction:** fake git exiting 1 on prune (deterministic, via
  `FACTORY_GIT_BIN`); the old code returned `Ok(())`.
- **Fix:** `prune` requires successful termination and reports the
  operation, exit status, and a bounded stderr excerpt. The same audit of
  adjacent maintenance methods (`add_worktree`, `remove_internal`,
  `commit_changes`, `is_ancestor`, `rebase_onto_in`) found they already
  checked status; all were migrated to the bounded supervisor with
  structured `TimedOut`/`OutputLimitExceeded` errors.
- **Files:** `crates/factory-git/src/lib.rs`, `crates/factory-git/src/error.rs`.

## C. Security review

- **Streaming secrets:** known secrets cannot be reconstructed from the
  concatenation of any persisted or streamed output chunks — verified at
  the callback boundary, in SQLite (mid-run via a second connection), and
  in final rows; cancellation and failure paths flush only secret-free
  content. Covered secrets: policy-denied environment values,
  agent-configured environment values, MCP grant tokens. Not covered:
  unknown secrets, encoded/transformed values, >1 MiB secrets split across
  chunks (documented).
- **External process bounds:** every factory-owned git/gh command has a
  deadline, bounded capture, null stdin, `GIT_TERMINAL_PROMPT=0`, and
  tree termination. A blocked subprocess cannot occupy a worker past its
  deadline; outputs cannot grow unbounded; truncated JSON is rejected, not
  parsed.
- **Timeout uncertainty:** push/create timeouts are `Uncertain` → delivery
  `interrupted` → reconciliation required before any retry; no force-push
  paths exist; published results cannot be overwritten by stale attempts
  (ASF-18 operation-id guards untouched and re-tested).
- **Path/ignore boundaries:** the Context Engine is read-only,
  non-symlink-following, bounded, and now honors repository-local ignore
  rules consistently across index/search/excerpts (git-check-ignore
  differential evidence); ignored files with fake credentials provably
  never reach agent context.
- **PR bodies:** byte-exact transport via exclusive-create temp file
  (removed on all paths), 262,144-character cap, no argv exposure, no
  shell anywhere in the path.

## D. Performance results

- ASF-N06 before/after table above (same machine, same synthetic dataset
  generator, `#[ignore]`d benchmark kept out of CI).
- ASF-N01: redaction adds one bounded scan per ~8 KB chunk over the secret
  list and holds ≤ max-secret-length bytes per stream (flood test asserts
  the bound); no benchmark was run because the cost is dominated by the
  existing decode/append path.
- ASF-N02/N03: capture memory is hard-bounded per command; diff capture is
  bounded to budget + 4 KB regardless of diff size (flood and 300 KB-diff
  tests).
- ASF-N07: `IgnoreRules::collect` adds one bounded directory enumeration
  (no content reads) at engine construction; the differential suite runs
  in ~1.3 s including git subprocesses.

## E. Compatibility

- Windows validated locally (process trees via `taskkill`, `.cmd` fakes,
  unicode paths, UTF-8 fixtures). Linux/macOS code paths compile and run in
  CI (unix process groups, `sh` fakes); CI results pending push.
- Existing SQLite databases: no migrations; new read paths only.
- Existing context manifests: unchanged format; ASF-R01 content hashes
  keep working (refresh test proves both directions of ignore-rule
  changes).
- API contracts: unchanged; delivery error strings for timeouts are new
  values of existing states, surfaced through the existing interrupted /
  recovery-required fields the dashboard already renders.
- GitHub delivery behavior: ASF-18 atomic claims, operation ids, durable
  states, existing-PR linking, reconciliation, and retry restrictions are
  preserved and re-tested (22 delivery tests, including two new ones).

## F. Validation results

Commands (all with `-j 1 -- --test-threads=1` per repo convention; exit
codes captured directly, logs not piped through tail):

| Command | Result |
| --- | --- |
| `cargo test -p factory-policy` | PASS (62) |
| `cargo test -p factory-proc` | PASS (10) |
| `cargo test -p factory-git` | PASS (13 unit + 8 bounded-execution) |
| `cargo test -p factory-context` | PASS (53 unit + 3 differential) |
| `cargo test -p factory-db` | PASS (36) |
| `cargo test -p factory-eval` | PASS (22; scalability bench `#[ignore]`d, run explicitly: PASS) |
| `cargo test -p factory-core --test streaming_redaction` | PASS (2) |
| `cargo test -p factory-core --test policy_enforcement` | PASS (8) |
| `cargo test -p factory-core --test github_delivery` | PASS (22) |
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets -j 1 -- -D warnings` | PASS (after 4 mechanical lint fixes in new code) |
| `cargo test --workspace -j 1 -- --test-threads=1` | PASS (585 tests, 57 suites, 0 failures) |
| dashboard `format:check` / `lint` / `typecheck` / `test` / `build` | PASS (no TS/TSX changed; 303 tests, build ok) |
| GitHub Actions (linux/windows/macOS) | NOT RUN (push pending) |

The exact final outcomes of the last three rows are recorded in the
repository history of this file if amended, and in the final chat summary.

## G. Remaining risks

| Risk | Severity | Evidence | Reproduced | Suggested correction | Why unresolved |
| --- | --- | --- | --- | --- | --- |
| Redaction cannot catch secrets Factory does not know (incl. encoded forms) | informational | design | n/a | none — documented boundary | inherent to the allow-list model |
| >1 MiB configured secrets not streaming-matched across chunks | low | unit test documents | yes (by construction) | none realistically required | 1 MiB cap bounds memory; real credentials are far smaller |
| Evaluation is a full lean scan per request | low | benchmark after-fix table | measured | materialize/cache if histories grow another order of magnitude | current numbers make a cache premature |
| Best-effort tree termination (no SIGKILL escalation ladder) | low | factory-proc kill path | n/a | add escalating kill if descendants survive | `child.kill()` follows immediately; no surviving-descendant case observed |
| macOS/Linux validation of new process-supervisor paths | pending CI | — | not locally possible | run CI matrix | push not yet performed at authoring time |

No newly discovered defects remain open from this campaign.
