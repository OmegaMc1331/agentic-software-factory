# P03 - Application and project API contracts

All endpoints use the same loopback server, browser session cookie, Host and Origin validation and existing CSRF middleware. Obtain a session with `POST /api/auth/session` from an allowed origin. Every mutation requires `X-Factory-CSRF`; unauthenticated project management is refused. IDs identify resources; they are not authorization credentials. This application serves one trusted local user's registered projects.

## Application endpoints

| Method / path | Input | Success |
| --- | --- | --- |
| GET `/api/application` | none | immutable `default_project_id`, `authorized_roots`, limits |
| GET `/api/projects` | none | deterministic `ProjectView[]` |
| POST `/api/projects/register` | `path`, optional `display_name`, `trust_project: true` | 201 persisted `ProjectEntry`; canonical duplicates reuse the entry |
| POST `/api/projects/create` | `destination`, `display_name`, `trust_project: true` | 201 new Git + Factory project entry |
| POST `/api/projects/clone` | `remote`, `destination`, `display_name`, `initialize` (default false), `allow_network` (default false), `trust_project: true` | 201 cloned registered repository |
| GET `/api/projects/:id` | validated ID | 200 `ProjectView` |
| PUT `/api/projects/:id` | `display_name` | 200 renamed view; ID unchanged |
| DELETE `/api/projects/:id` | none | 204; closed and exclusively owned registry entry removed, files preserved |
| POST `/api/projects/:id/open` | none | 200 open view; single-flight and OS lock protected |
| POST `/api/projects/:id/close` | none | 200 closed view; busy projects receive 409 |
| POST `/api/projects/:id/initialize` | `trust_project: true` | 200 initialized closed view; existing `.factory` never overwritten |
| POST `/api/projects/:id/relink` | `path`, `accept_identity_change` (default false), `trust_project: true` | 200 closed view retaining ID; filesystem/DB identity change requires acknowledgement |

Management JSON uses snake_case, rejects unknown fields and does not accept a browser-generated authorization root. `ProjectEntry` fields are `project_id`, `canonical_root`, `display_name`, creation/open timestamps, schema version and physical root/Git/database identity strings. `ProjectView` adds `lifecycle`, `availability`, and nullable `problem`. The last two describe failure without removing the row. An initialized V1 directory without Git remains compatible; new project creation initializes Git.

Example registration request:

```json
{"path":"C:/work/project B","display_name":"Project B","trust_project":true}
```

The path must be under a directory authorized by the process at startup. Initialization is separate from registration. A clone with existing initialized Factory data preserves it even when initialization was requested; incomplete pre-existing Factory data is retained for explicit recovery. Creation/clone reserves a new destination; failure retains the directory for explicit recovery and never silently replaces an existing folder. A timed-out mutation can still be finishing: inspect the registry and filesystem before another create/clone attempt.

## Project-scoped operations

Prefix every existing project endpoint with `/api/projects/<project_id>` instead of `/api`. Handler request/response contracts from [the V1 API](../api.md) remain applicable:

| Scope | Routes after the project prefix |
| --- | --- |
| Workflows | `/runs`, `/runs/from-issue`, `/runs/:id`, `/runs/:id/start`, `/runs/:id/cancel`, team, artifacts, recipe |
| Tasks / routing | `/tasks/:id/retry`, artifacts, routing-preview, routing-decisions, routing |
| Agent configuration | `/agents`, `/agents/:agent/tools`, `/roles` and role assignments/preference/policy/tools |
| Recipes | `/recipes`, `/recipes/:id` |
| Configuration | `/config` with existing revision/optimistic-concurrency behavior |
| Graph | `/graph`, `/graph/workspace` |
| Performance | `/performance/agents`, `/performance/agents/:agent` |
| Sessions | `/agents/:agent/sessions`, `/sessions/:id`, `/sessions/:id/stream`, `/sessions/:id/terminal` |
| MCP | `/mcp/servers`, server mutation/test/tools, invocations, metrics |
| GitHub / delivery | `/github/status`, `/runs/:id/delivery`, `/runs/:id/pr-preview`, `/runs/:id/pull-request`, `/runs/:id/delivery/reconcile` |
| Context search | GET `/context/search?q=...` |

