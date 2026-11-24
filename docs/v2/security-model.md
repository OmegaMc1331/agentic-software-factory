# V2 security model: implemented through P07

The authoritative baseline is [ASF security](../security.md). V2 preserves one loopback listener, authenticated browser sessions, Host/Origin checks, mutation CSRF and authenticated project-scoped SSE/WebSockets. Project IDs are routing identities, not authorization credentials. P03 rejects overlapping registered roots, maintains native project/database identities and owns one runtime per canonical project.

P05 adds [guarded filesystem capabilities](file-service-architecture.md) and [typed scoped file routes](filesystem-api.md). Files are identified by project, backend root and relative path. Protected Git/Factory internals, links/reparse points, hardlinks, path escapes and unsupported writes are refused. Managed task roots are inspection-only. The manual editor has authenticated local-user authority; it does not borrow or broaden an agent role's tools/permissions.

Saves require expected revisions, prepare synchronized temporary files and verify the displaced original after native replacement. Acknowledgment and uncertainty are distinct. Draft persistence is separate from saving. Project/browser draft quotas do not evict dirty data automatically. File operations and search have explicit concurrency, size and traversal bounds. Git reads pin the project's common-directory identity and use validated explicit Git/work-tree/common directories, preventing a worktree marker from redirecting a comparison to another registered repository.

P05 file polling remains advisory; save-time revision validation is authoritative. Existing agent policies, process supervision, worktree ownership and uncertain GitHub-operation reconciliation remain unchanged. P06 introduces the explicitly approved IDE capabilities described below; it does not authorize publication.

Frontend document IDs are bound to project/root/path before cached buffers are displayed. View-derived save/close intents and document-change propagation retain that binding, including across renamed files and worktrees. Verified canonical aliases are invalidated on move/release. These client checks prevent presentation misrouting; they do not replace Rust's authenticated root/path and revision checks.

Local processes/extensions/agents running as the OS user can exceed logical application policies. Git worktrees and project separation are not OS sandboxes. Native replacement/directory-flush behavior differs by platform; directory namespace races and observable-state ABA limits remain documented. Native Linux/macOS, privileged Windows symbolic links and release-grade crash/power-loss qualification remain separate gates.

[Security/functional evidence matrix](editor-test-matrix.md) maps the enforced contracts to executable tests. Dependency/packaging evidence (historical record held privately) records pinned packages, notices and advisory queries without implying that an empty advisory result proves absence of vulnerabilities.

## P06 additional trust boundaries

Approved tool profiles select executable/argument vectors on the server. Launch calls select a project-scoped profile ID and an authorized writable root, with explicit user confirmation. The environment is cleared and only documented OS/toolchain variables are inherited. Executable and absolute file-argument bytes are fingerprinted at approval and rechecked at launch. This is not whole-dependency attestation and does not remove executable/path replacement races against a hostile local OS user.

LSP JSON-RPC and DAP transports have bounded framing, queues, pending requests and event history. Unknown server requests, dynamic registration, arbitrary server commands, DAP `runInTerminal` and unrestricted PID attach are refused. The optional js-debug target-session reverse request is restricted to one adapter-owned opaque target on the same private endpoint; it accepts no browser/adapter-selected executable, PID or remote host. Protocol crashes terminate the owned process tree and report failure.

Language edits are untrusted proposals. Rust verifies returned-edit provenance, open-document versions, project/root identities, protected paths and current P05 revisions. Applying reviewed edits changes shared buffers; normal P05 saving remains authoritative. Resource create/delete/rename edits are refused. Debugger sources resolve through the same file authorization. Agent worktrees remain read only and cannot launch these processes.

Development terminals are explicit local-user shells, separate from configured agent consoles. Authenticated WebSockets retain Host/Origin/session checks and project ownership. Existing bounded PTY history, per-subscriber queues and overflow notifications are reused. Input is at most 64 KiB; slow socket writes have a deadline. View closure does not terminate a terminal; explicit Stop does. Windows Job Objects cover owned protocol and terminal descendants. Unix process groups are implemented, but abrupt-server-death guarantees and native PTY/adapter qualification remain pending on those platforms.

Extension API 1 accepts declarative JSON only: host-defined editor operations, literal diagnostics, approved syntax aliases, commands/keybindings, text panels, settings and references to already approved tool profiles. It executes no arbitrary JavaScript, install scripts, archive contents or VS Code extension code. No raw filesystem/process/network capability is available. Unknown capabilities, traversal-like entrypoints, unsupported versions, invalid dependencies and integrity failures are refused. Declarative limitations are explicit rather than descriptive permissions attached to unrestricted code.

