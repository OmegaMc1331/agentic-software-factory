# P03 - V1 preservation, migrations and recovery

## Storage ownership

Project storage stays in `<repository>/.factory/`: DB, config, recipes, roles, context, artifacts, histories and worktrees are not merged into a global database. The new application registry is separate, schema version 1. P03 does not add a project SQL migration, renumber a workflow/task/session, rewrite TOML or recreate agent profiles.

Opening an initialized V1 repository registers its canonical root and opens the same DB through Factory Core. A legacy initialized non-Git directory still opens for existing CLI/API compatibility; Git features remain unavailable there. Registry removal preserves its files; relink preserves its registry identity. A project unavailable at startup is retained and reported. Never delete `.factory` or reset a DB to make opening succeed.

## Database upgrades

The existing eighteen SQL migration definitions are unchanged. Before applying pending steps to a nonempty file, `factory-db` validates a contiguous supported migration history, takes an online SQLite backup into `.factory/backups/db-before-v18-<timestamp>-<pid>-<sequence>.sqlite3`, and verifies `PRAGMA quick_check` on that backup. Existing backup locations must be real directories without symlinks/reparse points. A backup error refuses the upgrade.

All pending schema steps and migration-history writes now run in one immediate transaction. A failure rolls the complete pending batch back. An already current DB takes the read-only fast path and creates no migration backup. Existing schema-18 V1 projects therefore require no conversion. New empty DBs do not need a historical backup. A noncontiguous/newer schema is refused with the original file retained.

The SQLite online backup API includes committed WAL contents. Do not replace this with copying only `db.sqlite3` while writers run. Backups are verified SQLite snapshots, not a claim that all repository files or external process state were snapshotted. They contain the original project's data and inherit the local user's storage trust boundary; no remote upload is introduced.

## Startup reconciliation

The project OS lock is acquired before runtime recovery. Existing interrupted-run/task/attempt/session transitions and MCP grant revocation remain in force. An interrupted external delivery is marked uncertain/interrupted and requires the existing read-only reconciliation before retry. No startup push or PR replay occurs.

Opening a runtime now uses `reconcile_interrupted_preserving_worktrees`: historical and in-progress worktrees survive opening, including clean worktrees. Existing explicit cleanup commands retain their semantics. This is state reconciliation, not P10 checkpoint-based mission resumption. A CLI without native resume support has no recovered volatile memory.

## Failure and recovery

| Failure | Behavior / recovery |
| --- | --- |
| Registry unavailable/corrupt/newer/missing table | Refuse authoritative lookup; retain file/journal. Stop writers and restore a verified registry backup or repair a copy offline. No default fallback. |
| Repository moved | Retain unavailable entry. Close it, relink to an authorized root, acknowledge identity change only when appropriate, then open. |
| DB replaced | Refuse the live/registered identity. Close, inspect backups and relink explicitly; do not silently use the replacement. |
| Runtime lock owned elsewhere | Report refusal; other projects remain usable. Stop the owner or close it safely before retry. |
| SQL migration fails | Transaction rolls back pending steps; verified pre-upgrade backup remains available. Other projects continue. |
| Explicit initialization fails | No completed entry is invented. Existing `.factory` is retained; inspect it before repair, never automatically overwrite. |
| Create/clone fails after reserving destination | New directory retained for inspection, no successful registry entry. Existing directories are never overwritten. |
| Browser switches project / closes a file | No project runtime mutation. P10 will supply browser-owned session suspension. |

To restore a project DB, stop every owning runtime/process first. Preserve the failed DB and any journals in a separate recovery directory, verify the chosen backup, then restore the consistent snapshot offline. Revalidate registry DB identity through explicit relink before reopening. Never restore over a live SQLite connection. Restoring a snapshot discards later DB changes; code/worktrees may need separate reconciliation. P03 does not promise transactional rollback of arbitrary tool execution or remote GitHub actions.

## Evidence

`factory-db/tests/v1_compatibility.rs` checks the frozen P00 V1 SQL fixture's sixteen tables and every original row/column through repeated opens. Existing migration tests cover older schemas. P03 adds backup verification and all-pending-step rollback tests, then opens that same frozen fixture through a real `ProjectManager`, verifies workflows/tasks/attempts/sessions/artifacts/linkage, preserves a real clean Git worktree and config bytes, closes/reopens and restarts the manager with stable identity. A deliberately inconsistent migration fixture verifies backup retention, rollback and B's availability when A fails.

See [the isolation matrix](project-isolation-test-matrix.md) and delivery evidence (historical record held privately) for executed commands and platform limits. Windows native checks are distinct from pending Linux/macOS and exact-commit remote CI.

## P05 editor data

P05 does not alter the project workflow database or its migration sequence. The lazy `.factory/editor-state.sqlite3` sidecar uses its own transactional schema version 1 for drafts and file-operation recovery. `.factory/editor-recovery` retains explicitly deleted entries and uncertain write versions. A corrupt existing sidecar is refused and preserved rather than reset. Protected editor APIs cannot modify either database or Git internals.

Workspace presentation documents advance to version 3 without changing the application SQLite table schema. Versions 1 and 2 are accepted by the frontend decoder; file contents remain outside layout JSON. Older application versions may refuse the newer presentation document, so preserve its recovery copy when rolling back. Stop owning processes before backing up databases, SQLite journals/WAL, recovery entries, source files and worktrees. A copied SQLite main file alone is not a verified live backup. Restored drafts never overwrite disk automatically; file recovery restores never overwrite an existing destination.
