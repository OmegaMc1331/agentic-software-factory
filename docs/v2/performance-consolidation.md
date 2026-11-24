# Performance consolidation

P13 separates historical debt, controlled comparisons, derived query/process analysis and unexecuted qualification. It does not infer a production guarantee from one developer host.

## Method

`scripts/v2_p13/performance.mjs` runs real native Git and authenticated production memory/Git APIs against disposable repositories. It requires the copied executable's matching SHA-256 build record and a fresh output directory. Inputs are explicitly synthetic; no model calls or remote writes occur. Every memory uses ordinary schema/source/secret validation, one create and two edits. The harness verifies exact staged commit contents and preserved unstaged changes independently with Git.

Before/after comparisons use the same Windows host, unoptimized debug profile with symbols disabled, fixture data, public API path and sampling method. The P12 baseline executable hash is `bfb64a1385512d840417c1144a55dd389a7a58e4e4a5517cfd4a126874006beb`. Its recorded build source is not replaced with the driver script's current commit. Initial script preflight failures remain retained; they are not benchmark samples.

Host: Windows 11 Home build 26200, Intel i5-12500H, 16 logical processors, approximately 16.8 GB visible RAM. The fixture resides on C:. Physical-storage identity and before/after available RAM/CPU samples are retained in raw results. This is a shared desktop, not an isolated lab. Baselines were collected without another ASF build/test, but observed host CPU load was nonzero (17–21% at the initial samples). Tail values are sample observations, not service-level guarantees.

## Controlled baseline

| Operation | Fixture / samples | Before |
| --- | --- | --- |
| Memory create | First 100 records, n=100 | 18.195 ms median; 20.434 ms p95 |
| Memory update | First 100 records, n=200 | 18.130 ms median; 20.999 ms p95 |
| Memory create | Next 900, collection reaches 1,000 | 27.911 ms median; 38.362 ms p95 |
| Memory update | Same next 900, n=1,800 | 28.511 ms median; 38.267 ms p95 |
| Search | 1,000 memories, n=10 | 25.871 ms median |
| Context retrieval | 1,000 memories, n=10 | 40.796 ms median |
| Index rebuild | 1,000 memories, n=3 | 27.379 ms median |
| Git clean status | 100 / 1,000 / 6,000 files, n=5 each | 410.54 / 414.62 / 555.03 ms median |
| Git changed status | Same, up to 200 changed, n=5 each | 426.30 / 455.80 / 478.21 ms median |
| Git diff | Same, n=5 each | 698.07 / 1,143.32 / 772.26 ms median |
| File stage | 6,000 files, n=1 | 7,128.72 ms |
| Local commit | 6,000 files, n=1 | 7,140.61 ms |

These are newly controlled P13 baselines. P11's historical 9,000-record/53.44-minute load and P12's debug/concurrent-host-load figures are separate workloads and cannot be used as an implementation speedup denominator.

## Narrow changes and correctness

Read-only `EXPLAIN QUERY PLAN` on the populated baseline identified an unindexed FTS memory-ID delete, a combined duplicate/scoped-subject OR query and materialized search membership/order work. Migration 26 rebuilds only the derived FTS table, maps its rowid to the authoritative record rowid and adds a project/newest-row index. New writes replace that exact FTS row inside the existing revision transaction. Search uses a validated rowid/ID/revision join; duplicate detection unions the existing indexed fingerprint and scoped-subject probes. Source checks, secret screening, revision history, deletion ledger, retention budgets and current invocation validation remain intact. There is no bulk-import bypass.

The populated-index migration fixture exposed SQLite FTS5's write requirement during backup integrity validation. The original source connection remains read-only; the newly created owned backup is opened read/write for its actual `quick_check`. A failed check still refuses upgrade. The fixture verifies populated legacy index remapping, unchanged authoritative revisions, ordering/pagination, edits, disabled eligibility, deletion and rebuild consistency.

