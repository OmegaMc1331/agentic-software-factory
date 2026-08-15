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
Depending on the agent, they may access files outside the project, inherited
environment variables, the network, and other user-readable locations. Use only agents
you trust.

Factory doesn't manage model-provider credentials. Agents are external CLIs that you
install and authenticate. They inherit Factory's environment, plus any variables in
their `.factory/config.toml` entry.

PTY sessions run with the same OS permissions as Factory. A PTY is an interaction
mechanism, not a sandbox.

## Git worktrees

Each task runs in `.factory/worktrees/t<task-id>` on its own Git branch. Worktrees
separate task files and branches; they are not security sandboxes. An agent can still
access anything permitted to the invoking OS user.

Cancellation stops scheduling and terminates the current configured agent process tree
when possible. Factory preserves the worktree, session output, and evidence instead of
deleting partial work.

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
