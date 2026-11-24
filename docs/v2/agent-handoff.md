# Safe agent handoffs

P09 uses existing P08 task claims, attempts, process supervision, worktrees, review and integration. A handoff is reconstructed task context, not transfer of provider-internal model state or hidden reasoning.

Requests bind project, run, task, target profile revision, reason and idempotency key. Rust validates ownership, role, executable availability and effective policy. A SQLite transaction checks current task eligibility, profile revision and the one-active-handoff constraint. Duplicate identical commands return the same record; a changed payload or competing active request conflicts.

Supported boundaries are before dispatch and after a confirmed implementation-family Worker exit with captured Git evidence, before main review starts. Requests arriving after review begins, during integration or after a terminal task state are refused. Pending advisory/review tasks may be reassigned before dispatch; active advisory/planning/review-process replacement is not claimed. Immediate interruption is unavailable through the handoff UI; ordinary P08 mission cancellation remains separate.

States have concrete meanings:

| State | Durable meaning |
| --- | --- |
| waiting | Current Worker must exit; no replacement has started. |
| ready | A safe boundary was verified and the target assignment is persisted. |
| executing | A new replacement session and immutable launch snapshot exist. |
| completed | The replacement executed and the task completed through review/integration. |
| rejected | A known, scoped replacement was refused by authorization; no process starts. |
| failed | Eligibility, execution, review, recovery or uncertain interruption prevented completion. |
| cancelled | The owning mission was cancelled. |

At Worker exit ASF releases the old agent capacity, validates evidence, commits captured changes and atomically selects handoff versus review. The preceding attempt becomes `handed_off`, with its evidence and session retained. The task keeps the same durable scheduler owner and worktree. A new bounded attempt receives independently resolved permissions and profile configuration. The original source base is retained, so the final Reviewer evaluates all preserved and replacement changes together. Captured partial work is explicitly unapproved.

Context includes the task and mission contracts, current dependency facts, previous session/attempt identity, handoff reason, captured commit/patch and current authorized worktree. Existing mission construction supplies acceptance criteria and policy. Handoff evidence is bounded to 65,536 bytes in the prompt; an explicit omission notice replaces oversized patch context. The replacement must inspect and verify preserved work. It does not inherit MCP grants, process permissions or approval decisions.

The ordinary attempt limit also bounds handoffs; a request on the last attempt is refused. Old agent permits are released before the replacement reserves capacity. Workflow/project/application permits continue to account for the owned task. There is never a second scheduler claim for the same attempt. Legacy routing edits cannot race an accepted handoff or a claimed dispatch.

P08 reconciliation marks uncertain interrupted attempts and handoffs accurately, retains worktrees, closes stale claims and does not blindly replay external effects. A pending before-dispatch reassignment may survive restart because no process has started. An uncertain running handoff fails for inspection rather than pretending to resume an in-flight model. P10 browser leases, browser-close suspension, model checkpoints and general resume are excluded.

Evaluation treats a handed-off attempt as non-quality interruption for aggregate historical metrics; its precise domain status and handoff evidence remain available. A handoff is not reported as successful merely because assignment metadata changed.

History is paged in batches of 200 with an exclusive handoff-ID `before` cursor. Each workflow admits at most 1,024 accepted/rejected audit records. Known scoped permission rejections are retained idempotently, while unknown/cross-project targets disclose no foreign profile data.

## P10 suspension boundary

A lifecycle fence prevents handoff reconciliation from launching a replacement. Waiting requests, target profile revision and the interrupted original attempt remain durable. Resume validates the current target profile and independent effective role permissions before moving an eligible request to the `reconciled_suspension` boundary. The interrupted attempt's recorded exit/status is preserved; it is not rewritten as a successful handoff. No capacity is transferred from an unverified live process. Ordinary P09 boundaries and review requirements remain unchanged.


## P11 project memory integration

P11 reconstructs eligible current project-memory context for the replacement invocation after the existing safe boundary. Original attempts retain their original usage references; no hidden inference state transfers. Deleted/disabled memory is excluded from new selection. See [retrieval](memory-retrieval.md).
