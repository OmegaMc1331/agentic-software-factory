# Workspace persistence - P04

## Storage and versions

The authoritative presentation document is saved through authenticated `GET /api/workspaces`, `GET /api/workspaces/:id` and `PUT /api/workspaces/:id`. Storage is adjacent to the configured P03 registry: replacing its extension with `workspaces.sqlite3`. The default `registry.sqlite3` therefore uses `registry.workspaces.sqlite3` in the same application-data directory, outside every project's `.factory` directory.

The application DB uses SQLite `PRAGMA user_version=1`, table `workspaces(id, revision, document, updated_at)`. This database version is distinct from JSON document version 2. First creation is transactional. Unknown/nonempty foreign DBs and unsupported future database versions are refused without reset. A workspace-storage error does not stop the project registry or its runtimes. Existing project SQL migrations 1-18 are unchanged.

Limits: 64 saved workspace documents, 1,048,576 UTF-8 serialized bytes per document, eight concurrent blocking workspace operations and a five-second SQLite busy deadline. Limit/corruption errors preserve existing records. No automatic eviction deletes workspaces or project data. Back up/repair the application DB offline with writers stopped; do not copy only the main file of a live WAL database and call it a consistent backup.

## Contract

PUT body is `{base_revision, document}`. An absent record requires base revision 0 and returns revision 1. Every subsequent save requires the exact current revision. Check/update happen inside one immediate transaction. A stale writer receives HTTP 409; capacity/body errors are 413, unavailable storage 503, request saturation 429, malformed identity/schema 400 and missing documents 404. Existing auth/Origin/CSRF statuses still apply. Reads do not require CSRF; every write does.

The document contains:

- `version: 2`, stable `id`, ordered `openProjects` and `sessions`.
- `activeProjectId`, `activeSessionId`, `preferences.theme` (`system`, `light`, `dark`).
- Each session: stable ID, primary project ID, title, views, serialized Dockview tree, active view, preset, activity, timestamps and navigation-archive flag.
- Each view: stable ID, kind, owning project, containing navigation session, numeric resource/agent reference where applicable, title, hidden/closed flags and allowlisted presentation state.
- Note state: draft, dirty flag, selection anchor/head and scroll. Search/paging state: query, cursor and page.

Browser decoding limits open projects and sessions to 32 each, view records to 128 per session, note text to 65,536 characters, and layout recursion to 16 levels. Duplicate identities, malformed references, oversized data, unsafe group names and unsupported layouts are refused. File paths are not authorization inputs: P05 must introduce typed file references together with its Rust file service.

No cookies, CSRF tokens, provider credentials, project configuration, agent output or privileged runtime objects are automatically copied into the document. User-authored note contents remain local workspace data and should be treated as potentially sensitive. Project identities in the document confer no authorization.

## Browser state and saving

`asf.v2.workspace.last` in localStorage selects the last workspace. The existing P02 `asf.v2.p02.theme` key remains the shared theme preference; the workspace also retains its chosen theme. `asf.v2.workspace.recovery` in sessionStorage holds a bounded pending document and base revision for that browser tab. It is not a runtime checkpoint or a durable process lease.

Presentation changes schedule a 350 ms debounced save. Only one write is in flight; changes made during it remain pending for the next revision. A successful acknowledgement clears the recovery copy only if it covers the latest generation. Quota or API failures remain visible and pending; the app never displays Saved for an unacknowledged revision. Before-unload warns while changes have not reached durable storage. The pagehide path attempts a save but is not treated as a reliable close notification.

A browser reload can recover the tab's pending document. A normal close/reopen restores the last acknowledged server document through the stored workspace ID. Hard loss of browser storage before a pending change is acknowledged is not guaranteed recoverable; this is why the unsaved-state warning remains. P10 mission recovery is unrelated.

## Migration and recovery

Supported production version-1 documents are normalized to version 2 with default presentation fields, validated identities and retained drafts. Existing P02 `asf.v2.p02.workspace.*` documents contain fixtures and a different identity model: they are left intact and deliberately not imported into production. No synthetic project, chat or terminal record becomes live by migration.

Only an allowlisted layout is passed to Dockview. Node shapes, dimensions, IDs, uniqueness, panel references and component types are checked. The reconstruction discards arbitrary parameters and derives `workspace` content, `asf` tabs, always-rendered content and useful minimum sizes from trusted view kinds. Invalid trees restore the preset while retaining open/closed view records and drafts. Unknown view kinds become a truthful integration-boundary view with a warning.

Unknown document versions and malformed documents preserve the stored value and present recovery choices. A safe empty workspace receives a new identity; it does not overwrite the bad document. Removed/unavailable projects retain their tabs and views with an accurate error. Removed workflows or agent sessions show a scoped error. Renaming a display name changes labels, never identities.

Server restart does not change the layout ID or project/resource ownership. The P03 startup default can already be open; other runtimes remain closed until explicitly reopened. Restoring a layout never starts/retries/cancels a workflow, creates a process, sends terminal input or publishes a change. Existing streams show disconnection and require reconnection when necessary; volatile PTY/CLI memory is not restored.

## Multiple windows

The policy is revision-based conflict detection, not last-writer-wins and not automatic merging. Windows can begin from the same saved workspace. A window retains its local document until it saves. If another window saved first, the stale save is rejected with 409 and local changes remain in sessionStorage. The user chooses an independent copy (new workspace ID, same project/view identities) or explicitly reloads the server version. A change in one window never silently replaces another window's live layout.

The shared last-workspace pointer identifies which workspace a newly opened window will initially select; it does not transmit live layout changes. There is no global browser current-project variable affecting API routing. SessionStorage recovery copies are independent between browser tabs even when their initial content was cloned by the browser. Replacing pending local changes by a saved or empty workspace requires an explicit confirmation; the independent-copy option retains those changes. Replacement waits for an in-flight save and cancels its debounce timer.

Actual commands, restoration comparisons, malformed-data tests, multi-window tests and limitations are in V2-P04 (historical record held privately). Remote CI and native Linux/macOS qualification are separate from local Windows results.

## P05 schema version 3

The SQLite workspace table/schema is unchanged; the presentation document version is now 3. Version 1/2 documents migrate through the existing decoder. Typed views may add `file: {project_id, root_id, path}`, `documentId`, `comparison` and bounded explorer/editor presentation fields. Project identity is checked against the containing view, and source contents are excluded from layout JSON. Unknown or malformed records retain P04 safe recovery behavior. Draft contents and base revisions are separately owned by the [editor document service](editor-document-model.md). Restoring either layer never writes source files automatically.
