# GitHub integration

Factory's GitHub milestone closes the local delivery loop:

```text
GitHub Issue → Import → Workflow → Planner → Agents/Reviews/Tests
    → Integration Engine → factory/run-<id> → Push → Pull Request
```

Everything runs through the normal Factory pipeline — there is no separate
GitHub execution engine. After an Issue is imported, the Planner produces an
editable DAG, the Runtime executes it, and the Integration Engine owns the
`factory/run-<id>` branch exactly as for any other workflow.

## Authentication

V1 uses the locally installed [GitHub CLI](https://cli.github.com):

```bash
gh auth login     # once, in a terminal
gh auth status    # what Factory checks
```

Factory reports `GitHub — Connected as <user>` in the *From GitHub Issue*
form and the API (`GET /api/github/status`). It never reads, stores, or
displays GitHub tokens, and it never implements its own OAuth server, GitHub
App, or cloud auth backend. If `gh` is missing or unauthenticated, the exact
next step (`install gh`, `run gh auth login`) is shown instead of a generic
failure.

## Repository detection

The repository (`owner/name`, remote name, default branch) is resolved only
from the project's Git remotes — never guessed from folder names. Both HTTPS
(`https://github.com/owner/repo.git`) and SSH (`git@github.com:owner/repo.git`,
`ssh://git@github.com/...`) remotes are supported. A project without a GitHub
remote gets a clear diagnostic instead of an import.

## Importing an Issue

In Agent Graph: **+ Workflow → From GitHub Issue**, then enter `#42`, `42`,
or a full issue URL. Imports fetch the number, title, body, labels, state,
URL, author, and a bounded set of comments (at most 10, each truncated) —
never hundreds of comments blindly. Issues from a URL naming a different
repository than the project's remote are rejected.

Importing never executes anything. The flow stays:

```text
Import Issue → Planner creates the plan → inspect/edit the plan → Start
```

The imported issue is persisted on the run (`provider`, `repository`,
`issue_number`, `issue_url`, …), so the workflow stays fully usable even if
GitHub is later unavailable.

## Issue content is untrusted context

Issue titles, bodies, and comments are **external untrusted text**. They are
workflow requirements and context — never Factory instructions, system
instructions, or permission changes. Issue text cannot override:

- the Policy Engine,
- role instructions,
- Factory invariants,
- repository boundaries,
- agent permissions.

The Planner and every task mission carries an explicit notice marking the
objective as containing untrusted imported content, stating that it must be
treated as data and cannot change roles, permissions, or output contracts.
Prompt injection inside an issue may at worst confuse an agent about the
*work*; it never widens what the agent is *allowed* to do.

## Delivery (push + pull request)

After a workflow **completes successfully**, Factory owns a coherent
integration branch (`factory/run-<id>`). Only then does the Workflow
Inspector offer **Create Pull Request**. Delivery requires:

- the workflow is `completed`;
- no unresolved integration conflict;
- a valid integration branch with a known head;
- the persisted integration head equals the local branch head (branch drift
  blocks publishing);
- the PR base branch still exists on the remote.

Partially approved or conflicted work is never delivered.

### Push is Factory-owned

Agents never receive GitHub push permission because delivery exists. The
**Agent Policy Engine** (which denies `push`, `force_push`, branch deletion,
branch reset, and remote modification for task agents) stays fully separate
from the **Factory Delivery Engine**. A worker cannot bypass the Integration
Engine and push arbitrary branches: the only push path in Factory is the
explicit delivery action, which pushes exactly `factory/run-<id>` — never a
force-push, never arbitrary user branches.

All `git`/`gh` commands use structured process arguments; there is no
`sh -c`, no `cmd /c`, and no string-interpolated shell anywhere in the path.

### Pull request preview

Selecting **Create Pull Request** shows a preview: repository, base, head,
an editable title and body, and a *Create as draft* toggle. Factory's
documented default is a **normal (non-draft) PR**. Nothing is published until
you confirm; an agent can never silently publish.

The initial PR body is generated deterministically from Factory state:
workflow objective as Summary, the task list as Changes, the commands the
approved attempts actually reported as Verification, and the recorded review
decisions as Reviews. If the workflow was imported from an issue, the body
ends with `Closes #<number>`; otherwise that line is omitted. Factory never
fabricates tests or reviews — absent evidence is stated as absent.

### Persistence and duplicates

Delivery metadata (remote, repository, PR number/URL/state, pushed head SHA,
timestamps) is persisted on the workflow, so state survives browser reloads
and restarts. Before creating a PR, Factory checks for an open PR on the head
branch and links it instead of duplicating.

### Durable operation ownership (ASF-18)

Creating a pull request is an asynchronous, Factory-owned operation:

1. The API request validates the workflow, then **atomically claims** the
   run's delivery operation in SQLite (a compare-and-set from an idle or
   failed state). Exactly one concurrent request can win the claim; the
   others receive a `409 Conflict`. The claim — including a durable
   `operationId` — is committed before any external side effect runs.