Git's source-filter guard previously checked batches of 100 paths, each through the controlled Git read path. Code inspection gives approximately 120 helper calls for 6,000 paths across attribute/config checks; that count is **derived from implementation**, not a captured process trace. Adaptive batches now accept up to 1,000 paths while conservatively bounding quoted UTF-16 argument length. Every path still receives an actual complete NUL-delimited `filter` result; malformed, incomplete or mismatched output fails closed. Active unapproved filters, sensitive paths, stale generations, locks, coordinator refs and worktree permissions remain checked. A native 1,100-path regression puts a prohibited filter in the last batch and verifies no index/HEAD mutation on refusal.

## Consolidation results

The completed controlled after run used copied P13 build2 (`5ac35484a456d85fef950348316f73318ad2af6cbcea9419f2246e5877b81e23`). Both runs exited their owned server and independently verified exact commit contents. C: maps to the Samsung NVMe SSD. No competing ASF build/test ran during either controlled API comparison; other desktop load remains a limitation.

| Same operation / sampling | Before | After | Observed comparison |
| --- | --- | --- | --- |
| Create, records 101–1,000, n=900 | 27.911 ms | 20.503 ms | 26.5% lower median |
| Update, same stage, n=1,800 | 28.511 ms | 20.527 ms | 28.0% lower median |
| Same next 900 create+two-edit load | 76,067.77 ms | 63,745.91 ms | 16.2% lower elapsed time |
| 1,000-record search, n=10 | 25.871 ms | 15.733 ms | 39.2% lower median |
| 1,000-record retrieval, n=10 | 40.796 ms | 46.741 ms | 14.6% higher median; not an improvement |
| 1,000-record rebuild, n=3 | 27.379 ms | 29.773 ms | 8.7% higher median |
| 6,000-file clean status, n=5 | 555.03 ms | 443.92 ms | 20.0% lower median |
| 6,000-file changed status, n=5 | 478.21 ms | 479.15 ms | Essentially unchanged in these samples |
| 6,000-file diff, n=5 | 772.26 ms | 705.00 ms | 8.7% lower median |
| 6,000-file stage, n=1 | 7,128.72 ms | 2,800.84 ms | Single sample, 60.7% lower elapsed time |
| 6,000-file commit, n=1 | 7,140.61 ms | 2,998.02 ms | Single sample, 58.0% lower elapsed time |

The first 100-record create/update medians worsened (18.20/18.13 ms to 32.60/32.30 ms). The following 900-record samples were faster. Initialization/host variability and unchanged retrieval/source checks remain visible; no universal memory speedup or production latency guarantee is claimed. Additional stress/resource/browser observations and their concurrency conditions are reported separately in the delivery. The historical P11 workload still requires final integrated release-build validation.

The existing Vitest configuration uses jsdom and two workers. `runner-memory.ps1` measures whole owned process-tree working sets/private commit, preserving the complete test run and exit code. Shared RSS may double-count shared pages and one-second samples can miss short peaks. Unrestricted-worker qualification is distinct from a two-worker pass; insufficient headroom is reported as not run, never fixed by disabling tests. No validation is traded for speed.

## Frontend and resource observations

The final complete frontend build reports the main chunk at 914.51 kB raw / 262.40 kB gzip, compared with the retained P12 906.30 / 260.24 kB output. This is growth of 8.21 / 2.16 kB, not an optimization claim. Settings (11.39 / 3.93 kB), Commands (2.20 / 1.15 kB) and Notifications (5.08 / 2.14 kB) remain separate lazy modules. The greater-than-500-kB warning is retained. P14 must establish an integrated bundle budget rather than suppressing it.

Browser timings measure navigation to an actually useful initial view, populated reload through durable presentation save, project switching and palette dispatch. Palette dispatch is not complete subsystem load time. Six repeated warm navigation samples and six actual Memory presentation open/close/unmount samples record DOM counts and JavaScript heap without forcing GC. These are bounded observations; they do not establish a long-duration leak or OS-memory guarantee. ResourceCache and SessionStreams tests additionally verify shared request/stream ownership, abort/timer cleanup and protection against stale responses.

