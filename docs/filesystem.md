# Filesystem contract

The factory respects a strict filesystem contract. Understanding it matters more than
the code, because every design decision in the codebase follows from it.

## The `.factory/` directory

The factory root is the directory in which you run `factory`. All system state lives in
`.factory/`, which is listed in `.gitignore` and never committed.

```text
.factory/
  db.sqlite3              # runs, tasks, task_dependencies
  worktrees/              # one directory per task
    t<task-id>/           # git worktree on branch factory/t<task-id>
```

Rules:

- **The main working tree stays clean.** Agent work happens in worktrees, never in the
  tree you commit documentation and CI to.
- **No state anywhere else.** No hidden dotfiles scattered across the tree, no separate
  config locations. A factory is self-contained in `.factory/`.
- **Worktrees are git worktrees, not copies.** `.factory/worktrees/t<id>` joins the
  same repository, uses the shared object store, and reports itself via
  `git worktree list`. You can inspect them with plain git.

## Why worktrees and not folders

Worktrees give real isolation with zero copying overhead:

- Each task's worktree points at its own branch (`factory/t<task-id>`).
- Commits made inside a worktree never touch the main branch or other tasks' branches.
- `git` itself tracks which worktrees exist, so the factory can enumerate them with
  `git worktree list --porcelain` instead of guessing over the filesystem.
- Refusing to remove worktrees with uncommitted changes (`factory worktree remove`),
  combined with branch-per-task, guarantees you never silently lose agent output.

## Repository requirements

- The factory root must sit inside a git repository for worktree commands. `init` and
  `run` (planning) do not require a repository; worktree creation does.
- Repository discovery walks upward and stops at the filesystem "ceiling" so a factory
  inside a nested path can never escape into an unrelated parent repository.
- On Windows, paths are canonicalized before bounded checks so the 8.3 short-path form
  of `%TEMP%` cannot defeat them.

## What is intentionally *not* stored

- Plan output is not cached. Every `factory run` replans from the provider.
- Model conversation history is not retained. Only the derived plan, token usage, and
  task state are stored.
- Nothing is stored in the repository itself: `.factory/` is ignored, so the history
  that ships in the repo is exactly what you intend to share.