# Project memory governance and privacy

The authenticated local user controls creation, category, content, source references, status, explicit supersession and deletion. Mutations require the existing Host/Origin/CSRF checks and project authorization. Edits and settings updates use expected revisions; stale edits conflict and retain the browser draft. A successful UI save follows SQLite acknowledgement.

Memory is technical context beneath application security, project/role/profile restrictions and the mission contract. A category called Project Constraints is not a new security configuration. Instructions in a memory cannot add an executable, grant MCP access, change sandbox arguments or authorize GitHub publication. Actual policy changes use the existing authorized configuration interface.

## Corrections and conflicts

User corrections create new immutable revisions and are not silently replaced by older inference. Same-subject/scope claims can be disputed. A user may edit their scope, confirm/supersede the relevant old claim, or reject a proposal. The system preserves historical claims rather than assuming the latest timestamp is correct. Source change only establishes a need for revalidation.

The orchestrator may reference a saved memory and offer its management view when a user corrects a decision. Its finite action does not silently mutate memory. Manual memory controls remain available in the workspace without direct SQLite editing.

## Deletion contract

Deleting a record removes its current row, all revision rows, derived search rows and related queued/accepted candidate copies. No retrieval cache can resurrect it: context is selected from current rows and checked transactionally. A normalized claim fingerprint is retained as a deletion guard for automatic extraction, without the deleted title, content or source excerpt. Explicit user creation is a new intentional action, not automatic restoration.

Invocation audits retain only memory IDs, revisions and hashes. They do not provide the deleted text. Independent source documents, source-control commits, conversation messages, agent output, reviews and external provider retention are separate records; deleting a memory does not erase them. Context already sent to a provider cannot be recalled. Existing SQLite backup files and WAL/page remnants can retain older data; P11 does not claim cryptographic erasure or automatically destroy backups. No backup-expiration guarantee is introduced. Restore requires explicit operator review of the recovered state.

Disabling excludes a record from automatic retrieval while retaining editable history. Disabling the project prevents memory use and extraction without deleting records. Limits do not silently purge history. Settings expose only enforceable local controls; no invented token pricing, model confidence or retention guarantee is shown.

## Trust boundaries

Known-secret screening is bounded and imperfect. Memory is retained locally in the project database, and selected context is supplied to the configured authenticated coding CLI/provider under its existing execution policy. No plaintext provider keys or secret environment values belong in memory rows or usage snapshots. Repository sources use P05 authorization, including protected internal paths; source-derived context also obeys agent read restrictions.

Independent projects have separate storage, indexes, extraction commands and audit. Request-bound immutable project identity authorizes work; browser tab selection cannot redirect an in-flight request. The UI aborts obsolete reads and does not use a global active-project cache to construct context. Existing P03/P05 filesystem race limits, CLI trust/OS privileges and P10 manual resume remain unchanged.