For example, A's `GET /api/projects/p_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/runs/1` and B's `.../p_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/runs/1` resolve independent database rows. A nonexistent resource in B never searches A. Unknown, removed, unavailable or closed scopes never fall back to the default. Scoped responses include `X-Factory-Project`.

Context search limits the query to 512 UTF-8 bytes and returns at most 30 `{path,line,snippet}` hits through the existing `ContextEngine`, with its ignore, traversal, symlink/junction, byte-budget and secret-exclusion rules. It is not a generic filesystem API or a file editor service. P05 will implement document access separately.

SSE: existing `session`/`error` events and keep-alives, authenticated cookie, per-project DB and context lease. WebSocket: same Host/Origin/session checks, project-scoped terminal lookup, bounded subscription and existing `input`/`resize` commands. Terminal creation uses the configured interactive agent invocation; P03 does not expose arbitrary shell command execution. Session deletion keeps its existing 202 response. Workflow cancellation keeps its existing 204 response.

## Error semantics

Management/scoping errors return `{error,code}`:

| Status | Code | Meaning |
| --- | --- | --- |
| 400 | `invalid_project` | malformed ID/body/path/name/remote/trust acknowledgement |
| 403 | `project_path_forbidden` | canonical path outside startup allowlist |
| 404 | `unknown_project` | unknown/removed ID |
| 409 | `project_conflict` | closed/busy/duplicate owner or lifecycle conflict |
| 409 | `project_unavailable` | filesystem, project DB, runtime lock, initialization or migration failure |
| 429 | `project_limit` | registered/open/management/request capacity exhausted |
| 503 | `registry_unavailable` | authoritative registry cannot be read safely |
| 409 | `no_default_project` | unscoped project API without a startup default |

Authentication errors and existing project-handler errors retain their existing status/JSON contracts. Invalid management JSON is rejected by Axum's JSON extractor (400/415/422 depending on syntax/content type/schema); callers must inspect status rather than assuming every rejection has `code`. Shared runtime operation/terminal caps map to 429 before creating project state.

## Legacy compatibility and frontend state

`factory start` from a V1 repository registers a stable default and binds unscoped project APIs to it for that server's lifetime. Another browser's selection cannot change it. `--no-default-project` disables unscoped project dispatch explicitly. A broken startup default is exposed as unavailable while other contexts can open. The original `ApiState`/`router` embedding API remains supported; the CLI uses `FactoryApplication`/`application_router`/`serve_application`.

The P03 `/projects.html` production entry uses P02 controls/themes, the existing authenticated API client, real path-entry forms and real registry state. V1 navigation links to it; V1's workbench remains intact. Selected context is a validated registry ID in browser-local storage. Reads carry that ID, use independent AbortControllers and discard stale generations. No frontend current-project value can redirect a backend operation. Only one selected project's snapshot is held; switching clears it and checks its identity before rendering. P04 will own persistent workspace/project tabs.

## P04 application workspace persistence

The same authenticated application router exposes `GET /api/workspaces` (last-updated descending list), `GET /api/workspaces/:id` and `PUT /api/workspaces/:id`. IDs are `w_` plus 32 lowercase hexadecimal characters. PUT accepts `{base_revision,document}`; the document identity must match the URL and supported JSON versions are 1/2. The result is `{revision,document}`. First save requires revision 0; later writes compare-and-swap the exact current revision inside an immediate transaction. HTTP 409 preserves both the saved revision and stale client's local document.

