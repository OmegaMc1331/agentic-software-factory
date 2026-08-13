# Security

The factory is a local developer tool with no networked services of its own. Its
security surface is small, and one boundary deserves an explicit warning.

## Git worktrees are not a security sandbox

Each task runs in its own git worktree. That is isolation for branches and concurrent
work — it keeps commits and files separate from the main tree. It is **not** a security
boundary.

Coding agents run as normal subprocesses with the permissions of the user running
`factory`. Depending on the agent, they may access:

- files available to that user, including outside the worktree;
- inherited environment variables;
- the network;
- other locations on the machine.

Do not run untrusted agents, and treat agents as having the same access as a terminal
you open yourself.

## Credentials

The factory does not manage model-provider credentials. It has no API keys and never
talks to a model provider. Agents are external CLIs that you install and authenticate
yourself; they handle their own authentication.

Two practical notes:

- Agent subprocesses inherit the factory's environment.
- `.factory/config.toml` can define extra environment variables for an agent; anything
  you put there is visible to that agent's process.

## The local API

`factory start` binds `127.0.0.1` only. The API has no authentication; do not expose
the port beyond a trusted machine.

The write surface is one endpoint: `PUT /api/config`, which replaces
`.factory/config.toml`. The body is validated (agent names, commands, role references,
environment keys) before anything is written, and the write is atomic (temp file +
rename). There is no endpoint that executes shell commands.

## Database

SQLite lives at `.factory/db.sqlite3` with the process default permissions. All SQL is
parameterized; schema changes come only from versioned migrations applied at open.

## Installers

`install.ps1` and `install.sh` download prebuilt binaries from GitHub Releases over
HTTPS. They never run the downloaded binary during installation. Each archive ships
with a published SHA-256 checksum; the installer verifies the download against it and
aborts on any mismatch, so the served binary is trusted only as far as GitHub itself is.
Installation is user-local (under `%LOCALAPPDATA%` or `$HOME/.local/bin`) and requires
no administrator rights. The installers do not store credentials.