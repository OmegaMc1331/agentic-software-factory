# P05 editor documents

`src/v2/files/documents.ts` owns document buffers outside React and Dockview component lifetimes. `FileApi` captures an explicit project ID for every operation. The map key is the serialized tuple `(project, root, relative path)`. A separate `d_…` ID identifies a recovery document. Project A, Project B and A's task worktree cannot collide on `src/main.rs`.

An existing document ID is checked against that complete file identity before opening or rendering it. A malformed restored view cannot briefly display another project's cached buffer. Canonical path aliases are accepted only after backend verification, are invalidated on move, and are removed when a clean document is released. Closed alias views can reopen without following a dangling ID.

## Shared buffer, independent views

An editor document contains immutable CodeMirror `Text`, loaded base text/revision, current disk snapshot, newline/BOM metadata, dirty/conflict/recovery state and a generation. Two panes share that text; each has its own `EditorState`, selection, scroll and history. Changes are broadcast to sibling panes without echoing them as independent local edits. A canonical path alias resolves to the existing clean document. Renames performed by ASF update document identities and every referenced workspace tab while preserving dirty text.

Dockview owns presentation only. Hidden CodeMirror DOM instances are released after a short visibility grace period, retaining editor state in the document service. Moving a panel therefore neither rereads the file nor destroys its buffer. Syntax modules load locally on demand; unknown languages remain plain text. P05 supplies highlighting, indentation, history, selections, brackets, folding and local find/replace. LSP, completions from a language server and debugging are not provided by this phase.

Workspace JSON version 3 stores `file`, `documentId`, view-specific selection/scroll, expanded paths and root selection. It does not store repository contents. Supported P04 version 1/2 layouts migrate; existing workspace notes remain separate notes. Tab titles are compact presentation strings, never file identity.

## Dirty and save rules

Only a confirmed filesystem response advances the loaded base revision. Edits made during an in-flight save remain dirty against the acknowledged saved content. Save All returns per-document results and retains failed buffers. Read-only agent/binary/large/mixed-encoding documents cannot be mutated by the editor model. Closing a dirty repository view offers Save and close, Discard and close, Keep draft and close, or Cancel. Discard affects the shared buffer, including sibling panes; closing a presentation view never cancels a workflow.

Clean external changes refresh text while retaining bounded selections and view state. Dirty external changes preserve the loaded base, current disk and local draft separately. Comparison provides read-only base/disk/HEAD versus local text. Reload requires confirmation. Manually resolved saves re-read the reviewed disk revision and still submit expected-revision writes. An external rename/delete leaves a missing-file state and retained draft; P05 does not guess a replacement path.

Text comparisons require complete supported content. Binary/unsupported metadata and large-file previews cannot become empty or truncated text diffs, including when an older layout restores such a view. Both diff sides are keyboard focusable and read only. Dirty-close actions wrap inside the dialog so Cancel, discard, recovery and save remain reachable at desktop widths.

Discarding a conflicted shared draft adopts the last observed complete disk text/revision in every sibling pane without writing the repository. It does not leave a stale loaded base behind as a falsely clean buffer. Any later external update is still detected by polling and save-time revision checks.

## Recovery ownership

The project sidecar `.factory/editor-state.sqlite3` stores versioned drafts with compare-and-swap revisions. A draft contains complete file identity, schema 1, base revision/content, unsaved content, BOM/newline metadata and dirty state. No restored draft automatically writes disk. Server-side quota cleanup evicts old/least-recent clean records only; dirty drafts are never automatically removed to meet a quota.

IndexedDB `asf-editor-drafts`, store `drafts`, journals immediate local updates independently of layout JSON. It is bounded to 128 records / 32 MiB. Clean entries may be evicted; dirty entries cause a visible capacity refusal. `asf.v2.editor.window` is a tab-local owner. BroadcastChannel ownership claims disambiguate copied sessionStorage when a browser tab is duplicated. Without BroadcastChannel a fresh owner is used. Database connections/channels close with the document manager.

Project draft CAS detects competing window writes. A stale window retains its buffer and can explicitly fork an independent recovery ID; that does not bypass disk-revision protection. Different local retained versions are not silently selected over each other. Corrupt records are retained and reported. If browser storage or Rust recovery fails, the UI states that recovery is unconfirmed and warns before leaving. A browser/process failure before either storage acknowledgment can still lose the most recent input; P05 does not promise otherwise.