Workspace SQLite is application metadata adjacent to the project registry, not a project database. This schema has version 1, a 64-record cap, a 1,048,576-byte JSON limit, eight concurrent blocking requests and a five-second busy deadline. Invalid ID/document is 400, unknown workspace 404, stale revision 409, size/capacity 413, request capacity 429 and unsafe/unavailable storage 503. Authentication, Host, Origin and CSRF rules apply unchanged; PUT without CSRF is refused. Reads/restoration cannot authorize project operations. See [persistence](layout-persistence.md) for paths and recovery.

The P04 V1 adapter `/?project=<p_id>` selects a document-lifetime project prefix for existing project APIs, SSE and terminal WebSockets. `/api/auth/session` stays global. Without the query parameter, V1 keeps the immutable startup default. `/workspace.html` ignores this adapter and always passes project IDs explicitly. The existing terminal DELETE response is accepted even when HTTP 202 has an empty body; no server contract changed.

## P05 file and editor APIs

The project router now includes guarded file roots/tree/content, exclusive create, revision-aware move/delete/save, bounded search/cancel, change checks, read-only HEAD blobs, recoverable deletion/uncertain-write inspection and CAS draft storage. [Filesystem API](filesystem-api.md) is the authoritative endpoint/error model. All calls retain P03 project authentication/origin/CSRF and backend-owned root identities; a standalone legacy state without a project context cannot authorize a new file route; application legacy aliases remain bound to the immutable explicit server-default project. Workspace presentation schema version 3 adds file/document references without embedding source content.

## P06 IDE services

All paths below follow `/api/projects/<project_id>/ide`. The same default-project compatibility routing applies; the V2 client always supplies the project. Authentication, Host/Origin checks and mutation CSRF are unchanged. IDE JSON bodies are bounded to 2 MiB. Profiles/configurations use strict typed JSON and explicit trust confirmations. Local installation paths are accepted only when approving a profile; service launches select its stored ID.

| Method and suffix | Request | Response / behavior |
| --- | --- | --- |
| GET `/state` | none | Profiles, language servers, debugger sessions, terminals, configurations and installed extensions for this project |
| GET `/audit` | none | Last 100 bounded sensitive-operation attempts; no environment, transcript or source contents |
| POST `/profiles` | `{profile,confirmed:true}` | Validated approved profile with canonical executable and content fingerprint |
| POST `/profiles/:id` | `{}` | Remove profile if not referenced by an active service |
| POST `/configurations` | `{configuration,confirmed:true}` | Approved project/root/program debug configuration |
| POST `/lsp/start` | `{root_id,profile_id,client_id,confirmed:true}` | Actual initialized `ServerInfo`; reuses this project's compatible live instance |
| GET `/lsp/:id/events?cursor=N` | cursor | `{events,cursor,gap,alive}`; diagnostics filtered to authorized open documents |
| POST `/lsp/:id/sync` | `{document:{file,base_revision,version,text,language_id},saved}` | URI after protocol synchronization; save notification verifies disk content/revision |
| POST `/lsp/:id/request` | `{method,params,confirmed,request_id?}` | Allowlisted actual protocol result; rename requires confirmation |
| POST `/lsp/:id/cancel` | `{request_id}` | Cancels matching outstanding request; sends LSP cancellation |
| POST `/lsp/:id/prepare-edit` | `{edit}` | Provenance/version/root/revision-checked buffer edit plan; never writes disk |
| POST `/lsp/:id/resolve` | `{uri}` | P05-authorized `FileRef`; outside-root sources refused |
| POST `/lsp/:id/close` | `{file}` | Close this shared document, not the editor pane |
| POST `/lsp/:id/stop` | `{}` | Bounded graceful shutdown and owned process-tree termination |
| POST `/dap/start` | `{configuration_id,breakpoints,confirmed:true}` | Actual initialized/configured debugger session |
| GET `/dap/:id/events?cursor=N` | cursor | Bounded actual adapter events with cursor/gap/alive |
| POST `/dap/:id/request` | `{command,arguments,confirmed}` | Allowlisted debug request; evaluation needs explicit confirmation and adapter support; attach refused |
| POST `/dap/:id/breakpoints` | `{file,breakpoints:[{line,condition?}]}` | Adapter response; source paths resolved through P05 |
| POST `/dap/:id/stop` | `{}` | Disconnect with debuggee termination and supervised cleanup |
| POST `/terminals/start` | `{root_id,profile_id,confirmed:true,cols,rows}` | New development-terminal metadata; no numeric AgentSession is created |
| GET `/terminals/:id/socket` | WebSocket upgrade | Existing authenticated Origin-checked listener; metadata/snapshot then bounded binary PTY output; JSON `input`/`resize` messages |
| POST `/terminals/:id/rename` | `{name}` | Updated metadata |
| POST `/terminals/:id/stop` | `{}` | Explicit process termination; hiding a view does not call this |
| POST `/extensions/install` | `{manifest,trusted:true}` | Validated local declarative package installed disabled; SHA-256/provenance retained |
| POST `/extensions/:id/enable` or `/disable` | `{}` | Project activation state, with dependency/API/platform/integrity validation |
| POST `/extensions/:id/settings` | `{settings}` | Only declared bounded string preferences |
| POST `/extensions/:id/uninstall` | `{}` | Remove this project's installation; dependants must be removed first |

