# Source Control architecture (P12)

P12 extends `factory-git::Repo`, project-local `factory-db`, existing `Factory` reviews/delivery, P03 contexts and P05 roots. The browser presents typed intentions. It does not execute Git, own refs, authorize filesystem paths or maintain another integration state machine.

Every request binds an immutable project ID, a P05 root ID and the selected worktree. `gitCommonIdentity` hashes the resolved common directory. P03 still owns the runtime and Git-common OS ownership locks. A separate per-operation `asf-git-operation.lock` coordinates local mutations across Repo instances; existing worktree/commit/ref/rebase operations use it too. It is not another project owner or scheduler. Manual mutations wait at most five seconds for that lock. Existing scheduler operations use the configured bounded Git deadline (120 seconds by default) so an eight-Worker queue is not incorrectly failed by the shorter UI admission window. Neither path bypasses the shared lock. Git's index/ref locks remain necessary for coordination with external Git clients.

The primary user root supports bounded manual index, commit and branch operations. Every registered managed task or integration root remains inspection-only, even after its task exits. `factory/` refs remain coordinator-owned. P12 introduces no worktree deletion, branch deletion, force checkout, clean, hard reset, stash/drop or force push action.

## Observed state

Status uses porcelain v2 with NUL delimiters. It distinguishes index/working flags, rename source, conflicts, untracked paths, unborn and detached HEAD, tracking and ahead/behind. UTF-8 paths containing control characters remain representable in status but are unsupported for portable manual operations. Non-UTF-8 metadata fails explicitly. Reads suppress fsmonitor and configured filter programs. Filtered repositories can therefore require a trusted external Git client rather than an unqualified claim that canonicalized content was compared normally.

The response includes HEAD, an index digest and a generation over Git status/index and bounded changed-file observations. This is not an atomic filesystem snapshot. Supported staged selections require complete fingerprints; text comparisons and manual file staging are limited to 1 MiB per selected file, with a 64 MiB aggregate fingerprint budget. At most 10,000 changed/tracked entries are inspected by the applicable operation. Over-budget data is not presented as safely stageable. Large already-staged objects remain identifiable through the actual index/commit.

Status refreshes manually, after confirmed mutations and at a bounded ten-second interval while the repository component is mounted. Only subscribed views poll. Editor saves remain P05-owned. Branch switching checks live dirty buffers and persisted P05 recovery drafts, retains drafts and refreshes document revisions without overwriting dirty buffers.

## Persistence and compatibility

Additive migration 24 stores local operation claims/results, versioned human feedback and Reviewer context-usage metadata. Existing P03–P11 records remain authoritative. A duplicate successful command returns its original result; an executing/uncertain command requires inspection rather than blind replay. Local operations and Git are not one SQLite transaction. A process crash between them can require reconciliation against actual refs/index and the retained operation record.

History is paged, branches bounded and review lists are metadata-only SQL pages. Detailed agent output remains lazy through existing AgentSession routes. Integration summaries use bounded pages and explicitly label oversized details. No status, staging, diff or history action invokes memory extraction or scans the P11 memory library. P10 manual resume and browser-presence policy remain unchanged.

See [staging](git-diff-and-staging.md), [review](review-workflow.md), [Git security](git-security.md), [GitHub delivery](github-delivery.md) and the delivery report (historical record held privately) for executed qualification rather than architectural promises.

The eight-request Source Control budget belongs to each `FactoryApplication` and is cloned into all of its project route states. Separate application instances have independent budgets. Project filesystem admission remains capped at four and uses the existing bounded wait. The authenticated budget-isolation regression exercises actual project routing with one saturated application, another healthy application and permit release.