IDE metadata uses an application SQLite sidecar with additive schema migration and bounded audit attempts. Corruption fails closed for IDE features while project files and V1 workflows remain available; recovery never deletes a project database. Protocol output and user-invoked debug evaluation can expose project/runtime information to the authenticated local user. Raw stderr is withheld and known credential values are redacted from tool errors; this cannot identify arbitrary secrets embedded in program output. No OS sandbox or universal secret-detection guarantee is claimed.

See [P06 test matrix](ide-integration-test-matrix.md) and delivery (historical record held privately) for executed versus pending native/platform/CI evidence.

## P07 conversation control surface

`factory-conversation` accepts a finite, strictly parsed action schema. Model output cannot supply an API path, shell command, target project, permission override or external-publication policy. Mission creation resolves only existing recipes, roles and agents through Factory Core; authored plans pass the existing DAG/role/operation validation. The same runtime owns execution, policy, worktrees, cancellation and evidence. Natural-language text is not a new authorization credential. Agents still retain their configured OS permissions; action validation does not sandbox their process.

Each conversation, command, run association, event and file reference resolves inside one ProjectContext. Durable command keys plus request fingerprints prevent acceptance retries from creating additional logical commands. Run creation and association are transactional. A restart marks unfinished conversation commands interrupted and relies on existing runtime reconciliation; it never blindly retries uncertain external effects. Stream subscribers have no execution ownership.

Context is explicit and bounded. Rust reopens P05-authorized project/root/path references, checks revisions, refuses protected/ignored/secret-pattern files and validates UTF-8 selection boundaries. Unsaved content is explicitly identified and remains untrusted data. Stored messages, context and output are redacted against known configured/inherited credential values; this is not a guarantee that arbitrary secrets embedded in source or agent prose can be detected. The user should attach only intended content. The backend does not automatically scrape the whole repository.

Prompts separate fixed role/action instructions from serialized conversation/file data. Adversarial text cannot add a new action variant or cross-project file capability. Within the permitted `create_mission` action, an existing coding agent can nevertheless act on malicious repository content under its OS authority; prompt structure is not a complete semantic injection defense. Existing role/policy safeguards and explicit remote-publication restrictions remain necessary. The conversation does not confer new MCP, terminal, extension or GitHub permissions.

Messages are rendered as text and fenced code; generated HTML, scripts and URLs are inert. File/mission/session links are constructed by ASF from validated structured identities. Events are project/conversation scoped, durably sequenced and delivered through bounded authenticated SSE with nonblocking subscriber queues. Stored history is paginated; raw CLI logs remain separate from chat. See [interaction](orchestrator-interaction.md), [events](conversation-events.md) and [test matrix](conversation-test-matrix.md).

Local retention is explicit: archiving preserves messages, context snapshots and evidence in the project's SQLite database. There is no per-message secure erasure or history-purge UI in P07. Stop ASF and use verified project database backups for whole-project recovery/removal; deleting a conversation tab removes no data. Secrets should not be entered into chat. P10 browser leases/checkpoints and P11 long-term memory remain separate work.

### P07-R1 authorization failures

Factory records native Codex policy failures separately from code-quality review. A read-only native startup mode for an implementation task, or a recognized native pre-process policy rejection, stops automatic worker/reviewer cycling. Finite agent-reported permission blockers are labelled as reports with an unverified denying layer; they never grant permission. Unknown blocker schemas are validation failures. Actual process exits and output remain available, even when exit zero accompanies a blocked mission. This is conservative failure handling, not a new approval transport or OS sandbox. See [the direct permission diagnosis and regression matrix](p07-coding-permission-diagnosis.md). No default CLI permission, manual managed-worktree write rule, MCP grant or publication safeguard is broadened.

## P08 execution boundaries

Parallelism does not add authority. Every invocation retains project/run/task/attempt/session ownership, policy deny precedence and scoped MCP grants. All agent routing paths reserve configured capacity. Untrusted Planner output is restricted to typed unfinished tasks, roles, operations and dependency references; it cannot install a command, broaden write scopes or select another project.

P08 task processes use the shared existing process-supervision primitive. On Windows a kill-on-close Job Object is assigned; new P08 run dispatch refuses unavailable supervision before sending mission input, without changing a parent policy. There remains a spawn-to-assignment window, so OS-level universal containment is not claimed. Unix uses the existing separate process group with explicit termination; native qualification remains separate. PID/timing metadata is durably recorded at launch and retained on interruption. It does not authorize arbitrary attachment or PID termination.

Reviewer remains read-only and receives a bounded Rust-captured patch and base/head/hash facts, including new files captured as a source commit. This addresses read-only CLI Git-inspection limitations without granting write access. A rebased candidate gets freshness review and configured checks before compare-and-swap publication. Conflicts and uncertain effects are retained rather than force-resolved or replayed.

