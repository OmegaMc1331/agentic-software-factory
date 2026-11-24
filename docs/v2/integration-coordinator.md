# Integration coordinator

One durable scheduler owner serializes a run's integration lane; partial indexes additionally prevent two preparing/publishing records in that run. Different projects/runs retain their own refs. Workers and primary reviews may overlap.

1. Persist Worker commit, evidence and actual approving Reviewer decision, then AwaitingIntegration.
2. Atomically claim Integrating. Create a journal record with before/source SHAs and a separate candidate worktree at the reviewed source commit.
3. If the candidate diverges from the current run head, rebase the candidate onto that immutable head. The original task branch is never rebased or overwritten. Conflicts retain both worktrees and are recorded.
4. Review a stale-base candidate again, using its actual patch against the updated integration baseline.
5. Execute configured trusted `[[integration_checks]]` (program, argv, timeout 1-300 seconds; at most eight). They run with null stdin, bounded output, cancellation and process-tree supervision, filtered effective Worker environment and deny-first command policy. Model output cannot configure these commands. Checks that dirty the candidate or change its committed HEAD cause policy refusal. Reviewer mutations are refused as well. Actual Git conflicts, validation failures, policy refusals and cancellation have distinct journal statuses; a failed test is not labelled a Git conflict.
6. Journal Publishing with candidate SHA and check evidence, then compare-and-swap `git update-ref` from the expected prior head. Main is not checked out or merged.
7. Atomically record Published, run integration SHA and task completion. Propagate dependency readiness. Record cleanup failures without retracting an already published ref.

Every approved candidate is checked before publication. A clean Git rebase is not asserted to prove semantic compatibility. Projects must configure suitable combined checks; without them the record accurately contains no independent check executions. Independent acceptance additionally verifies a fresh checkout of the final integration branch.

A failed check, freshness rejection or Git conflict preserves source commits and candidate. The drained revision barrier may invoke the real configured Planner to create repair work against the latest approved run head. No side is silently chosen. The new repair task requires normal review and checks. Attempts and conflict records remain queryable.

Publication uniqueness is per attempt; legacy V1 specialized-review rework may create another explicit attempt for a task. P08 completed tasks are immutable and repaired using new task IDs. After a crash, a Publishing record whose exact candidate is already the actual run ref is finalized without another Git mutation; otherwise it is Interrupted and requires inspection. No exactly-once external process or multi-file transaction claim is made.


## P12 inspection and human feedback

Source Control and Review observe this coordinator's actual task commits, candidates, tests and conflict records. They do not merge managed worktrees independently. Human comments retain base/head/attempt and optional line anchors. A request for changes fences the short transition to Publishing transactionally; a late request cannot pretend to have blocked a publication already owned by the coordinator. Explicit human resolution clears only that human fence. Configured automated Reviewer approval and combined checks remain required. Read-only Reviewer context-use audit records the actual captured patch/snapshot supplied to each new invocation. See [review workflow](review-workflow.md).
