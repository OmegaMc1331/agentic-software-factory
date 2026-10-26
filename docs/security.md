# Security

Agentic Software Factory is a local developer tool. `factory start` binds only to
`127.0.0.1` and requires an authenticated loopback browser session for its API. Do
not proxy or expose it to another machine.

## Local API authentication

Every `/api` route except `GET /api/health` and the session bootstrap route requires
a short-lived browser session. The dashboard establishes it automatically; users do
not enter a password.

**Host and Origin.** The server rejects requests whose `Host` header is not exactly
`127.0.0.1`, `localhost`, or `::1` (with an optional port). A present `Origin` must
exactly match the request's scheme, host, and port, or one explicitly configured
development origin. Wildcards, suffix matches, `localhost.evil.example`, and
non-loopback origins are rejected. This blocks DNS-rebinding and cross-origin browser
requests before they reach any API, SSE, or WebSocket handler.

**Session.** At process start Factory generates a random 32-byte secret in memory and
sets a `factory_session` cookie for `POST /api/auth/session`, issued only for a
same-origin `Origin` and an `application/json` request. The cookie is `HttpOnly`,
`SameSite=Strict`, scoped to `/api`, and defaults to a 12-hour lifetime. Its value is
a signed, expiring token; restarting Factory invalidates every session. No session
secret is written to disk, logged, or placed in a URL or local storage.

**CSRF.** Bootstrap also sets the readable `factory_csrf` cookie and returns the same
value as `csrfToken`. The dashboard sends it in `X-Factory-CSRF` on POST, PUT, PATCH,
and DELETE. The backend requires the header and cookie to match. The dashboard keeps
the token in memory only.

**Streaming transports.** SSE (`/api/sessions/:id/stream`) and the interactive
terminal WebSocket (`/api/sessions/:id/terminal`) use the same `factory_session`
cookie. Authentication runs before the WebSocket upgrade, so an unauthenticated or
cross-origin handshake is rejected with HTTP `401` or `403` and never creates a PTY
connection. The dashboard refreshes the session before opening them.

**Development.** Vite proxies `/api` to the backend and forwards WebSockets. Because
the browser origin is then `http://localhost:5173`, start the backend with
`--dev-origin http://localhost:5173` (or `FACTORY_DEV_ORIGIN`). That exact origin is
the only addition; no production route trusts arbitrary localhost ports.

**Limits.** This is browser boundary protection, not user authentication. A local
process running as the same user, or any person with access to the machine and its
loopback interface, can request a session and is equivalent to a trusted local
operator. Keep the server bound to loopback and do not expose it through a proxy,
tunnel, or port forward.

## Workflow operations

Agent Graph can request only explicit Factory operations: create a workflow, start or
cancel it, and retry an eligible task. The backend resolves every role from validated
`.factory/config.toml` role assignments; workflow execution only invokes agents
assigned to a role on that workflow's team. Workflow endpoints cannot provide an
executable, select an arbitrary process, or submit a shell command.

The configuration and role APIs can change agent definitions and role assignments, so
access to the local dashboard is equivalent to access to this project's agent
configuration. Only configured agents are invoked by workflow operations.

## Role instructions

Custom role instructions are project-controlled text that Factory incorporates into
the mission sent to a coding agent. They change what the agent is asked to do; they
do not change what it is allowed to do:

- role instructions do not change OS permissions — agents still run as the invoking
  user;
- roles cannot bypass Factory's API boundary or its execution constraints
  (worktree isolation, `.factory` state protection), which always take precedence;
- creating a role does not create a new executable capability — roles select among
  already-configured agents and only alter mission text;
- custom role text is not a security sandbox; treat it like any other prompt to an
  agent you trust.

Permissions come from the **policy engine**, not from instructions. See
[Policies](policies.md) for the model; the security-relevant summary:

- policies are project-local orchestration controls, not OS-level virtualization;
  Factory does not sandbox the agent process, its filesystem calls, or its network;
- the Factory safety invariants always apply regardless of configuration:
  `.factory/**` and `.git/**` stay unwritable, dangerous Git operations (push,
  force push, branch deletion, reset, remote modification) stay denied, and the
  Integration Engine keeps exclusive control of integration branches;
- network `deny` is advisory everywhere — Factory records and states the boundary
  but cannot restrict a launched process's network on current platforms;
- environment policies filter the child process environment before launch (allow
  lists, deny lists, deny-wins), and denied values are redacted from captured
  session output; secret values are never logged or persisted. Redaction is
  **streaming**: each output stream keeps an independent stateful redactor that
  holds back the bounded suffix a future chunk could complete, so a secret split
  across process write boundaries never reaches the incremental database rows,
  the session event stream, or the final stored output. When a stream ends
  mid-secret, the unterminated prefix is flushed verbatim — it is not the secret
  value and can no longer be completed. Secret values longer than 1 MiB (far
  beyond real credential sizes) are matched within single chunks and in final
  whole-text redaction, but their cross-chunk streaming match is not guaranteed.
  Factory only redacts secrets it knows: policy-denied environment values,
  agent-configured environment values, and MCP grant tokens. It does not detect
  arbitrary secrets, or secrets that arrive encoded or transformed;
