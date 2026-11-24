# Controlled mission suspension

P10 extends the existing Factory/Runtime execution path. Rust first persists a suspension generation and reason in project SQLite. P08's scheduler-owner acquisition, atomic task claims, new agent invocations and replan commits check that fence. Revoked run MCP grants prevent new tool use while cancellation propagates. The browser does not own admission control.

Runtime signals its owned operation's cancellation flag. P08 stops dispatch, drains task completions, stops supervised CLI processes and joins task threads. Worker and Reviewer cancellation retain interrupted attempt history rather than code-quality or user-cancel outcomes. Existing worktrees, captured evidence, source bases, plan revisions, immutable P09 snapshots and unapproved changes remain available. RAII task/agent/operation permits are released after actual owned work exits.

Windows retains the existing kill-on-close Job Object and adds a bounded accounting check that its active process count reaches zero. The direct child is reaped and cleanup observation is stored separately from process start/finish metadata. P10 does not widen CLI sandbox permissions. Where an external CLI offers no cooperative interruption protocol, supervised termination is the available fallback; it does not preserve inference state. [Microsoft's Job Object contract](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects) covers normal descendant ownership and nested-job accounting. The existing spawn-to-assignment window and descendants created through mechanisms outside that ownership remain explicit limits.

POSIX execution retains scoped process groups. Native Linux/macOS behavior requires its own qualification. Unknown or unconfirmed cleanup cannot yield SUSPENDED. No browser can supply an OS PID to a termination route. P03 project locks and project-scoped runtime maps prevent another project's Worker from becoming a target.

The integration coordinator retains its publication journal and compare-and-swap branch update. A mutation that already published is reconciled as published, never rolled back by presentation state. Uncertain integration or remote delivery prevents checkpoint/resume until reconciliation. Replanning can finish producing untrusted output, but a fenced or stale proposal cannot replace the committed revision.

Pending P09 handoffs remain durable and cannot launch while fenced. Resume checks the current target profile revision, permissions and captured evidence before selecting a replacement boundary. Old AgentSessions and model snapshots remain historical records; no model-internal state is transferred.

A checkpoint is finalized only after Runtime ownership is drained and the database confirms no active scheduler/dispatch, running agent, running tool invocation or unresolved integration journal. Failed storage, unavailable worktrees or unverified cleanup enter recovery-required. The UI must not infer suspension from a dropped SSE connection.

Development terminals, LSP, debug adapters and extension services keep their P06 project/service ownership. Session or mission closure does not terminate unrelated user terminals, shared language servers or debuggers. Their existing explicit stop/project-close operations remain available.