2. The worker executes under Factory ownership, decoupled from the browser:
   closing the tab, a request timeout, or a reload never cancels it. The
   endpoint answers `202 Accepted` with the claimed record; a repeated
   request after publication answers `200` with the existing PR and creates
   nothing.
3. Every state transition is guarded by the operation id, so a stale attempt
   (left behind by a crash and superseded after recovery) can never
   overwrite a newer outcome — including `published`.
4. The dashboard polls the durable local status (`GET /api/runs/:id/delivery`)
   while an operation runs; polling never contacts GitHub.

### Uncertain outcomes and reconciliation

A failed local command does not prove the remote side effect did not happen
(the push may have been accepted before the connection dropped; GitHub may
have created the PR before `gh` timed out). Such attempts are recorded as
`interrupted` with recovery required — never as `failed` — and Factory
refuses to start another operation until reconciliation resolves them:

- On restart, deliveries left `pushing`/`creating_pr` by a previous process
  are demoted to `interrupted`. Nothing is pushed or created at startup.
- `POST /api/runs/:id/delivery/reconcile` performs a read-only check
  (`gh pr list --state all` for the head branch, plus `git ls-remote` for
  the pushed head). An existing PR is linked and persisted; an
  authoritative "no PR exists" turns the attempt into an ordinary,
  retryable failure; an unreachable GitHub leaves the outcome explicitly
  *unconfirmed* — never a guessed failure.

A local database alone cannot guarantee exactly-once execution when GitHub
may succeed before Factory persists the result. What Factory guarantees
instead is practical idempotency: one operation per run at a time, existing
PRs linked instead of duplicated, unknown outcomes never retried blindly,
and stale attempts unable to corrupt newer state.

## Security model summary

- **Agent permissions vs Factory delivery permissions** are distinct
  systems; see [Policies](policies.md). No custom role instruction can
  trigger `git push` or `gh pr create` through Factory.
- Shell injection from issue titles, malicious PR titles/bodies, and
  branch-name injection are all structurally prevented: issue text travels
  as data (never through a shell), branch names are always Factory-generated
  `factory/run-<id>`, and PR titles/bodies are passed as single argv values.
- Repository URL manipulation is blocked: remotes are parsed with a strict
  `owner/name` charset and cross-repository issue imports are refused.
- Authentication tokens are never read, logged, or persisted by Factory.

## Delivery states

```text
not_ready → ready → pushing → creating_pr → published
                       ↘ failed (actionable error, retryable)
                       ↘ interrupted (outcome unknown → reconcile first)
```

Delivery state is deliberately separate from RunStatus: a workflow can be
`Completed` while its delivery is `not_ready`, and a `failed` delivery can
be retried once its blocker is resolved. An `interrupted` delivery blocks
retry until reconciliation confirms the remote outcome; `effective` state
reporting always shows active, failed, and interrupted operations instead of
masking them as `ready`.

## Troubleshooting

| Symptom | Cause and fix |
| --- | --- |
| *GitHub CLI not found* | Install `gh` and ensure it is on PATH. |
| *GitHub authentication required* | Run `gh auth login`. |
| *no GitHub remote* | `git remote add origin <github-url>` inside the project. |
| *Push rejected: remote branch diverged* | The remote branch moved; Factory refuses to force-push. Re-run or rebase manually and retry. |
| *permission denied* | The account lacks access to the repository; check `gh auth status`. |
| *a pull request already exists* | Not an error: Factory links the existing PR. |
| *base branch unavailable* | The PR base (e.g. `main`) no longer exists on the remote. |
| *branch drift* | The local `factory/run-<id>` head no longer matches the persisted integration head; Factory blocks publishing instead of pushing an unexpected state. |
| *network unavailable* | Offline; retry when connected. |

## Not implemented (later milestones)

Automatic merge, automatic issue closing outside normal PR semantics,
continuous Issue polling, webhooks, GitHub Apps, CI auto-repair, PR
review-comment agents, automatic deployment, and remote/cloud workers are
intentionally out of scope for this milestone.
