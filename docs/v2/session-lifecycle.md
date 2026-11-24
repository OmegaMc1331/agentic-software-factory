# Workspace and mission lifecycle (P10)

P04 `s_` workspace identities retain their navigation meaning. Migration 22 adds a project-local lifecycle record for explicitly registered workspace sessions and immutable ownership metadata for new browser-managed runs. The P04 application layout store, P07 conversation, numeric run, AgentSession and browser lease remain separate entities.

The production workspace binds all restored sessions in open project contexts, including hidden navigation sessions. Selecting another project or closing a presentation view does not change an existing mission's owner. A conversation submission carrying a validated browser lease must refer to one of its acknowledged project/session bindings. Mission creation and ownership insertion share the existing conversation transaction. A closed owner refuses new missions. Existing unmanaged runs are not assigned to a recent or arbitrary browser session.

Session closure is an explicit authenticated, CSRF-protected API operation with a generation check. The confirmation lists associated missions and describes process reconciliation, preserved data and the independent IDE-service lifecycles. The persisted `closed` flag records close intent; each mission retains its separate truthful quiescing/suspended/recovery state. Reopening clears the close intent, without starting a process. The user separately authorizes resume.

| Lifecycle | Meaning |
| --- | --- |
| active | Browser-managed execution is eligible under the ordinary runtime rules. |
| suspend_requested | Durable fence committed; no new dispatch may claim ownership. |
| quiescing | Owned work is being stopped/drained and examined. |
| suspended | Ownership is drained, process cleanup confirmed and checkpoint committed. |
| reconciling | An authenticated resume request owns the next generation and validates state. |
| recovery_required | Uncertain, changed, missing or denied state blocks new execution. |
| settled | The original run's terminal outcome remains authoritative. |

Lifecycle is orthogonal to task and run outcomes. Completed tasks, reviews and old attempts are not renamed to suspended. Generations fence stale callbacks. Suspend/resume command identities are durable and idempotent. The existing P03 project ownership lock remains the multi-process execution boundary. No second scheduler or project lock is introduced.

Supported suspension triggers are explicit session close, last bound client expiration after grace, and an authenticated user suspend request. Runtime interruption is separately recorded as recovery-required; it is not described as graceful suspension. Completion wins when all tasks and publication records have genuinely settled before checkpoint finalization. Cancellation remains a distinct terminal action and preserves prior evidence.

Resume policy is deliberately **manual** for every reason. Returning clients restore presentation and establish presence, then inspect the suspended mission and request validation/resume. Reopening an explicitly closed session is required first. The server never starts browser-managed work solely because it restarted or an unrelated client connected. No lease, workspace ID or browser-supplied PID confers authorization.

There are at most 100 unsettled browser-managed missions per project/session and 1000 per project; reads prioritize unsettled ownership before settled history. Historical checkpoints and conversations are retained. Schema migration uses the existing verified backup and additive migration machinery. Older executables refuse the newer schema; rollback requires a stopped, verified pre-upgrade backup plus reconciliation of newer worktrees and Git history.

## P12 explicit retry compatibility correction

A settled failed/blocked browser mission may use the existing authenticated task-retry action after current policy validation. A short SQLite transaction checks the same project/session and lifecycle generation, an open session, drained claims/scheduler/process ownership, confirmed process cleanup and no uncertain or already-published selected-task integration/delivery. It marks only the eligible failed task ready, reactivates the run, advances the lifecycle generation and records `mission.retry_requested`. Previous attempts, completed siblings, worktrees and checkpoint history are retained. Duplicate/stale retries cannot create another task owner.

Runtime operation reservation distinguishes this explicit settled retry from ordinary start/planning admission. It reserves only the existing run's operation slot; the guarded database transition still must succeed before execution can spawn. A closed session or failed validation releases the slot without dispatch. The runtime regression exercises a real controlled child failure, settlement, closed-session denial, explicit reopen, duplicate retry denial and reviewed integration while preserving the original attempt.

This corrects the inherited settled-state fence that rejected every explicit browser-mission retry. It does not make suspended/recovery-required missions active, does not resume completed/cancelled tasks and does not change manual checkpoint-resume policy. A genuinely completed settled run may separately request explicitly authorized GitHub delivery through P12's full review/test/policy gates; completion settlement is not an execution-resume operation or publication consent.

Retry settlement appends a generation-scoped current outcome to P07 while preserving the original failure message. An authorized conversation observer may reconcile a missing settled-retry presentation event from durable state, even when its progress snapshot already contains the completed status. The short transaction rechecks the pending generation and emits at most one outcome; it launches no agent, resumes no work and does not rewrite prior messages. Once recorded, ordinary polling does not repeatedly acquire that presentation write transaction.