IDE errors are `{error,reason}` with a serializable domain variant. Invalid input/protocol is 400, unapproved/forbidden 403, unknown scoped resource 404, version/state conflict 409, request/process capacity 429, unsupported capability 501, deadline 504 and unavailable tool/process/storage 503. Existing authentication/JSON extractor errors retain their contracts. A successful profile approval is not proof that every language capability works; actual initialization/capabilities/events are authoritative.

Global protocol-call capacity is 16 with at most four ordinary blocking requests per project. Stop operations have a separate bounded four-request control lane so saturated tool requests cannot indefinitely prevent cleanup. Permits remain owned by the blocking operation even if the HTTP caller disconnects. See [service ownership](ide-service-architecture.md), [LSP](lsp-architecture.md), [DAP](dap-architecture.md), [terminal](development-terminal.md) and [extensions](extension-system.md).

## P07 conversations and missions

The following suffixes follow `/api/projects/<project_id>` and resolve the same authoritative context as other project operations. Authentication, Host/Origin validation and mutation CSRF remain mandatory. JSON bodies are limited to 128 KiB. IDs beginning `c_` identify conversations; numeric message/command/run IDs are local to the selected project. A workspace-session ID records navigation provenance and is not an authorization credential.

| Method and suffix | Contract |
| --- | --- |
| GET `/conversations?limit=100&cursor=<conversation_id>` | Creation-ordered page, newest first; the optional cursor must exist in this project |
| POST `/conversations` | `{title}` creates a durable conversation |
| GET `/conversations/status` | Actual configured Planner availability, agent and actionable configuration error |
| GET `/conversations/:id` | Metadata and revision |
| PATCH `/conversations/:id` | `{revision,title,status}` compare-and-swap; status is `active` or `archived`; archiving preserves history |
| GET `/conversations/:id/messages?limit=50&before=<message_id>` | Newest-first bounded history; message cursor, independent of conversation-list cursor |
| POST `/conversations/:id/messages` | `{key,workspace_session_id,content,context,mission?}` durably accepts a command and message; HTTP 202 is acceptance, not completion |
| GET `/conversations/:id/missions` | Linked real runs, tasks, sessions, attempts, artifacts and evidence snapshots |
| GET `/conversations/:id/events?after=<sequence>` | Authenticated durable SSE; `Last-Event-ID` is also supported |
| POST `/conversations/:id/commands/:command_id/cancel` | Request existing runtime cancellation; only commands in this conversation/project are eligible |

`mission` contains the supported form fields or an actual authored `Plan`. It is an explicit mission request, unlike ordinary chat, which uses a validated finite agent action. All three creation methods use one backend coordinator. An authored plan is validated and persisted unchanged; it is not replanned. Run association and creation are transactional. The command key and raw-request fingerprint reconcile repeated acceptance; changing a payload under the same key is a conflict. A completed/uncertain command is never automatically re-executed on reconnect.

