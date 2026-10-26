# ASF Final Context Search Verification — V01 through V03

Date: 2026-10-26

## Environment and scope

- Base commit: `437050822ddcc7cc0683797ecf2e174fc3b1e1d8`, matching the
  requested reviewed revision. Initial `git status --short` was empty;
  `git rev-parse HEAD` and `git log -10 --oneline` were inspected first.
- Implementation revision: `96cd3603c782a92ba5ef1cbe589637c772ccc1fb`.
  Local validation completed before committing. The user subsequently
  authorized committing and pushing the corrections to GitHub; the
  implementation was pushed to `main`. Tags,
  releases, deployments and changes to production repositories remain outside
  the authorization; none were performed.
- Native validation: Windows 11 Home, version 10.0.26200, x64;
  `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, host
  `x86_64-pc-windows-msvc`; `cargo 1.99.1 (5f94df478 2026-08-27)`;
  Node v24.22.1, npm 12.0.2. CI still specifies Node 22.
- Read the contributing rules, architecture, security documentation, N01–N08
  and F01–F04 reports, context implementation and tests, subprocess supervisor,
  and CI workflow. No unrelated crate implementation was modified.
- New direct Unix-only dependency: `libc = "0.2"` for OS-provided
  `O_NOFOLLOW`/`O_NONBLOCK`; version 0.2.189 was already locked transitively.
  No new package version, public API signature, HTTP contract, schema,
  migration, workflow transition, dashboard source, or delivery change.

Changed files: `Cargo.lock`, `crates/factory-context/Cargo.toml`,
`crates/factory-context/src/{files,index,lib,rank,resolve,search}.rs`,
`crates/factory-context/tests/search_boundaries.rs`, `docs/architecture.md`,
`docs/security.md`, and this report. `ignore.rs`, the differential test suite,
Factory Core, `factory-proc`, and the CI workflow are unchanged.

## ASF-V01 — CONFIRMED AND FIXED

### Reproduction and cause

Before the fix, fake ripgrep returned the canonical repository path followed
by `linked/secret.txt`. The intermediate directory link reached an isolated
private directory; its final file was ordinary. The production search returned
`SearchHit { path: "linked/secret.txt", line: Some(1), snippet:
Some("V01_SYNTHETIC_LINK_VALUE"), ... }`. The regression failed as intended.
The old validator checked lexical containment and only the leaf's link type.

The separate parent-traversal regression did not expose content locally on
Windows. The fixture now builds raw candidate strings rather than using
`PathBuf::join` for traversal: joining onto a Windows verbatim root can
normalize away the very components being tested. Explicit `..`, repeated
parents, and `.` are rejected before any candidate metadata/open. Linux/macOS
execution of that regression is pending; their pre-fix exploit is not claimed
as a locally observed result.

### Correction

`src/files.rs` centralizes repository content eligibility. Starting with the
canonical root, it requires an absolute contained path and normal, UTF-8
relative components; rejects explicit dot/parent components and Windows
alternate-stream syntax; inspects every component with `symlink_metadata`;
rejects links, Windows reparse attributes, non-directory parents and non-file
leaves; verifies canonical containment; and checks both supplied and canonical
names against Factory's existing ignore rules. Canonical names also deduplicate
Windows case aliases without implementing case folding.

The common open path validates immediately before and after opening, checks
the opened handle's type and size, uses Unix `O_NOFOLLOW | O_NONBLOCK` and
device/inode identity checks, and uses Windows `FILE_FLAG_OPEN_REPARSE_POINT`
with handle metadata rejection. Ripgrep hits are read through this path before
being emitted; an unreadable or newly ineligible path produces no content hit.
Index reads, link expansion and resolution excerpts share the same path.
Engine search/resolution also reject cached paths replaced by directory links.

### Evidence and limits

- `v01_parent_traversal_cannot_expose_private_content` and
  `v01_intermediate_links_cannot_expose_content`: production fake-rg path,
  engine search, selected context and rendered context contain no private
  path/content. The ordinary repository fixture remains searchable.
- The combined adversarial test covers external absolute paths, textual root
  prefixes, relative tool paths, dot/parent components, directories returned
  as files, ignored root/nested files, oversized files, internal/external
  links and a directory cycle, through fake rg and forced Rust fallback.
- Windows junction creation is mandatory in its dedicated test and its reparse
  attribute is asserted. Case aliases cannot bypass `*.env` or duplicate hits.
- A deterministic cached-directory replacement test rejects the link without
  requiring a concurrent race. Ordinary Unicode and space-containing names
  still work. Unix newline/backslash and non-UTF-8-name tests are platform-gated.
- **Remaining limitation:** parent components are checked by path, not walked
  through pinned directory handles. This is not complete protection against an
  adversary replacing directories during validation/opening. No such guarantee
  or OS sandbox is claimed. The standalone `search_index` remains a search of
  an index snapshot; the engine performs live eligibility revalidation.

Modified implementation: `files.rs`, `search.rs`, `index.rs`, `rank.rs`,
`resolve.rs`, and private module/dependency wiring. Regression tests are in
`files.rs`, `search.rs` and `tests/search_boundaries.rs`.

## ASF-V02 — CONFIRMED AND FIXED

### Reproduction and cause

First extracted the existing bounded reader without changing its behavior.
`v02_a_read_that_grows_past_reported_size_is_rejected` supplied a reader with
reported size 1 MiB and one extra byte, and failed before the correction:
the returned buffer was `Some` despite overflow. Fallback matching could use
that buffer while snippet extraction separately rejected it.

### Correction and evidence

`files::read_bounded` is the single repository-content size invariant. Metadata
is an early gate, not the sole proof: `take(MAX_INDEX_FILE_READ_BYTES + 1)`
bounds the actual read and overflow is rejected inside the helper. Callers
receive either eligible bytes of length at most 1 MiB or `None`; they cannot
forget a separate overflow check. Read errors discard partial content.

The same reader now feeds fallback matching, accepted rg candidates, snippets,
index fingerprints, link expansion, and resolution excerpts. The latter three
previously used whole-file `std::fs::read` before checking size; those live
repository-content reads are now bounded too. Index stability checks use the
opened handle and verify the actual byte count; ASF-R01 fingerprints remain
content-derived.

Deterministic tests cover empty/below/exact/above-limit content, stale metadata,
an extra byte carrying the only possible match, an endless reader (exactly
limit + 1 consumed), a read failure after a valid prefix, UTF-8 near the cap,
and a matching final valid line. A controlled real-file reader changes file
length after its first read: growth to cap + 1 is rejected, growth to cap is
accepted, and shrinkage returns the actual valid shorter content. There are
no sleeps or race-dependent assertions and no gigabyte buffers.

**Remaining limitation:** bounded reads do not promise an atomic snapshot of a
file concurrently edited in place. Returned repository content is bounded;
oversized files can still have metadata-only index/path results, as before,
but cannot produce a content hit or excerpt. Persisted manifest and ignore-rule
parsing are separate existing metadata/configuration paths, not claims of this
repository-content reading abstraction.

## ASF-V03 — CONFIRMED AND FIXED

### Reproduction and cause

The production fake-rg test creates `src/file001.rs` through `file100.rs`.
Before the fix, reversing its candidates selected `file091.rs`–`file100.rs`
instead of `file001.rs`–`file010.rs`. Sorting after `take(limit)` only sorted
an encounter-dependent subset. Both discovery paths also stopped at their
first 1,000 matches.

### Selection policy and algorithm

Both paths retain the lexicographically smallest 1,000 distinct canonical,
repository-relative UTF-8 keys in a `BTreeSet`. At capacity, a new better key
replaces the greatest key before insertion; stored candidates never exceed
1,000. Discovery continues over the available bounded scope. Ordered candidates
are revalidated/read before up to `min(limit, 1,000)` content hits are returned.
The comparison is case-sensitive Rust string order, using `/` separators,
without manual case folding or Unicode normalization.

Fallback directory enumeration is deterministic under a stable filesystem and
fixed ignore rules. A complete directory is collected within the remaining
50,000-entry budget, sorted by encoded filename, and its ordinary files are
examined before descending into sorted child directories. Entries include
ignored and special names. If collection consumes the last entry allowance,
the entire incomplete directory is discarded and scanning stops. No extra
entry is probed: even an exactly-at-budget directory is conservatively
incomplete. Previously completed directories form the deterministic partial
scope. No arbitrary filesystem enumeration prefix is searched.

For M discovered candidates, K = 1,000 and directory sizes d, ordered retention
costs O(M log K), directory sorting O(sum(d log d)); content I/O is separately
bounded per file. Enumeration/frontier storage is O(50,000 entries), candidate
storage O(K paths), and content buffering O(1 MiB), with path/string overhead.
These are algorithmic bounds, not measured RSS or benchmark claims.

### Evidence and limits

- Forward, reverse and an explicit deterministic permutation of 100 real
  candidates select the same first ten paths through production fake rg.
- Unit retention tests check the bound after every insertion, including
  duplicates, forward/reverse/permuted inputs and more than 1,000 keys.
- A real 1,051-file fixture with duplicate reversed rg output and an unlimited
  requested result limit returns exactly the same first 1,000 paths as Rust.
- Unicode, mixed case, spaces, nested paths, ignored files, link candidates
  and duplicate representations yield the same selected subset on repeated
  fake-rg and fallback searches.
- A synthetic 50,001-entry iterator exercises the exact production directory
  collector in both orders, proves exactly 50,000 entries are consumed and
  that the incomplete directory is discarded. Smaller real filesystem
  fixtures with different creation orders verify the same partial scope.
- **Remaining limitations:** scope is partial when the entry budget is
  exhausted and is not a global exhaustive top-k over unvisited files. The
  existing search response has no partial-result flag. Rg's complete bounded
  capture can cover a larger scope than fallback; parity is asserted where
  both fully evaluate the same literal-query fixture. Existing rg regex vs.
  fallback literal matching semantics are unchanged. Non-UTF-8 relative
  filenames are skipped rather than lossily mapped into the string response.

## Independent second-pass review

After the initial fix, reviewed component validation, leaf and parent swaps,
handle checks, actual read lengths, duplicate identities, early stops, bounded
large-directory collection, status/truncation handling and test environment
isolation. The second pass pinned canonical Windows alias handling, raw Windows
adversarial fixtures, incomplete NUL records, and cached-directory replacement.
Index key canonicalization now happens after non-following eligibility checks;
resolution link expansion shares the engine's ignore snapshot. No blocking
V01–V03 defect is known after that review and targeted validation.

Rg keeps its existing supervisor, 30 s default timeout, 1 MiB stdout / 64 KiB
stderr bounds, NUL separation and status rules: 1 is authoritative no-match;
failure/timeout/truncation falls back. Unterminated nonempty output also falls
back. No `factory-proc`, git/gh supervision, secret-redaction or delivery code
was changed. Fake-rg environment changes are serialized under the existing
process lock and now restore any previous override on drop.

The first pushed CI run
[40380682163](https://github.com/OmegaMc1331/agentic-software-factory/actions/runs/40380682163)
passed Linux Rust and dashboard checks but exposed a macOS fixture failure:
creating `invalid-\xff.rs` returned `EILSEQ` before search ran. The follow-up
fixture accepts only that explicit filesystem rejection; all other creation
errors still fail. The raw non-UTF-8 candidate is always injected into fake rg,
newline/backslash assertions always run, and a UTF-8 replacement-name alias
ensures a lossy path conversion cannot open a different file. Linux continues
testing a real non-UTF-8 directory entry. No platform test or assertion was
disabled and production code is unchanged by this follow-up. Final CI is
pending the follow-up push.

## Validation

Local logs are retained under ignored `.factory/verification-v01-v03/`.

| Exact command | Result on native Windows |
| --- | --- |
| `cargo test -p factory-context -j 1 -- --test-threads=1` | PASS: 66 unit + 7 differential + 22 boundary harness cases; 3 boundary fixture skips for Windows privileges, 1 pre-existing ignored timing diagnostic |
| `cargo test -p factory-context --test gitignore_differential -j 1 -- --test-threads=1` | PASS: 7 tests, including the unchanged git-check-ignore matrices |
| `cargo test -p factory-context --test search_boundaries -j 1 -- --test-threads=1` | PASS: 22 harness cases; 19 exercised, 3 inherited leaf-link fixture skips |
| `cargo test -p factory-context --test search_boundaries -j 1 -- --test-threads=1 --nocapture` | PASS; verified the three privilege skips explicitly |
| `cargo test -p factory-core --test role_aware -j 1 -- --test-threads=1` | PASS: 18 fake-agent integration tests, including mission context isolation |
| `cargo test -p factory-core --lib -j 1 mission::tests -- --test-threads=1` | PASS: 9 tests |
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets -j 1 -- -D warnings` | PASS |
| `cargo test --workspace -j 1 -- --test-threads=1` | PASS; existing ignored diagnostics/platform skips retained |
| `cargo test --workspace -j 1` | PASS with normal CI test threading; existing ignored diagnostics/platform skips retained |
| `npm run format:check` (in `apps/dashboard`) | PASS |
| `npm run lint` | PASS |
| `npm run typecheck` | PASS |
| `npm test` | PASS: 303 tests, 27 files |
| `npm run build` | PASS; bundle-size advisory is non-fatal |
| `node --test scripts/release-checksums.test.mjs` | PASS: 41; 2 skipped because native SHA checking/shasum utilities are unavailable |

