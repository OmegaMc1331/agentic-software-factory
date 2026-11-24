# Project-owned agent profiles

P09 extends the existing `Agents`, `Config`, `Factory`, task attempts and P08 scheduler. It introduces no second executor. SQLite migration 21 adds profiles, append-only revisions, execution snapshots and handoffs in each project's existing database.

Profiles use stable `ap_` identifiers; names are display text and need not be unique. A profile references a trusted TOML installation by key. It never supplies a command, argv, environment, authentication material or executable path. The installation owns those fields. API bodies reject unknown outer fields.

TOML remains authoritative for trusted installations, role definitions and project policy. SQLite owns custom profiles and their role eligibility, preference, instructions and additional restrictions. A runtime-only Config overlay maps enabled profiles into existing agent selection. Its fields are excluded from serialization, so saving legacy configuration cannot write the overlay into TOML. Existing V1 agent names, assignments, recipes, sessions, routing and attempts remain intact; no historical identity is rewritten or cloned into a competing configuration store.

Create, update and archive use short SQLite transactions. Updates require the expected revision. Revisions are retained even after archival; history pages contain 100 revisions and accept an exclusive `before` revision. Active profile listing is bounded to 1,000 profiles, with a project creation ceiling of 1,000 including archives. Profile input is bounded by the 64 KiB route limit, 16 KiB instructions, 100-byte name, 2,000-byte description, 16 roles and finite execution limits. Archival disables future execution and retains all foreign references.

Each task dispatch and implementation attempt reloads eligible profiles. The active invocation owns an immutable selected configuration. Session creation and snapshot insertion share a transaction that checks the current profile revision and enabled/archive state. A conflicting profile edit prevents launch. An edit after snapshot selection affects future executions and does not reconfigure the selected process.

Snapshots include the profile revision and instructions, requested model/mode, adapter kind, role, effective policy, relevant policy declarations and selection time. Secrets and environment values are absent. Native model observations are stored separately and do not rewrite the launch snapshot. Integration reconstructs the producing attempt's policy while retaining current project/role restrictions. Historical model labels are never taken from the current mutable profile.

The frontend's Agent Management panel is part of the P04 workspace. Profile editing retains its draft after a stale revision error. Existing role APIs expose effective profile assignments for mission selection. The legacy TOML settings surface remains usable for trusted installations and existing agents.

The project manager binds the P03 registry ID once on the execution context. Capacity keys keep their existing canonical-root identity. Profile/handoff audit records use the separate registered project ID; standalone Core fixtures without a registry retain their explicit local-root identity. This is not a mutable active-project setting.

Import/export and global reusable templates are not provided in P09. Trusted installations remain in TOML and custom configuration remains in project SQLite; there are not two independently editable copies. Migration 21 is additive and uses the existing verified pre-upgrade database backup. No down-migration is claimed. Rolling back after profile executions or handoffs requires operator reconciliation of the database, task worktrees and integration history; restoring an old database alone must not cause uncertain effects to be replayed.


## P11 project memory integration

P11 preserves immutable P09 profile snapshots. Each invocation separately records selected memory revisions; profile instructions and memory remain distinct. Memory edits cannot change agent permissions or rewrite previous sessions. See [retrieval](memory-retrieval.md).