Domain errors use `{error,code}`: invalid input 400 (`invalid_conversation_action`), forbidden context 403 (`context_forbidden`), unknown scoped resource 404 (`unknown_conversation`), stale metadata/command-state/key conflict 409 (`conversation_conflict`), runtime/configuration operation failure 409 (`conversation_operation`), capacity 429 (`conversation_limit`), and unavailable storage 503 (`conversation_storage`). Oversized HTTP bodies use Axum's 413; existing middleware and extraction errors retain their own contracts. There is no destructive history DELETE route: archive is supported, purge/export UI is deferred. See [conversation ownership](conversation-architecture.md), [mission creation](mission-creation.md) and [events](conversation-events.md) for exact limits, replay and failure semantics.

## P08 scheduler contracts

New V2 conversation missions snapshot P08 scheduler policy in the owning project SQLite (migration 20). Existing runs without that snapshot retain width one and no autonomous replans. The same Factory execution engine is used in both cases.

- `GET /api/projects/:projectId/runs/:runId/scheduler`: immutable effective policy, current durable owner/claim count, bounded recent scheduling events, integration journal and plan revisions.
- `PUT` on that route: validated pre-execution scheduler policy. Existing runtime/recipe width remains a cap. An owner or any attempt makes the policy immutable (409); invalid bounds return 400. This cannot change executable profiles, policies or tools.
- `GET .../scheduler/events?after=ID`: ascending durable event page, at most 50, with next cursor. IDs belong to the project database, not an application-global run namespace.

All routes use established authentication, Host/Origin checks, mutation CSRF and P03 project resolution. Claims require exact revision and an active durable owner. No browser control can grant process capabilities.

`deadlineSeconds` is the effective workflow-wide deadline: a positive integer for a persisted P08 policy or null for a legacy run. A legacy run retains V1's per-invocation timeouts and has no newly imposed workflow-wide deadline. Read this effective field rather than interpreting the legacy compatibility template's numeric duration as enforced. Configuration is read after durable owner acquisition, closing the pre-dispatch policy-update race.

Synchronous runtime failures classified as native agent resource exhaustion return 503 with the existing runtime error envelope and `AGENT_RESOURCE_EXHAUSTED` diagnostic prefix. An already accepted asynchronous conversation command keeps its 202 acknowledgment; subsequent persisted command/run/session state reports the blocker. A 202 never promises model availability or workflow completion. Capacity/claim conflicts are 409; invalid scheduler bounds are 400. Resource exhaustion does not trigger identical Worker retries or an automatic recovery Planner.

Task attempts add optional `evidence.gitSnapshot` (base/head, displayed patch SHA-256, explicit truncation). Agent output metadata adds optional process PID/start/finish and actual process-tree supervision state. Old JSON remains readable through defaults; no historical timestamp is backfilled. Process identities are for audit, not an unrestricted browser PID-control endpoint.

P05 roots also expose Rust-discovered integration snapshots as read-only, distinct project/root identities. The latest clean published candidate remains inspectable; older clean candidates may be pruned after later publication, and conflicts/uncommitted changes remain preserved. Neither saving nor development-shell launch is authorized on these roots.

## P09 profile and handoff contracts

All routes below are scoped under `/api/projects/:projectId/`. They use existing authentication, Host/Origin checks and mutation CSRF. Unscoped profile management is refused. Bodies are limited to 64 KiB; at most 16 profile API operations are active at once.

