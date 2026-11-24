# Candidate performance qualification budgets

These assessment thresholds are recorded before the P14 release-binary workload.
They are engineering acceptance criteria for the disposable fixtures, not service
guarantees for every repository, provider or machine. A failed threshold remains
visible; changing a threshold after measurement requires an explicit explanation.

The reference host is the actual Windows i5-12500H desktop with approximately
16.8 GB visible RAM and NVMe fixture storage. Release builds use optimization and
LTO. Record OS/browser/tool versions, storage, free RAM/commit, host CPU load,
fixture content, sample count and concurrent processes. Do not run the benchmark
alongside ASF compilation, regression suites or model execution.

## Interactive operations

| Operation | Fixture / sampling | Target | Qualification ceiling |
| --- | --- | --- | --- |
| Memory create/update | Ordinary P11-length content; 100, 1,000 and 10,000 records; three revisions | Median <=100 ms at 10,000 | P95 <=250 ms |
| Memory search/retrieval | Bounded ordinary API results; >=10 samples at each size | Median <=500 ms | P95 <=1 s |
| Memory index rebuild | 10,000 authoritative records; >=3 samples | Median <=2 s | Report every sample and verify records/revisions after rebuild |
| Git status | 5,000/6,000 files, clean and modified; >=5 samples | Median <=1 s | P95 <=1.5 s |
| Bounded text diff | Same repository; >=5 samples | Median <=1.5 s | P95 <=2 s |
| Large staging/commit | 6,000 files, >=3 independent fixtures | Each <=10 s | Preserve exact contents, unstaged changes and safe refusal paths |
| Useful workspace | Actual packaged application and browser | Cold <=2 s; warm <=1 s | Report individual observations and host load |
| Project presentation switch | Durable state acknowledged | <=1.5 s | Correct project and preserved buffers are mandatory |
| Warm command dispatch | Actual keyboard/palette action | <=100 ms | First module loading reported separately |
| Warm Settings/Memory/Source Control | Actual populated views | <=500 ms | Record partial/unavailable data honestly |

The memory write thresholds allow headroom above P13's approximately 20-30 ms
medians at 1,000 records while requiring practical responsiveness at the supported
large fixture. The Git read ceilings bound the inherited approximately 0.4-1.1 s
observations; they do not replace correct state with a stale cache. Large mutations
are allowed seconds because they must validate and write thousands of real files.
The UI must remain responsive and show an actual pending operation throughout.

## Large workload and resources

Exercise the full original P11 100/1,000/10,000-record ordinary workload, including
its four concurrent writers and three revisions. A new 9,000-record stage exceeding
15 minutes remains a significant supported-scale concern. This ceiling is about
33 ordinary create/edit operations per second over 27,000 writes. It does not make
the historical 53.44-minute result acceptable, and the earlier debug result is not
a valid release-build implementation-speedup denominator.

For fewer than 20 latency samples, show median and maximum rather than implying a
robust tail estimate. Search/index consistency, provenance, revision CAS, retention
limits and deletion must pass irrespective of timing. Do not shorten content or
reduce record count after a slow result.

Monitor the complete owned Vitest process tree with the normal two-worker setup.
The measured P13 two-worker peak was approximately 0.55 GB; a candidate peak above
1 GB or a material unexplained increase needs investigation. Shared working sets
can double-count pages, private commit differs from RSS, and one-second samples
can miss peaks. Higher-worker qualification is separate and requires its existing
headroom guard. Never suppress tests to meet the budget.

Record time series for repeated editor/panel/project/terminal operations, browser
heap/DOM and owned server/child processes. Distinguish retained presentation state,
lazy-loaded modules, bounded caches and database growth from a leak. No universal
"no leaks" claim follows from a short stress run or one forced-GC measurement.

Provider response time, tool compilation, optional language-server indexing and
the default 180-second lease plus 60-second grace are measured separately from
ordinary interactive latency. Faster lease tests cannot qualify real timing.
