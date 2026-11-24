# Using the V2 workspace

Open `/workspace.html` or use `factory start --workspace` with the explicitly
authorized development directory. Existing users can still open the V1 `/` entry.

Use the project selector to register an existing project, create a project or clone
when its network operation is authorized. Trust acknowledgement allows configured
local tools; select only repositories you intend to trust. Reload restores
presentation state, not execution side effects.

Explorer opens authorized files in shared CodeMirror documents. Save acknowledges
the actual Rust write; on conflict preserve your draft and inspect the disk version.
The Focus, Code, Agents and Review layouts rearrange views without changing mission
ownership. The command palette and keyboard reference list the actual available
commands. A terminal retains its own input shortcuts.

Use Conversation for the orchestrator and mission objective. Configure an available
agent installation/profile and model through Settings/Agents before execution.
Planner output is validated by Rust. Progress shows actual tasks, attempts, review
and integration. A completed Worker is not necessarily an integrated mission.
Agent replacement uses a safe handoff boundary and preserves earlier AgentSessions.

Closing an editor, project presentation or hidden browser tab does not explicitly
cancel a mission. Closing its owning workspace session or expiration of the last
eligible browser client can request controlled suspension. Reopen the session,
inspect checkpoint/recovery state, then resume manually. A crash may require
reconciliation rather than displaying a false safely-suspended status.

Memory contains project-scoped categories, revisions and provenance. Create or edit
facts, review proposals, disable obsolete records or delete them through the Memory
activity. Inspect source validity. Memory is context, not an enforceable permission
policy. Disabled/deleted records must be excluded from future invocation context.

Source Control separates index and working-tree changes. Open the displayed diff,
stage a file or a supported hunk, then inspect staged files before committing.
Stale previews fail safely. Dirty editor buffers block unsafe branch changes.
Managed agent/integration worktrees are inspection surfaces owned by the runtime.
Use Review to inspect genuine captured changes, test results, Reviewer decisions
and human feedback. Recovery goes through the integration coordinator.

GitHub views show real remote data or a truthful unavailable state. Delivery preview
is side-effect free. Publishing requires effective policy and explicit authorization
for the intended repository/branch/commit. A local branch or preview is not a PR.

Settings distinguishes application preferences, workspace presentation, project
configuration and agent profiles. Effective denials remain authoritative. Some
advanced configuration is still file-backed and is labelled accordingly. Optional
LSP/DAP/terminal profiles require actually installed approved tools. Extensions are
the supported declarative ASF API, not arbitrary VS Code extensions.

Notifications retain important genuine runtime outcomes. Open a notification to its
owning project/resource, mark it read or dismiss it. Browser notifications are
optional and require an explicit saved preference and browser permission.

On failure use the view's Retry, Diagnostics or Settings action. Preserve drafts,
evidence and uncertainty. Do not solve an error by deleting databases, forcing Git,
granting broader agent tools or blindly replaying external side effects.