Limits are 64 resident documents per window, two queued document I/O slots, 128 view records per navigation session and bounded server/browser draft stores. Clean unreferenced documents can be released. Dirty buffers remain resident or durable until explicit resolution. The document service polls at most eight documents every two seconds, round-robin across roots/projects. With more documents notification latency grows; every save still checks the actual revision.

The shared file client bounds all file requests to two active operations and 128 waiting jobs per window. User reads and mutations take priority over directory refreshes; cancellation bypasses that queue so a saturated search can still be cancelled. Root catalogs share a project-keyed cache. Explorer polling rotates through eight expanded branches rather than starving branches beyond the first eight. The server's independent application/project limits remain authoritative across windows.

Each asynchronous refresh captures its file identity, base revision and a read epoch. An older response cannot replace a later refresh, a renamed document or a confirmed save. Explicit reload also checks the buffer generation before discarding text. Save All waits for already-pending document recovery, then reports individual save outcomes.

## Recovery cases

| Situation | Behavior |
| --- | --- |
| Reload / reopen | Restore layout references, load draft and disk separately |
| Rust restart | Reauthenticate/reopen project as required; same file/draft IDs remain scoped |
| Root temporarily missing | Preserve recovery contents; show unavailable root/file |
| File externally changed | Keep local draft and expose exact disk revision for comparison |
| File externally renamed/deleted | Retain the old-path draft; never recreate or retarget implicitly |
| ASF rename/move | Update references, preserve dirty text and rebase only if disk still matches the loaded base |
| Competing recovery revision | Preserve both stored revision and stale local text; require independent recovery or explicit resolution |
| Corrupt/quota-limited storage | Visible error, previous stored data retained, no false success |

The global workspace, navigation session, editor document and backend numeric `AgentSession` remain different entities. P05 introduces no P10 session suspension behavior.

## P06 semantic and debug attachments

`DocumentManager` remains the sole source-buffer owner. Its IDE compartment attaches the official CodeMirror LSP client, breakpoint/execution decorations and enabled declarative editor contributions. These do not create another document store or write files. Multiple panes share one buffer and LSP document; independent selection/scroll remains view state.

LSP versions are distinct from P05 generations and disk revisions. Reconnection seeds counters from live Rust document metadata, preserving monotonic synchronization after restoring a draft. Requests synchronize actual unsaved text. Released/renamed documents close their old language URI; moving a pane does not close it. Versioned diagnostics are accepted only for the current LSP version; unversioned publications explicitly state that freshness cannot be verified.

Rename, formatting and code actions capture buffer versions before the server computes edits, then require Rust preflight and user review. Additional affected files are opened/synchronized and the request repeated. Applying an edit updates shared buffers; P05 Save/Save All still performs revision-aware disk writes. Resource operations and unrestricted server commands are refused. An extension formatter also updates a buffer and cannot silently save or bypass a conflict.

Debug source locations resolve to authorized `FileRef` values. Breakpoint state is keyed by project/root/path/line; execution markers belong to the selected owned debugger. A failed language server or debugger leaves P05 editing, draft recovery and saving operational. Managed agent worktrees remain read only and cannot launch IDE processes.
## P07 conversation context integration

Conversation attachments reuse `DocumentManager` rather than creating another buffer. `contextSelection(documentId)` selects a focused pane's range, then its last-focused pane, then an available pane. The document catalog subscribes to stable ID/generation/status snapshots; opening or changing a file updates the attachment selector without remounting the editor.

The reference retains project ID, backend root ID, relative path and base disk revision. Dirty documents supply the shared unsaved text explicitly. Selected CodeMirror offsets are converted to UTF-8 byte boundaries over decoded text with the document's actual LF/CRLF serialization; they are not LSP positions. Rust reopens the authorized file, validates the revision/range and labels saved, unsaved or stale provenance. Attaching context never saves or discards the buffer. Diagnostics are attached only when explicitly selected and associated with the current document generation.

Composer drafts and pending command identities use a separate window-owner/project/navigation-session/conversation key; source content is not added to layout JSON. Browser storage quota failures are surfaced, with the in-memory text retained. Accepted context snapshots persist in the project conversation database, independently of P05 draft recovery. See [orchestrator interaction](orchestrator-interaction.md) for limits, redaction and trust boundaries.
