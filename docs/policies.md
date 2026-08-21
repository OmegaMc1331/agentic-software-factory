# Policies (permission engine)

Factory policies are **project-local orchestration controls** that decide what each
role and agent is *permitted* to do during automated workflows. They are not a
sandbox: Factory does not use OS-level virtualization, and external coding agents
spawn their own descendants. Where Factory cannot technically enforce something, this
document says so explicitly.

One distinction runs through the whole model:

```text
Role instructions = what the agent is asked to do
Policy            = what Factory permits it to do
```

Instructions never grant permissions. A mission may *state* the policy boundary, but
enforcement happens in Factory Core — before launch, and against recorded evidence.

## Model and precedence

Policies live in `.factory/config.toml` under `[policies.roles.<id>]` and
`[policies.agents.<name>]` and are resolved by one centralized resolver in
`factory-policy`:

```text
Factory safety invariants          (always applied, cannot be removed)
        ↓
Role policy                        ([policies.roles.<id>])
        ↓
Agent-specific restrictions        ([policies.agents.<name>])
        ↓
EffectivePolicy                    (one per running role/agent pair)
```

Rules of the merge:

- an agent scope may only **further restrict** the role scope — allow lists
  intersect, deny lists union; an agent can never widen a role's access;
- deny always wins over allow, in every dimension;
- the Factory safety invariants are re-applied last, so no configuration can
  bypass them:
  - `.factory/**` and `.git/**` are never writable;
  - dangerous Git operations (push, force push, branch deletion, branch reset,
    remote modification) are never permitted to task agents;
  - Factory-owned integration branches stay under the Integration Engine's
    exclusive control.

The same `EffectivePolicy` is used everywhere: automated execution, validation,
`AgentSession` audit metadata, and the dashboard's Permissions sections. Permission
logic is never re-implemented in the runtime, API, or frontend.

## Dimensions

### Filesystem

```toml
[policies.roles.worker.filesystem]
read = ["**"]
write = ["src/**", "tests/**"]
deny_write = [".factory/**", ".github/**"]

[policies.roles.documentation_writer.filesystem]
read = ["**"]
write = ["README.md", "docs/**"]

[policies.roles.security_auditor.filesystem]
read = ["**"]
write = []
```

- Paths are **repository-relative globs**: `**` spans directories, `*` stays within
  one component, `?` matches one character. Matching is case-insensitive on Windows.
- A declared `filesystem` table is complete: absent or empty `write` means *no
  writes at all* (a read-only role); absent `read` means nothing may be read.
- `deny_write` always wins over `write`, and the baseline denial of `.factory/**`
  and `.git/**` applies even when a scope says `write = ["**"]`.
- Paths that cannot be mapped into the repository — absolute paths, drive letters,
  `..` traversal, control characters — are treated as outside every scope, so they
  can never match.

What Factory enforces here is **evidence-scoped**: a task is blocked before it
starts when its operation needs writes the role does not have, and an attempt fails
with a policy violation when its recorded changed files fall outside the effective
write scopes. Factory does not intercept individual filesystem calls made by the
external agent process or its descendants; a fully hostile agent can still touch
files as your OS user. The violation is detected and the attempt is failed —
deterministically, without consuming a retry.

### Commands

```toml
[policies.roles.worker.commands]
mode = "restricted"
allow = ["cargo", "npm", "pnpm", "git"]
deny = ["powershell", "cmd", "bash"]
```

Three modes: `unrestricted`, `restricted`, and `denied`. Matching is deliberately
restrained: Factory compares the **executable name** (the first whitespace-delimited
token, case-insensitive) against the allow/deny lists. `git commit -m ...` matches
`git`. Dangerous Git subcommands (`push`, `reset`, `remote`, `branch -d`) are denied
even when `git` is allowed.

Enforcement applies to the commands an agent **reports** in its evidence. Factory
cannot parse or intercept the full command line of everything an external CLI and
its descendants execute; the policy gates mission guidance (the agent is told the
boundary) and fails attempts whose reported commands violate it. Factory itself
never offers a generic shell endpoint — that invariant is independent of policies.

### Git

Normal task Git operations — reading the repository and committing inside the task
worktree — are allowed by default:

```toml
[policies.roles.worker.git]
allow = ["read", "commit_in_task_worktree"]
```

`push`, `force_push`, `delete_branch`, `reset_branch`, and `modify_remotes` are
Factory safety invariants: declaring them in `allow` has no effect. The Integration
Engine remains the only component that advances Factory-owned integration branches;
task agents cannot bypass it.

### Network

```toml
[policies.roles.researcher.network]
mode = "deny"   # or "allow"
```

