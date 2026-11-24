# Workspace navigation contracts - P04

## Structure

Navigation is project -> workspace navigation session -> typed views. The top project strip uses authoritative P03 IDs and supports selection, keyboard reorder and native drag reorder. The second row provides session tabs, an overflow/archived-session selector, session creation/rename/hide, presets, view creation/reopening, layout commands and theme selection. At constrained widths the selector retains session access while the tab strip collapses.

The activity rail opens real workflows, agents, context search and settings, plus truthful limited Explorer/Source Control presentations. V1 and P03 management stay reachable. V2 is opt-in at `/workspace.html`; the default V1 `/` route is preserved. `/?project=<id>` is the explicit V1 compatibility adapter for another registered project; its project scope is immutable for that document and signed by the same browser authentication boundary.

## Operations

| Intention | Presentation effect | Backend effect |
| --- | --- | --- |
| Open registered project | Add/select project and a saved or new navigation session | Explicit P03 open when needed; no workflow start |
| Switch existing project tab | Preserve old session; activate destination | No cancel, pause, shutdown or routing mutation |
| New navigation session | Stable `s_` identity, independent views/layout | Save workspace metadata only |
| Hide navigation session/project tab | Retain recoverable session/view state | No suspension; P10 is not implemented |
| Open/activate view | Resolve by kind, project and resource; stable `v_` identity | Scoped reads/subscriptions only unless an explicit existing action is selected |
| Close view | Retain closed record and draft; remove its presentation | Unsubscribe its readers/socket; process remains owned by Rust |
| Hide view | Retain view and subscription state for restoration | No execution command |
| Move/split/resize/maximize | Public Dockview operation, serialized tree | Save presentation metadata only |
| Reopen view | Reuse exact view ID and saved presentation state | Reattach its scoped readers; never create a process |
| Apply preset | Rebuild group arrangement, retaining resources/drafts/closed history | No execution command |

Session hiding is deliberately named as navigation, not a completed suspension. The status announcement states that backend work continues. AgentSession IDs, run IDs and navigation-session IDs are never substituted for one another.

## Typed view registry

`production/model.ts` is authoritative. Live types are home/workflows/workflow/progress/agents/console/terminal/search/sourceControl/review/tests/settings/scratch, with limited Explorer/Code integration boundaries. Editor, diff, diagnostics and extension creation remain disabled pending their services. A restored unsupported view recovers as a boundary rather than executing arbitrary extension code.

By default a view belongs to its containing session's primary project. The explicit Open another project's view beside this action can pin a different project's view into that layout. The view's own project ID remains the API owner and appears in its contextual header. This is an intentional presentation composition, not cross-project agent context injection.

View titles can change without changing IDs. Closed views reopen by exact ID, so two same-title workspace notes do not overwrite one another. Resource identity includes project, kind and numeric resource ID; P05 file contracts must additionally carry authorized file/worktree identity rather than a basename.

## Keyboard and focus

Shortcuts operate only while focus is inside the application and a modal is not trapping focus. Browser tab/window shortcuts such as Ctrl+W, Ctrl+Tab and Ctrl+L are retained.

| Key / alternative | Action |
| --- | --- |
| F6 / Shift+F6 | Next/previous dock group, focus active content |
| Alt+PageDown / Alt+PageUp | Next/previous view tab in the active group |
| Alt+Shift+W | Close active view; dirty draft confirmation when needed |
| Alt+Shift+P | Focus project selector |
| Alt+Shift+B | Restore a hidden view, or hide the active view if none is hidden |
| Alt+Shift+O | Open/focus Orchestrator boundary |
| Alt+Shift+1..4 | Focus / Code / Agents / Review presets |
| Shift+F10 | Open active view's context commands |
| Project/session tab Delete | Hide its navigation tab; no backend stop |
| Tab Alt+Left / Alt+Right | Reorder project/session tabs |
| Layout actions menu | Reorder view, move to another group, split, hide/close or maximize |
| Resize active panel dialog | Numeric width/height and step controls, bounded by viewport/minima |

F6 is intentionally used as an IDE group command only within ASF; outside the application it retains browser behavior. Docking is never pointer-only. Menus support native Enter/Space, arrows, Home/End, typeahead and Escape. Dialogs trap focus and restore it on close; draft closure uses an alert dialog. Context actions return focus to the active work area. Critical state changes use a polite live region, and states have text rather than color alone.

## Dirty-state integration boundary

The current CodeMirror note owns no repository file. It supplies dirty state, text, selection and scroll to the persistent view record. A close asks whether to retain the draft and close the presentation; cancellation keeps the view. Reopening restores that record. Pending workspace saves activate before-unload protection. P05 must connect real document ownership and external-file conflict handling to this contract; it must not interpret a tab disappearance as a save or filesystem deletion.

## Failure and compatibility

A failed project or view does not remove other project tabs. Opening conflicts, permission errors, missing resources, authentication/API failures, lost streams and corrupt layouts expose recovery actions without a false success message. Runtime opening/reconnection stays explicit. Layout restoration is not a mission replay.

All V1 capabilities remain reachable through the scoped adapter. V1's existing Agent Graph, run creation/detail, consoles, roles, recipes, MCP, settings, performance and delivery retain their original components and Rust handlers. The P04 workspace does not copy those engines or expose unimplemented publication controls. See [architecture](workspace-architecture.md), [persistence](layout-persistence.md) and executed evidence (historical record held privately).

## P05 editor navigation

Explorer and Search now open real repository editor views; editor/diff view kinds are live rather than future placeholders. Root identity stays attached during open, split, move, rename and restore. Multiple panes normally share one document with independent selections. `Ctrl/Cmd+S` saves the current repository buffer and `Ctrl/Cmd+Shift+S` saves dirty documents with per-file results. Browser close shortcuts remain browser-owned. Existing `Alt+Shift+W` opens a dirty-file close dialog; Save requires backend confirmation, Discard affects the shared document, Keep draft requires confirmed recovery, and Cancel leaves the view open. Workspace notes preserve their existing P04 retained-note behavior. Later LSP/debug/extension controls remain unavailable.

CodeMirror uses Tab for indentation while editing. F6 leaves the current group; Ctrl+M (Alt+Shift+M on macOS) toggles CodeMirror tab-focus mode. Read-only source and comparison panes remain focusable. Keyboard tab switching restores focus after Dockview visibility updates and skips disabled controls.
## P06 IDE navigation additions

The production registry now enables Diagnostics, Debug, Development terminal and Extensions alongside the existing P05 file editor and P04 agent-console views. A development terminal carries an opaque IDE terminal identity; it is not a numeric AgentSession. Extension view identities include the project and installed extension/contribution ID. Restoration only reconnects/inspects existing state and never authorizes a launch.

| Source-editor shortcut | Action |
| --- | --- |
| F12 / Shift-F12 | Definition / references, when advertised |
| F2 | Reviewed symbol rename |
| Alt-Shift-F | Reviewed formatting, when advertised |
| Mod-Shift-Space | Official client signature help |
| Shift-F10 | Keyboard source context menu |
| Alt-Shift-W / Alt-Shift-K | Original test-package commands only while those packages are enabled |

Server results and source navigation retain immutable project/root/file ownership. A response to a language action in A cannot move the active B workspace back to A after a project switch. Hiding/moving editor/debug/terminal panels does not cancel workflows or terminate tools. Explicit Stop is a separate action. Native shell trust, debug launch and package installation keep their explicit confirmations.