No real external agent or paid model provider was invoked. The visible-output
boundary run confirmed three existing fixtures return early because this Windows
account cannot create file symlinks: `a_symlink_to_an_external_file_exposes_nothing`,
`e_internal_symlinks_are_not_followed_either`, and
`f_broken_symlinks_are_ignored_without_error`. Those leaf-link scenarios were
not exercised natively and await final-patch Unix/privileged CI; they are not
presented as local security evidence. The new directory-link and junction
regressions assert creation and executed successfully without elevation.
The timing diagnostic was not run because no benchmark is required here.

Both complete workspace runs exited 0. The unchanged prior regression suites
remain green locally, including ASF-R01 content fingerprints, ASF-N01 streaming
redaction, ASF-N02/F03 bounded subprocess supervision and delivery uncertainty,
ASF-N07/F02 gitignore precedence, and ASF-F01/F04 context boundaries/resource
limits. The Windows leaf-link skips above and Unix-only cases are not counted
as exercised platform evidence. No known blocking V01–V03 defect remains.

## CI and release readiness

Inspected `.github/workflows/ci.yml`; it is unchanged and retains Rust and
checksum tests on Linux/Windows/macOS, dashboard checks, installer end-to-end
tests and the embedded-dashboard smoke job.

Read-only GitHub inspection verified base-commit CI run
[40353722668](https://github.com/OmegaMc1331/agentic-software-factory/actions/runs/40353722668):
all nine jobs completed successfully, including all three Rust platforms and
all preserved integration jobs. That run validates the base, **not this local
patch**. The PR-only workflow-run tool returned no runs; the general read-only
Actions endpoint supplied the actual push run and its jobs.

| Platform/job | Base run | V01–V03 implementation |
| --- | --- | --- |
| Windows Rust CI | PASS | Pending; native local gates listed above |
| Linux Rust CI | PASS | Pending; no local Linux target/runtime execution |
| macOS Rust CI | PASS | Pending; no local macOS execution |
| Dashboard CI | PASS | Pending; local checks pass |
| Installer end-to-end / embedded dashboard | PASS | Not executed locally; pending final-patch CI |
| Release checksums, three platforms | PASS | Native Windows helper tests pass with the two documented utility skips; final-patch CI pending |

Release readiness: **VALIDATION INCOMPLETE**. The confirmed V01–V03 corrections
are implemented; targeted tests, both full Rust test modes, formatting, Clippy,
and every requested dashboard check pass natively. Final-implementation CI is
pending the fixture-correction push. Historical CI results are not substituted for
validation of this implementation. No release is published.
