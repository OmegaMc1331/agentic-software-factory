# Upgrade, database compatibility and recovery

Do not upgrade a production project to an unqualified candidate. Stop ASF normally
first and retain the old executable and its exact hash. Do not run old/new servers
against the same project simultaneously.

Back up the complete application-data directory and project `.factory` directories
while stopped, together with the repository and any retained worktrees. A standalone
copy of an open SQLite main file is not a valid WAL-aware backup. Use SQLite's
backup API or stop all writers before copying the main database and its sidecars.
Keep backups outside the project and protect them like source code and credentials.

P14 updates the bundled SQLite engine to the patched rusqlite/libsqlite3-sys
family. It retains the existing authoritative schemas and migration ordering.
Project database upgrades use the existing verified online backup before pending
migrations, then transactional schema changes. Unknown/newer/noncontiguous schemas
fail closed and preserve the original database. This is not a universal backup
service for every application sidecar.

Replace only the portable application files. Launch with the same explicit
registry and authorized roots. Confirm projects, conversations, completed attempts,
profiles, checkpoints, memories, reviews, settings and notifications before starting
new work. Reopening presentation state must not start missions or publish GitHub
changes. Pending external effects require reconciliation, never blind retries.

## Failures

- Lock/busy error: close the known writer and retry after inspecting ownership;
  do not delete lock/WAL files from a running database.
- Migration failure: retain the error and original database; inspect the verified
  pre-upgrade snapshot in `.factory/backups`. Do not initialize over the failure.
- Corruption: stop ASF, preserve the database and sidecars, verify a separate copy
  with `PRAGMA quick_check`/`integrity_check`, and restore a demonstrated valid backup.
  ASF does not promise recovery from arbitrary corruption.
- Missing project path: relink explicitly through the registry after checking
  identity. Do not substitute another similarly named directory.
- Incomplete checkpoint or uncertain integration/delivery: inspect durable evidence
  and use existing lifecycle/reconciliation controls. Do not manually replay a push,
  merge or completed task.

## Downgrade

Automatic rollback is unsupported. An old binary may not understand newer schemas,
SQLite features or execution records. Never delete migration rows to make it open.
Restore the complete compatible pre-upgrade backup into an isolated location and
verify it with its matching old executable. Preserve newer state separately; do
not overwrite it as part of rollback. A candidate passing an earlier-schema fixture
does not establish universal compatibility with every historical ASF release.
