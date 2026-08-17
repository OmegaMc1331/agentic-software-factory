# Plan: automatic repository integration of approved task work

Status: accepted (design interview, 2026-08)
Scope: v0.4.0. Addresses the "automatic branch integration" gap listed as
Not implemented in `README.md`.

## Outcome

Every approved *implementation-family* task (`implement`, `verify`, `post_process`)
lands its repository changes deterministically on a per-run integration branch
`factory/run-{n}` (n = run id). Tasks that follow (verify, post_process, rework,
advisory reads) base their worktrees on the latest integration head, so the run
accumulates one coherent, linear history. When the run completes, `factory/run-{n}`
holds the full integrated diff, ready for the user to review and merge into `main`
manually. `main` and the main working tree are never touched by Factory.

## Decisions (confirmed in interview)

1. **Topology & mechanics** — per-run branch `factory/run-{n}`; every task worktree
   is created from the latest integration head; integration is a fast-forward
   (`git update-ref`) of the run branch to the task branch head. Fast-forward first;
   only when `git merge-base --is-ancestor` fails (unexpected divergence) is the task
   branch rebased onto the integration head inside its own worktree; conflicts fail
   the task with a readable message.
2. **Commit strategy (hybrid)** — agents may commit their own work (encouraged, so
   authorship reflects them); factory always commits any remaining uncommitted
   changes at approve time with deterministic identity `<agent name> via Factory
   <run#>` / `factory@local`. Integration never depends on agent behavior.
3. **Worktree lifecycle** — implementation-family worktrees are removed on final
   approval (`git worktree remove`). Advisory and specialized-review worktrees are
   retained while dirty (evidence only) and pruned when clean. Startup reconcile
   prunes stale worktrees.
4. **Exception tasks** — advisory and specialized review never commit or integrate;
   their product is a `RoleArtifact`. `request_changes` from a specialized review
   routes back into the implementing task's worktree (`rework_after_review`) which
   then re-integrates under the normal rules.
5. **Surfacing** — new `runs.integration_sha` column (one `schema_migrations` step),
   `/api/runs/:id` detail gains `integration { branch, head, integratedTasks }`, and
   the dashboard renders it (WorkflowInspector overview + NodeInspector task tag).
   No branch-diff explorer.

Explicitly out of scope: auto-merging `factory/run-{n}` into `main` when the run
completes; parallel scheduling; plan editing.

## Repository invariants

- The run branch is always an ancestor of the active task branch. Guaranteed by the
  execution model: tasks run strictly sequentially; each worktree bases off the
  current integration head; rework stays inside the same task worktree; the
  divergence fallback rebases the task branch onto the run branch (making it a
  descendant again). Therefore integration is a fast-forward in every expected path.
- Evidence keeps meaning across rework: `base_sha` for an attempt must be pinned to
  `runs.integration_sha` when present (else the worktree HEAD at attempt start).
  This gives a stable "diff against run state" for the Reviewer across attempts.
- No ref or working-tree mutation of `main` ever happens (all integration uses
  `git update-ref` or worktree-local rebase).

## Phases

### Phase 1 — `factory-git` primitives (`crates/factory-git/src/lib.rs`)

Add, with tests (temp-repo based, mirroring existing `factory-git` tests):

- `commit_changes(worktree: &Path, message: &str, identity: &(String, String)) -> Result<Option<String>>`
  — `git add -A`, `git commit --author "<name> <email>"`; returns the new commit sha,
  or `None` when there is nothing to commit.
- `branch_exists(name: &str) -> Result<bool>` — `git show-ref --verify
  refs/heads/<name>` in the main repo.
- `is_ancestor(ancestor: &str, descendant: &str) -> Result<bool>` —
  `git merge-base --is-ancestor` (exit 0/1 both valid; 128 = missing object error).
- `update_ref(name: &str, sha: &str) -> Result<()>` — `git update-ref ... refs/heads/<name> <sha>`
  in the main repo (ref-only; no checkout, main working tree untouched).
- `rebase_onto_in(worktree: &Path, onto: &str) -> Result<()>` — `git rebase <onto>`
  inside the worktree; conflicts surface as a readable `GitError::CommandFailed`.
- Extend `add_worktree(worktree: &Path, branch: &str, base: Option<&str>)`:
  `git worktree add -b <branch> [<base>]`; `None` keeps the current HEAD base so the
  existing call sites stay valid.

### Phase 2 — `factory-core` integration logic

`crates/factory-core/src/factory.rs`:

- `run_branch(run_id: i64) -> String` helper: `factory/run-{run_id}`.
- `create_worktree(task_id)` (line ~1573): base = `run_branch(run.id)` when
  `branch_exists`, else `None`. Passed through the extended `add_worktree`.
- `integrate_approved_task(&self, run_id, task_id, attempt_id, worktree, agent_name)
  -> Result<Option<String>>`:
  1. Main `Repo::detect_bounded(root, root)`.
  2. If `!branch_exists(run_branch)`: `update_ref(run_branch, main HEAD)` — the first
     task's worktree bases off main HEAD, so a fast-forward is valid.
  3. If the worktree still has uncommitted changes (or untracked files):
     `commit_changes(worktree, "factory: integrate run-{n} task-{id} ({agent})",
     (&agent_name, "factory@local"))` — returns `None` when clean. If `None` and
     there is no diff, the task contributed no repository changes: return `None`
     without moving the run branch (task still completes).
  4. `head = repo.head_sha(worktree)`; if `!is_ancestor(run_head, head)`:
     `rebase_onto_in(worktree, run_branch)`; on error → task fail (see below).
     Recompute `head = repo.head_sha(worktree)`.
  5. `update_ref(run_branch, head)`; `db.set_run_integration(run_id, Some(head))`;
     return `Some(head)`.
  - Failure mode: any integration error after reviewer approval finishes the attempt
    as `Failed` with a clear message, marks the task `Failed`, and returns `Ok(false)`
    so the run stops (`execute_active_run_inner` marks the run `Failed`). The user can
    retry; the same worktree is reused.