| Route | Contract |
| --- | --- |
| GET/POST `agent-profiles` | Bounded project profile list / create a stable profile referencing an existing trusted TOML installation. |
| GET `agent-profiles/providers` | Local installation availability, actual adapter capabilities and redacted legacy agent information. No credentials or invented model catalog. |
| GET/PATCH/DELETE `agent-profiles/:id` | Inspect / update with `expectedRevision` / archive with expected revision. Archival retains history. |
| GET `agent-profiles/:id/revisions?before=N` | At most 100 immutable revisions, exclusive revision cursor. |
| GET `runs/:id/agent-handoffs?before=ID` | At most 200 persisted handoffs, exclusive record cursor. |
| POST `runs/:run/tasks/:task/handoff` | Target profile, expected profile revision, reason and idempotency key. Returns durable state, not a promise that replacement already executed. |
| GET `agent-sessions/:id/execution-snapshot` | Immutable selected profile, model, instructions and effective policy; legacy sessions return no profile snapshot. |
| GET `agent-sessions/:id/model-observation` | Separate native model observation where available. It does not modify the selected configuration. |

Profiles use camelCase fields and reject unknown outer fields. They cannot supply executables, argv, environment values or authentication. Stale revisions and conflicting operations return 409; unsupported model/provider configuration returns 400; scoped unknown resources return 404; policy refusals return 403; capacity returns 429; provider/storage availability returns 503. Responses contain an explicit `code`. Known, scoped denied handoffs are retained as rejected audit records without launching a process. Identical retries reconcile the original command; changing the payload under its key conflicts.

Handoff errors separately identify `disabled_profile`, `unsupported_role`, `project_mismatch`, `unsafe_handoff_boundary`, `active_operation_cannot_be_interrupted` and `task_no_longer_eligible`. A claimed task awaiting its first attempt is an unsafe reassignment boundary, not an authorization grant or a code-quality failure. Existing `unknown_profile`, `unsupported_provider`, `unsupported_model_configuration`, `provider_unavailable`, `usage_limit_exceeded`, `permission_denied`, `capacity_exhausted`, `stale_profile_revision` and `handoff_already_pending` retain their distinct meanings.

See [profile ownership](agent-profile-architecture.md), [provider contracts](provider-model-configuration.md), [permissions](agent-permission-governance.md) and [handoff boundaries](agent-handoff.md). No P09 route interrupts an arbitrary PID, executes browser-provided commands, migrates model-internal state or enables P10 suspension.

## P10 browser presence and mission lifecycle

Authenticated application routes use the existing Host, Origin and CSRF middleware. A browser lease is presence evidence, never an authentication credential. Mutations also require the lease's acknowledged project/workspace binding.

| Method | Path | Contract |
| --- | --- | --- |
| POST | `/api/browser-clients/register` | `clientInstanceId: bc_<32 hex>`, bounded `bindings: [{projectId, sessionId}]`; returns random lease ID, generation, sequence and timing. |
| POST | `/api/browser-clients/:leaseId/heartbeat` | Same identity/generation, next sequence and full binding set. Exact duplicate sequence acknowledges without extending expiry. |
| POST | `/api/browser-clients/:leaseId/release` | Owned lease identity/generation; optional early release. Correctness uses server expiration. |
| GET | `/api/projects/:p/workspace-sessions/:s/lifecycle` | Durable close intent, generation, mission ownership and bounded session audit. |
| POST | `/api/projects/:p/workspace-sessions/:s/close` or `/reopen` | `expectedGeneration`, `idempotencyKey`; generation CAS. Reopen alone does not resume. |
| GET | `/api/projects/:p/runs/:r/lifecycle` | Orthogonal lifecycle plus bounded recorded process ownership; unmanaged legacy marker. |
| POST | `/api/projects/:p/runs/:r/suspend` or `/resume` | Expected generation and durable command key; current state returned on matching duplicate. Resume is explicit. |
| GET | `/api/projects/:p/runs/:r/checkpoints?before=N` | Up to 20 versioned checkpoints, descending ID cursor. |

Presence bodies are limited to 32 KiB, lifecycle bodies to 4 KiB, and at most 16 concurrent lifecycle/presence requests are admitted. Invalid identity/authentication is refused, stale transitions return a conflict, missing or inconsistent execution evidence cannot authorize resume. Lease defaults and limits are in [browser-presence.md](browser-presence.md); checkpoint limits and reconciliation are in [checkpoint-and-resume.md](checkpoint-and-resume.md).


