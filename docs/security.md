# Security

Agentic Software Factory is a local developer tool. `factory start` binds only to
`127.0.0.1`; the API has no authentication, so do not proxy or expose it to another
machine.

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
  session output; secret values are never logged or persisted;
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
arguments — no `sh -c`, no `cmd /c`, no string-interpolated commands. PR titles and
bodies travel as single argv values and as JSON in the API; branch names are always
Factory-generated (`factory/run-<id>`); repository slugs are parsed with a strict
`owner/name` charset and cross-repository issue imports are refused. The API exposes
only semantic operations (`from-issue`, `delivery`, `pr-preview`, `pull-request`) —
there is no generic GitHub command endpoint.

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
are parameterized and schema changes use versioned migrations. Session stdout and
stderr are bounded in the database but may contain agent-produced sensitive content.

`PUT /api/config` and `PUT /api/graph/workspace` validate their bodies before atomic
temp-file-and-rename writes. The graph workspace contains visual metadata only.

## Installers

`install.ps1` and `install.sh` download release archives from GitHub over HTTPS and
verify their published SHA-256 checksums. Installation is user-local and doesn't
require administrator rights. The installers don't run the downloaded binary or store
credentials.