- tasks that cannot legally execute are blocked before an agent process starts, and
  policy violations fail an attempt without consuming the normal retry budget.

## Agent Console

The Agent Console is not a general shell. It reads a known Factory-managed
`AgentSession`. Workflow sessions expose output through scoped SSE. Interactive
sessions launch only the selected configured agent in a PTY and use a WebSocket scoped
to that live session ID. The API cannot spawn `cmd.exe`, PowerShell, Bash, or another
process selected by the browser.

Terminal input and resize are accepted only for a Factory-owned interactive session.
Automated Planner, Worker, and Reviewer sessions remain non-interactive. Arbitrary
command execution through the dashboard API is not supported.

Interactive terminal subscriptions are independently bounded and never pause the PTY.
A slow WebSocket is removed with close code `1013`; its queued bytes are released while
healthy subscribers and session persistence continue. The retained reconnect snapshot
is capped at 1,000,000 bytes, and each subscriber queue is capped at 2,097,152 bytes /
256 chunks with at most 8 subscribers per terminal. See
[Architecture](architecture.md#interactive-terminal-backpressure).

## External tools / MCP

MCP server configuration does not grant access. A role must explicitly allow a
qualified tool, agent and Recipe policy can only narrow that allow list, every deny
wins, and write/destructive/unknown tools require an exact role allow. Tool metadata
and output never participate in authorization. See [External Tools / MCP](mcp.md).

Factory starts stdio servers with structured argv and a repository-bounded working
directory; HTTP endpoints must be absolute and cannot embed credentials. Server env
and headers accept only `${ENV_NAME}` references. Resolved values are redacted and
are never persisted or sent in missions. Scoped task grants are stored hashed,
expire, are revoked after the session, and cannot call tools outside their catalog.

Schemas are validated, discovery and calls have timeouts, raw responses have hard
size limits, and returned text is visibly marked untrusted external data. The local
API exposes semantic configuration/test/inspection operations only—never raw MCP
JSON-RPC, an arbitrary executable, or unrestricted dashboard tool invocation.

These controls do not make MCP an OS sandbox. Configure only trusted server processes
and URLs, and do not separately give coding CLIs unmanaged native MCP credentials if
Factory is intended to be the authorization boundary.

## Process permissions

External coding agents run with the OS permissions of the user who started Factory.
Depending on the agent, they may access files outside the project, the network, and
other user-readable locations. Use only agents you trust.

Factory doesn't manage model-provider credentials. Agents are external CLIs that you
install and authenticate. By default they inherit Factory's environment, plus any
variables in their `.factory/config.toml` entry; when a role or agent policy filters
or denies environment variables, Factory replaces the child environment with the
computed set instead of inheriting, and denied values are redacted from recorded
output. See [Policies — environment](policies.md#environment-and-secrets).

PTY sessions run with the same OS permissions as Factory. A PTY is an interaction
mechanism, not a sandbox.

## Role-aware execution

Each task runs in `.factory/worktrees/t<task-id>` on its own Git branch. Worktrees
separate task files and branches; they are not security sandboxes. An agent can still
access anything permitted to the invoking OS user.

The runtime dispatches every task by its **operation** (`advisory`, `implement`,
`verify`, `review`, `post_process`), which is validated against the role's execution
class before the workflow starts. Specialized review tasks receive a **diff snapshot**
bounded to 60 kB and run in their own worktree rather than sharing the implementation
worktree. They evaluate evidence; they cannot modify the implementation's files
unless a plan explicitly allows it.

Cancellation stops scheduling and terminates the current configured agent process tree
when possible. Factory preserves the worktree, session output, and evidence instead of
deleting partial work.

## GitHub integration

The GitHub milestone (Issue import, delivery push, pull request creation) keeps the
trust boundaries explicit. See [GitHub](github.md) for the user-facing flow.

**Untrusted Issue content.** GitHub Issue titles, bodies, and comments are external
untrusted text. They enter Factory as bounded, verbatim *data*: the run's objective
and a persisted link. Every mission that includes them carries an explicit notice
that the content is requirements/context, never instructions, and cannot change
roles, permissions, policies, repository boundaries, or output contracts. Issue text
never reaches a shell, a system prompt position, or a permission decision.

**Authentication.** Factory uses the locally installed, locally authenticated `gh`
CLI. It checks `gh auth status` and shows the connected account; it never reads,
stores, logs, or displays GitHub tokens, and it has no OAuth server, GitHub App,
token storage, or cloud auth backend.

**Delivery vs agents.** The Factory Delivery Engine is the only code in Factory that
constructs a `git push`, and it pushes exactly the Factory-generated
`factory/run-<id>` branch — never a force-push, never arbitrary user branches, and
only after the user confirms a PR preview. Task agents keep their normal policies:
push-class Git operations stay denied regardless of configuration or imported Issue
content, and no role instruction can trigger `git push` or `gh pr create` through
Factory. See [Policies — agent vs delivery permissions](policies.md#git).

**Injection resistance.** All `git`/`gh` invocations use structured process
arguments — no `sh -c`, no `cmd /c`, no string-interpolated commands. PR titles
travel as single argv values; PR bodies travel through an exclusive-create
temporary file consumed by `gh pr create --body-file` (removed on success,
failure, and timeout), never through argv, and bodies larger than 262,144
characters are rejected up front. Branch names are always Factory-generated
(`factory/run-<id>`); repository slugs are parsed with a strict `owner/name`
charset and cross-repository issue imports are refused. The API exposes only
semantic operations (`from-issue`, `delivery`, `pr-preview`, `pull-request`) —
there is no generic GitHub command endpoint.

**Repository-boundary safety.** The Context Engine treats symbolic links
(including Windows junctions) as opaque names, never as files or directories:
link targets outside the repository are neither traversed nor read, link
cycles cannot loop the walk, and external tools' path output is revalidated
against the canonical repository root before use. This bounds *Factory-managed
context*; it is not an OS sandbox — a separately launched coding agent retains
its own filesystem access.

Candidate validation rejects explicit `.`/`..` components, relative tool output,
non-file targets, symlinks in every component, and Windows reparse points.
Canonical names must pass the same ignore rules as the supplied names.
Repository content reads repeat eligibility checks at opening, reject final
links through platform no-follow options, check the opened handle, and share a
1 MiB reader that rejects overflow instead of treating a truncated prefix as
complete content. This also applies to index fingerprints, link expansion and
resolution excerpts. It does not provide fully race-proof filesystem isolation:
parent directories are not traversed through pinned OS handles, so malicious
concurrent replacements remain outside the guarantee. Static traversal and
intermediate-link attacks are covered by deterministic regression fixtures.

**Bounded subprocess supervision.** Every Factory-owned `git`/`gh` command runs
under a shared supervisor with an explicit per-operation deadline, concurrent
stdout/stderr draining, hard output-capture limits, a null stdin, and
process-tree termination when the deadline fires (`taskkill /T` on Windows, the
child's own process group on Unix). Git and gh invocations set
`GIT_TERMINAL_PROMPT=0` so missing credentials fail instead of waiting for
terminal input; credential helpers and keyring flows keep working. Deadlines
default per operation class (30–180 s, 120 s for local git) and can be scaled
with `FACTORY_GIT_TIMEOUT_SECS` / `FACTORY_GH_TIMEOUT_SECS`. A timeout after an
external side effect — `git push` or `gh pr create` — is classified as an
**uncertain** outcome: the delivery is marked `interrupted` and requires
reconciliation before any retry; a timeout never triggers an automatic second
push or pull-request creation. JSON responses are never parsed from truncated
output; a capture that exceeds its limit is an explicit error.

### Custom and specialized role boundaries

Role definitions — including custom and specialized review roles — are prompt
context, **not authorization**. A role changes what the agent is asked to do; it does
not change what the agent is allowed to do:

- custom roles do not gain shell capabilities beyond the configured agent — they
  select among already-configured agents and only alter mission text;
- custom roles do not bypass worktree isolation or `.factory` state protection;
- custom roles do not gain provider or network capabilities the agent CLI does not
  already have (Factory never claims web research on behalf of an agent);
- review roles cannot execute arbitrary API-side shell commands — the workflow
  runtime owns task execution and the dashboard cannot invoke agent processes
  directly;
- agents still inherit the user's OS permissions; a mission cannot elevate them.

## Local files

SQLite lives at `.factory/db.sqlite3` with the process default permissions. SQL writes
are parameterized and schema changes use versioned migrations. Automated stdout and
stderr are drained concurrently, captured with a 1 MB per-stream head/tail bound, and
stored with truncation metadata; oversized structured output is rejected before parsing.
Session logs may still contain agent-produced sensitive content. Secret values are
redacted before persistence, including across streaming chunk boundaries; see
[Agent Console and redaction](#process-permissions) above.

`PUT /api/config` and `PUT /api/graph/workspace` validate their bodies before atomic
temp-file-and-rename writes. The graph workspace contains visual metadata only.

## Installers

`install.ps1` and `install.sh` download release archives from GitHub over HTTPS and
verify their published SHA-256 checksums. Installation is user-local and doesn't
require administrator rights. The installers don't run the downloaded binary or store
credentials.