- Hook the approve path in `execute_implementation` (line ~784): after
  `review.decision == ReviewDecision::Approve`, call `integrate_approved_task`
  **before** `finish_task_attempt(Approved)` / `mark_task(Completed)`. The attempt
  records the integrated sha as its `commit_sha`. Verify and post_process approve the
  same way.
- Advisory (`execute_advisory`) and specialized review (`execute_specialized_review`)
  are unchanged — no commit, no integration.
- Pin `base_sha` for implementation-family attempts to `db.get_run_integration(run_id)`
  when present, else `repo.head_sha(worktree)` (lines ~544, ~844, ~1038).
- After final approval + integration for implementation-family tasks, call
  `remove_worktree(task_id, false)` (line ~1588). Keep the task branch ref (commits
  are referenced by the run branch; the task-name branch is deleted by `git worktree
  remove`'s cleanup only when it is the worktree's branch — confirm and, where needed,
  `prune`).

### Phase 3 — `factory-db`

`crates/factory-db/src/lib.rs`:

- New migration `version < N`: `ALTER TABLE runs ADD COLUMN integration_sha TEXT;`
- `set_run_integration(id, sha: Option<&str>)`, `get_run_integration(id) -> Result<Option<String>>`,
  surfaced on `Run` (or as a helper) so the API reads it without shell access.
- Tests: upgrade path from the previous schema; set/get round-trip and clearing.

### Phase 4 — `factory-api`

- `crates/factory-api/src/types.rs`: `RunDetail` gains
  `integration: Option<IntegrationStatus>` where
  `IntegrationStatus { branch: String, head: Option<String>, integratedTasks: usize }`.
- `crates/factory-api/src/app.rs` `get_run` (~line 269): populate from
  `db.get_run_integration`; `integratedTasks` = count of tasks with a latest attempt in
  `approved` (or state `completed`).
- Tests: run detail serializes the `integration` object.

### Phase 5 — dashboard

- `apps/dashboard/src/types.ts`: `IntegrationStatus` interface; `RunDetail.integration`;
  extend `Task`/`TaskMeta` only if a per-task integrated flag is needed (prefer deriving
  from `RunDetail`).
- `apps/dashboard/src/components/WorkflowInspector.tsx` (~line 276): add an
  "Integration branch" row in the overview (`factory/run-{n} @ short sha · n tasks
  integrated`, or "—" before the first approval).
- `apps/dashboard/src/components/NodeInspector.tsx` (task branch, ~line 242): show an
  `integrated` tag when the task is completed and run integration exists; show the
  task branch name.
- Update `api.test.ts` / `App.test.tsx` fixtures for the new field.

### Phase 6 — docs & README

- `README.md:188` — drop "automatic branch integration" from Not implemented; add a
  short "Repository integration" note under *How it works* (branch `factory/run-{n}`,
  worktree bases, prune-on-completion, no automatic `main` merge).
- `docs/architecture.md` — workflow-lifecycle diagram: insert the integration step
  after "approve"; add a migrations paragraph for the `runs.integration_sha` step.
- `docs/security.md` — confirm integration adds no dashboard-executable surface and no
  mutation routes; all git plumbing stays in the Rust process.

### Phase 7 — end-to-end validation

- New integration test (temp repo, fake agents): plan implement → verify → implement;
  assert `factory/run-1` linearly accumulates both commits, the verify task's worktree
  bases off the first implementation's integrated head, worktrees are removed on
  completion, and `git diff main..factory/run-1` equals the union of accepted diffs.
- Add/re-run existing core tests that assumed no integration (adjust worktree-lifetime
  and base-sha expectations).

## Sequencing

1. Phase 1 (git primitives + tests) — compiles alone, additive.
2. Phase 3 (db) — additive migration.
3. Phase 2 (core) — gated on 1+3; then 4, 5, 6, 7 in order.
Check each phase: `cargo build`; `cargo clippy --workspace --all-targets -- -D warnings`;
`cargo test --workspace`; dashboard `npm run format:check && npm run lint && npm run
typecheck && npm run test && npm run build`.

## Risks

- Rebase conflicts in the divergence fallback: fail the task with a readable message,
  never silently discard work.
- Pinning `base_sha` to `runs.integration_sha` changes reviewer diffs on rework
  (fuller, task-scoped diff): intended; verify mission output contract is unchanged.
- Migration is additive and non-destructive for existing databases.
- Optional agent commits must not alter the structured output contract — only prose.

## Acceptance criteria

1. `factory/run-{n}` exists after the first approved implementation task and, after a
   completed run, `git diff main..factory/run-{n}` equals the union of accepted changes.
2. A later task's worktree bases off the latest integration head (verify sees the
   implementation it checks).
3. Completed implementation-family worktrees are removed; advisory/review worktrees
   remain only while dirty.
4. Rework and specialized-review `request_changes` re-base and re-integrate; the run
   branch stays linear.
5. `runs.integration_sha` persists; `/api/runs/:id` carries `integration`; the
   dashboard renders it; fixtures/tests updated.
6. `main` is untouched; no new mutation routes; README/docs updated; CI green.