Network policy is **advisory** on every current platform. Factory has no reliable
mechanism to restrict the network of an arbitrary launched process on Windows,
macOS, or Linux, so `deny` means: the boundary is stated in the mission, recorded in
the session audit, and shown in the dashboard — never claimed as isolation. The
`networkEnforcement` field of every policy view reads `advisory`. Do not treat
`mode = "deny"` as a sandbox.

### Environment and secrets

```toml
[policies.roles.worker.environment]
allow = ["PATH", "HOME", "USERPROFILE", "RUST_BACKTRACE"]
deny = ["AWS_SECRET_ACCESS_KEY", "GITHUB_TOKEN"]
```

When a policy filters or denies variables, Factory **replaces** the child process
environment instead of letting the agent inherit Factory's whole environment: the
launched process receives exactly the computed variables. Variables configured on
the agent entry still apply, minus anything denied. Deny wins over allow, and keys
compare case-insensitively.

Denied values are treated as secrets: their values are registered for redaction and
replaced with `[REDACTED]` if they ever appear in captured session output. Secret
values are never written to the database, the audit record, or logs.

## Resolution in practice

```text
Task → role → agent → EffectivePolicy
```

- Before a workflow starts (and before every retry), each task's effective policy
  is validated. A task that cannot legally execute blocks the workflow with a
  useful reason — for example *“Documentation Writer cannot perform operation
  'implement': no writable filesystem scope (allowed writes: README.md, docs/**)”* —
  and the run is marked `blocked`. **Policy blocks never consume task retries**;
  fixing `.factory/config.toml` and starting again resumes with the attempt budget
  intact.
- Agent selection is policy-aware: within a role's agent pool, Factory prefers the
  capacity-aware choice but skips agents whose own restrictions would block the
  task.
- After each attempt, recorded evidence is checked against the same effective
  policy. A violation fails the attempt (marked `blocked by policy`) without a
  retry.

## Presets and defaults

Core roles can be configured with compact presets instead of full tables:

| Preset           | Shape                                                        |
| ---------------- | ------------------------------------------------------------ |
| `read_only`      | Full read, no writes, git + restricted commands              |
| `implementation` | Worktree-wide writes (minus invariants), restricted commands |
| `documentation`  | Writes limited to `README.md` and `docs/**`                  |
| `review`         | Read-only                                                    |
| `custom`         | No preset defaults; explicit dimensions still apply          |

New projects created by `factory init` ship presets for the core roles — Planner,
Architect, Researcher, Reviewer, and Security Auditor are `read_only`; Worker and
Test Engineer are `implementation`; the Documentation Writer is `documentation`.

**Existing projects keep working unchanged.** A configuration without `[policies]`
resolves to the legacy permissive defaults (open filesystem, unrestricted commands,
full environment inheritance, advisory-allow network), visibly marked as
*permissive* in the dashboard. Nothing is silently restricted on upgrade. The Git
safety invariants apply regardless of configuration.

Custom roles select a preset at creation (Agent Graph role form) or later (Role
Inspector → Policy preset, or `PUT /api/roles/:id/policy` with
`{"preset": "read_only" | "implementation" | "documentation" | "review" | "custom" | null}`).

## Audit

Every automated `AgentSession` persists a compact policy snapshot — policy source
(`role:worker`, `role:worker + agent:codex`, or `default`), filesystem mode
(`open` / `restricted` / `read_only`), network mode, environment mode, and the
effective write scopes. It contains no secret values.

## Dashboard

The Role Inspector and the Agent Inspector each show a **Permissions** section with
the effective policy: filesystem mode and scopes, command policy, network (marked
advisory), environment mode, and Git permissions. The Role Inspector also edits the
role's policy preset. Legacy permissive configurations are flagged visibly.

## Enforcement boundary — what is and is not enforced

Enforced by Factory:

- starting tasks whose role/agent policy cannot legally execute them;
- failing attempts whose **recorded evidence** (changed files, reported commands)
  violates the effective policy, without consuming retries;
- computing and installing the child process environment (allow/deny lists)
  before launch;
- redacting denied secret values from captured output;
- the safety invariants: Factory state (`.factory/**`, `.git/**`) stays unwritable,
  dangerous Git operations stay denied, integration branches stay under the
  Integration Engine.

Not enforced (documented limitations):

- individual filesystem calls by the external agent process or its descendants —
  agents run with your OS permissions; worktrees isolate work, they are not
  sandboxes;
- network access of launched processes — always advisory;
- command lines beyond the reported-evidence check — Factory is not a shell
  parser and does not intercept process trees;
- OS-level virtualization of any kind (containers, VMs, seccomp, AppArmor,
  Windows Sandbox) — out of scope for this milestone.

In short: policies make Factory a disciplined orchestrator with hard gates where it
controls the boundary, not a sandbox. Run agents you trust.