## P11 project memory integration

P11 adds authenticated project-scoped `/memory` CRUD, revisions/provenance, search/retrieve, settings, candidate decisions, source resolution, explicit bounded extraction and invocation usage. See [memory architecture](project-memory-architecture.md). Mutations preserve CSRF/Host/Origin requirements; updates/deletion require the expected revision. A browser memory ID is not authorization.

All memory paths below are relative to `/api/projects/:p/memory`. Records and bodies use the camelCase DTOs in `factory-types::memory`; category/status/source-kind enum values use snake_case. Unknown write fields are rejected. Requests are limited to 64 KiB, sixteen concurrent memory operations, and two explicit extractors plus the application operation budget.

| Method | Suffix | Contract |
| --- | --- | --- |
| GET / POST | `/` | GET takes bounded `MemorySearch` query parameters; POST creates `{memory: MemoryInput, reason?}` without an expected revision. Creation acknowledges the stored record. |
| GET / PATCH / DELETE | `/:id` | PATCH requires `{memory, expectedRevision, reason?}`; DELETE requires `{expectedRevision}` and returns the explicit deletion/retention notice. Status changes use PATCH. |
| GET | `/:id/revisions?before=N`, `/:id/provenance` | Descending revision page (up to fifty) or current structured source references. |
| POST | `/search` | `MemorySearch`: `query`, optional category/status/sourceType/updatedAfter/before/limit. At most one hundred records; response reports cursor/search mode and bounded fallback limits. |
| POST | `/retrieve` | `{query}` returns a bounded current context preview under the authenticated user's source access. It does not call a model or record a fake invocation. Actual agent dispatch additionally applies the selected effective role/profile read policy and records usage. |
| GET / PUT | `/settings` | PUT the complete `MemorySettings` with its current `revision`. Stale revision conflicts. Extraction defaults disabled; retrieval and context/category budgets are separate controls. |
| POST | `/sources/resolve` | A typed `{projectId, reference}` source is independently resolved. Repository references use `{kind: "repository_file", rootId, path, revision?}`; protected/foreign sources are denied. |
| POST / GET | `/extract`, `/extractions` | POST `{sources: [...]}` makes one bounded authorized Planner invocation when enabled. The source-version claim is durable; an already recorded version returns an explicit notice without another model call. GET exposes the latest one hundred extraction records. |
| GET | `/candidates?before=N` | Up to fifty proposals and `nextCursor`; acceptance state is durable. |
| POST | `/candidates/:id/accept`, `/candidates/:id/reject` | Accept takes `{memory?: MemoryInput}` for an optional authorized edit; repeated existing decisions are idempotent. Rejection does not create a memory. Deletion removes related proposal aliases. |
| POST | `/:id/supersede` | `{expectedRevision, targetId, targetRevision}`; both current revisions are required before explicitly resolving related claims. |
| POST | `/index/rebuild` | Rebuild only derived FTS data from authoritative rows. |
| GET | `/usage?sessionId=N&before=N` | Bounded immutable invocation references: memory ID/revision/hash, settings/generation, role, session and context hash. Deleted content is not reconstructed. |

`MemoryInput` contains category, title, content, subject, component scope, lifecycle status and typed sources. A manual source is an explicit user assertion, not a fabricated repository citation. Repository provenance identifies the actual P05 document/content revision; an integration source must match an existing recorded local commit. Current source resolution is independent of a generated citation string.

Errors retain explicit codes: `invalid_memory`, `invalid_memory_source`, `sensitive_memory_content`, `memory_policy_denied`, `memory_not_found`, `stale_memory_revision`, `memory_duplicate`, `memory_limit`, `memory_busy_or_limit`, `usage_limit_exceeded`, `memory_provider_unavailable`, and storage/execution unavailability. Policy refusal does not trigger a repair loop or automatically alter settings. Authentication/Host/Origin/CSRF failures continue to use the established middleware contracts.