Integration snapshot roots are separate read-only P05 identities; manual saves, extension writes and process-root requests remain refused. The Worker may write only its own authorized task workspace under its actual CLI/host permissions. No blanket approval, sandbox bypass, unrestricted conversation shell endpoint or GitHub publication path is added.

Provider usage exhaustion is not authorization. The qualified native Codex diagnostic maps to `AGENT_RESOURCE_EXHAUSTED`, stops bounded recovery and retains partial evidence. ASF does not buy credits, switch credentials, escalate CLI permissions or reinterpret a quota refusal as a Reviewer verdict.

## P09 profiles and replacement

Profile requests select existing trusted TOML installations; they cannot supply executable paths, arbitrary arguments, environment secrets or credentials. A structured model identifier is passed as an argv value, not shell source. Effective profile restrictions narrow application/project/role/installation policy, and review-class profiles remain read-only. Empty P09 read scopes deny ASF-provided file excerpts and source evidence. Direct native CLI reads still depend on its sandbox and host: these are not claimed to be intercepted by ASF. Effective policy responses expose the additional restriction layers.

Immutable session snapshots and revision checks separate active execution from future profile edits. A replacement is independently authorized and gets new session/attempt-scoped tool grants. It does not inherit the previous process's permissions. A task claim remains unique across the handoff, with the old process confirmed exited and its Git evidence retained before a new attempt. Known scoped denials have durable rejected records; foreign/unknown profiles disclose no cross-project information. Archival never deletes historical evidence. See [permission governance](agent-permission-governance.md) and [handoff boundaries](agent-handoff.md).

## P10 lifecycle security

Browser lease IDs do not authenticate requests. Registration, renewal, release and lifecycle mutations retain existing signed sessions, Host/Origin checks and CSRF. Lease ownership hashes the valid authenticated session; client ID/generation/sequence and exact project/workspace bindings are checked independently. Profile, checkpoint and lifecycle IDs cannot select arbitrary operating-system processes. Only Rust-owned AgentSession metadata is used for cleanup and reconciliation.

Scheduling fences and generation CAS persist in each project's database. Resume revalidates effective current policy, provider/profile eligibility, worktree identity, Git state and external-side-effect uncertainty. Unsupported checkpoint versions and malformed Git references fail closed. P07 notification quota errors do not undo a suspension fence. Native job/group supervision is not an OS sandbox; parent/host restrictions remain authoritative. Windows process-tree evidence cannot qualify Linux/macOS behavior.


## P11 project memory integration

P11 memory is untrusted factual context, never a Rust policy layer. Provenance resolution reuses P05 filesystem authorization and ignore rules. Credentials and protected sources are screened with documented detection limits. Source citations are independently resolved; project databases, CAS and conditional context validation prevent cross-project access and stale-context commits. See [governance](memory-governance.md).


## P12 Git and delivery boundaries

Typed Source Control operations reuse P03 ownership and P05 guarded roots. Managed agent/integration worktrees and factory refs remain coordinator-owned. Index changes use literal pathspecs, expected generations, Git locks and an alternate index. Read inspection suppresses fsmonitor, configured filters, external diff and textconv. Active filters remain unsupported for manual mutations; approved hooks/signing require explicit local-program trust, which cannot widen command policy. Git is not claimed to be OS-sandboxed.

Publication requires repository-scoped explicit authorization, current integration identity, effective policy, authenticated GitHub identity/permission and existing delivery ownership/reconciliation. Read-only API responses, memory and comments cannot authorize writes. The V2 publication endpoint additionally checks effective push URLs and rejects rewrites to a different/local destination. Legacy controlled transport fixtures retain their existing supported behavior. See [Git security](git-security.md) for residual local configuration/path races and credential-redaction limitations.


## P13 integration boundaries

Application preferences are strictly typed presentation data in the existing application store, not policy configuration. The project settings summary excludes secret-bearing environment/command/header fields and does not invent a new permission resolver. Commands and deep links reuse owning project/root/resource authorization; a resource ID or shortcut grants no access. Restoring layouts cannot submit missions, resume P10, execute commands, stage/commit or publish.

Durable notifications originate in actual database transitions or typed Rust service failures. Fixed generic text and hashed service provenance avoid copying raw objectives, files or agent output. OS notifications require an explicit saved preference and browser permission and contain generic application guidance only. Notification mutation and preferences use existing auth/Host/Origin/CSRF. See [notifications](notifications.md). Memory/Git performance changes retain provenance, secret checks, conditional revisions, path/ref validation, filter inspection, common/index locks and worktree ownership. Native Git helpers and hostile-local-path races retain their documented trust limits.
