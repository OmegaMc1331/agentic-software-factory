# Source and integration review

The existing automated Reviewer and P08 integration coordinator remain authoritative. The Review workspace reads actual task attempts, captured patches, base/head identities, test-command reports, AgentSessions, handoffs and integration journals. A Worker report is labelled as a report; actual combined integration checks remain separate. Missing reviews are shown as missing.

## Read-only evidence

Existing role-aware mission construction supplies the Rust-captured implementation patch and Git snapshot to the Reviewer. P12 records the exact snapshot and instruction digest against the real prepared AgentSession and whether the captured patch was actually supplied. This uses the invocation's evidence, not a later lookup or an invented model inspection claim. It grants no shell or filesystem write access. Earlier sessions without this audit remain historical and are not retroactively labelled verified.

Reviewable changes are resolved within the owning repository. The UI can compare recorded commits through the same controlled Git service and inspect managed worktrees read-only. Actual Reviewer statements and independent verification are required for real acceptance; merely displaying the patch is insufficient.

The browser review-detail route revalidates both committed changed paths and recorded evidence paths against current project read policy, protected-path validation and P05 native file guards before returning the captured patch. Evidence-path enumeration includes both sides of renames. Historical evidence is retained in the database when current access is denied. A review record is not a bypass for reading a protected file or a currently hard-linked source.

The shared runtime evidence capture also uses NUL-delimited, rename-expanded changed paths. This preserves original filenames for existing write/read policy checks and avoids lossy trimming/quoting of names containing spaces or Unicode. Presentation-level rename detection remains available in Source Control.

## Human feedback

Human comments are separate durable records with actor, command identity, attempt, base/head and optional path/side/line. Rust validates that a path belongs to the real diff and a line exists in the selected text revision. Rendering is inert. A new task attempt/head marks older anchors outdated; the comment is not silently attached to another line.

A human request for changes fences pending integration publication. Its transaction races safely with the coordinator's transition to Publishing: a change request cannot be accepted as a new publication fence after that transition already owns the operation. Explicit human resolution/approval annotation clears only the human block. It cannot create an automated Reviewer approval, invent passing tests or bypass integration validation. Feedback on an already integrated result remains feedback rather than retroactive rollback.

Before remote delivery, unresolved human findings on published task attempts are rechecked. New change requests are rejected while a delivery owns its pushing/PR-creation phase. A completed mission with a later published repair labels its earlier conflict as historical; the original conflict journal and worktree remain inspectable. An active or unresolved mission continues to show recovery required.

## Conflict recovery

P12 exposes actual unmerged index stages and preserved integration journal records. It never chooses a merge side. Conflicting candidates, original task commits and failed checks remain available. The established P08 drained revision barrier and configured Planner can introduce bounded repair work on the current approved integration head. That work requires normal tests, review and coordinated integration. A blocked/exhausted recovery remains visibly blocked. Ordinary editor and staging actions cannot write into managed candidate worktrees.

P09 original/replacement sessions retain their own authorship and execution snapshots. P10 checkpoints and suspended worktrees are inspected without restarting a mission or publishing. No Source Control inspection modifies P11 memory; extraction remains governed by its existing policy.