The two-worker measured whole suite passed with sampled peak summed working set 548,270,080 bytes and private commit 552,194,048 bytes. The four-worker suite passed at 855,195,648 / 834,211,840 bytes. Initial monitor failures are retained. A 16-worker run was refused before launch at approximately 1.11 GB available RAM, below the documented 4 GiB guard. Existing concurrency is unchanged. This investigates the historical concern without falsely qualifying unrestricted execution.

## Final-artifact follow-up

Build 10, SHA-256 `2223781778a155d7c2f45e4ad5f18664d76e4fac0d65461464ee1051dd512c43`, repeats the same non-stress API workload on November 16 after all other ASF qualification jobs close. It uses the same hardware/profile and payloads as the November 15 comparison. This is a separate shared-desktop observation, not a same-boot or isolated-host guarantee. Available RAM changed from approximately 1.63 to 2.44 GB; sampled host CPU changed from 0 to 12 percent. The quantile convention uses the sorted upper-middle sample for even-count medians and nearest-rank p95.

| Same operation | P12 baseline | P13 first after | Final follow-up |
| --- | --- | --- | --- |
| First 100 creates, median | 18.195 ms | 32.596 ms | 23.528 ms |
| First 200 updates, median | 18.130 ms | 32.304 ms | 22.992 ms |
| Next 900 creates, median | 27.911 ms | 20.503 ms | 23.739 ms |
| Same 1,800 updates, median | 28.511 ms | 20.527 ms | 23.720 ms |
| Next 900 create/two-edit elapsed | 76.068 s | 63.746 s | 61.691 s |
| 1,000-record search, median, n=10 | 25.871 ms | 15.733 ms | 21.991 ms |
| 1,000-record retrieval, median, n=10 | 40.796 ms | 46.741 ms | 79.945 ms |
| 1,000-record rebuild, median, n=3 | 27.379 ms | 29.773 ms | 43.012 ms |
| 6,000-file clean status, median, n=5 | 555.03 ms | 443.92 ms | 574.75 ms |
| 6,000-file diff, median, n=5 | 772.26 ms | 705.00 ms | 947.39 ms |
| 6,000-file stage, n=1 | 7.129 s | 2.801 s | 3.455 s |
| 6,000-file commit, n=1 | 7.141 s | 2.998 s | 3.875 s |

Write/stage/commit gains remain visible, but the later sample does not reproduce every earlier latency gain. Retrieval/rebuild and some Git reads remain explicit performance debt. Their cause and release-build acceptability are not established by these desktop samples; correctness checks were retained. This is a medium performance qualification concern, not a demonstrated data-integrity failure. Do not report a universal speedup or a resolved retrieval bottleneck.

One owned server sample after operations was 33,718,272 bytes working set and 12,087,296 bytes private commit; it excludes provider processes and is not a peak. The 1,000-record, three-revision database occupied 18,186,240 bytes with a 4,235,392-byte observed WAL. The separate 10,000-record stress accounting remains in the delivery.

The final browser recorded 231.2 ms from empty navigation to a useful workspace and 535.8 ms for its first project presentation switch through durable save. These single samples are method-labelled, not performance guarantees. Six unforced open/close samples had 883–888 DOM nodes and approximately 53.9–69.0 MB JavaScript heap. A separate diagnostic uses explicit CDP garbage collection: six real Memory open/close/unmount cycles moved used heap from 7,859,116 bytes before lazy loading to 8,541,992 bytes after, with 1,060–1,064 DOM nodes. Persisted presentation metadata and lazy modules remain; six cycles do not prove the absence of a long-duration leak. Git HEAD/index/working content stayed unchanged during that inspection.

The monitored test-runner samples contain all 561 tests present at their source snapshot. The final expanded 565-test suite passed separately with the unchanged two-worker configuration; its exact source receipt is retained. The earlier memory peaks are not relabelled measurements of that later suite.