## P12 Source Control and review

All routes below are relative to `/api/projects/:p/source-control`. Existing authentication, Host/Origin checks and mutation CSRF apply. Registered P05 root IDs select authorized worktrees; they are not paths or credentials. Bodies are limited to 64 KiB, with eight application and four per-project concurrent filesystem operations. Errors distinguish invalid input, stale state, busy locks, required program trust, policy denial, missing evidence and unavailable/rate-limited GitHub.

| Method | Suffix | Contract |
| --- | --- | --- |
| GET | `/status?rootId=R` | Actual porcelain-v2 status, HEAD/index/generation and common identity. |
| POST | `/diff` | `{rootId,diff:{path,comparison,base?,target?}}`; working_index/index_head/working_head/commits. Complete version identity and supported server-generated hunks. |
| POST | `/index` | `{rootId,key,request:{expectedGeneration,paths,unstage,hunk?,diffVersion?,trustPrograms}}`; paths max256 (the UI selects at most128 changes, including rename pairs). Hunk is an opaque server identity, never an arbitrary browser patch. |
| POST | `/commit` | `{rootId,key,expectedGeneration,message,trustPrograms}`; actual staged index only, verified resulting hash. |
| GET / POST | `/branches` | GET takes rootId. POST `{rootId,key,expectedGeneration,name,create,trustPrograms,dirtyBuffers}`; safe create-and-switch/switch only. |
| GET | `/history`, `/commit-files`, `/worktrees`, `/conflicts` | History uses rootId/start?/skip/limit with max100; commit-files uses base/target. Conflicts read actual index stages and cannot choose a merge side. |
| GET | `/reviews?runId=N&before=N`, `/reviews/:attempt` | Metadata-only attempt pages100; exact captured evidence, session provenance and bounded integration details. |
| POST | `/reviews/:attempt/comments` | `{key,anchor:{base,head,path?,line?,side?},decision,body}`. Decision is comment/request_changes/approve_annotation; human feedback is distinct from automated review. |
| GET | `/operations?before=N` | Latest100 durable operation results. Uncertain commands are not blindly repeated. |
| GET | `/github?page=N`, `/github/pulls/:number?page=N` | Explicit authenticated reads; remote identity, PRs30/page and evidence100/page. No continuous provider polling. |
| GET / POST | `/delivery/:run` | Side-effect-free remote preview / explicit authorized publication through the existing coordinator. POST binds key, repository/name ID, expectedIntegration, base/head/title/body/draft and authorize:true. |
| POST | `/delivery/:run/update` | Only the run's recorded PR; key, authorize, repositoryId, expectedIntegration, expectedRemoteUpdatedAt, title/body/draft. Read-back reconciliation after uncertain results. |

See [Source Control](source-control-architecture.md), [staging](git-diff-and-staging.md) and [delivery](github-delivery.md). A local operation journal is not an atomic Git/SQLite transaction. Live external publication qualification is separate from controlled provider-contract tests.


## P13 preferences and notifications

- `GET/PUT /api/application/preferences`: application-owned `{revision,preferences}`; Rust validation, 4 KiB bound, acknowledged commit and stale-revision 409. Presentation preferences cannot modify execution policy.
- `GET /api/projects/:id/settings`: read-only safe projection of current project configuration/revision; no environment values, provider command arguments or credentials. Existing configuration/agent/tool/memory endpoints remain the mutation authorities.
- `GET /api/projects/:id/notifications?before=N&limit=N&archived=bool`: actual project events, unread count and bounded keyset page (maximum100).
- `PATCH /api/projects/:id/notifications/:id`: `{revision,read,dismissed}`; conditional acknowledgement, repeated identical desired state idempotent, conflicting stale intent409. There is no browser notification-creation route.

All use existing authentication and Host/Origin validation; mutations require CSRF. IDs and deep links never grant access. See [settings architecture](settings-architecture.md), [notifications](notifications.md) and the P13 delivery for exact executed evidence.
