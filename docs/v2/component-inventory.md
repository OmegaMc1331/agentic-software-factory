# V2 component inventory — P02

The library contains **58 reusable exported React components/providers**: 17 primitives/overlays, 17 application components, 13 agentic presentations, 9 code presentations, Icon and ThemeProvider. Counts exclude types, hooks, internal helpers, tests and prototype compositions. The tables identify the actual contracts; TypeScript source is authoritative for complete native HTML attributes.

These are **ready for reuse as local frontend foundations**, with Windows/Edge validation. This is not release certification or backend integration. Eleven workspace/gallery compositions are prototype-only. No required P02 library component is intentionally left as an empty implementation. Real filesystem, terminal, LSP, DAP, Git and orchestration behavior is outside their responsibility.

## Primitives and overlays — 17

Source: `design-system/primitives/Controls.tsx` and `Overlays.tsx`.

| Component | Main contract | States and accessibility |
| --- | --- | --- |
| Button | Native button props; `tone`, `loading`, `icon`, forwarded ref | Neutral/primary/quiet/danger; native disabled, busy blocks activation; default type button |
| IconButton | Button props plus required `label`, `icon` | Accessible name required; bounded icon control; same disabled/busy behavior |
| Input | Native input props; `label`, `hint?`, `error?`, `hideLabel?`, ref | Explicit name separate from descriptions; invalid and disabled states; controlled or native uncontrolled |
| Textarea | Native textarea props plus field props | Multiline, validation description, label and focus |
| Select | Native select props; label, error, hidden-label option | Native keyboard and disabled options; name excludes error text |
| Checkbox | Native checkbox props; label, error | Checked/unchecked, disabled and invalid; error is a description, not part of its name |
| RadioControl | Native radio props, label | Native mutually exclusive group via caller-supplied `name`; disabled |
| Toggle | `label`, `checked`, `onChange`, `disabled?` | Named switch with `aria-checked`; Space/Enter native button activation |
| SearchField | `value`, `onChange`; label, placeholder, inputRef, key handler, controls/activeDescendant | Clear action, Escape clears nonempty query; optional combobox/listbox semantics |
| Tabs | `label`, `items: TabOption[]`, `value`, `onChange` | Selected focus stop, disabled skipping, arrows/Home/End; optional controlled panel IDs |
| Menu | Named entries, anchor, close callback and return-focus target | Arrow/Home/End/typeahead; disabled/danger entries; Escape/outside close; viewport-clamped portal |
| Dropdown | Label, entries, optional icon/tone | Menu trigger announces expanded/menu state; entries emit caller intentions |
| ContextMenu | Children, label and entries | Pointer and Shift+F10; returns focus; context actions are actual callbacks |
| Tooltip | Children, content | Intent delay; described control; hover/focus, Escape, cleanup |
| Dialog | `open`, `onClose`, `title`; description, children, footer, alert mode | Native top layer, explicit Tab wrap, Escape/backdrop cancellation and focus return |
| ConfirmationDialog | Open/close/confirm, title, description, confirm label, danger | Distinct confirmation callback; safe dismissal does not confirm |
| Popover | Label, trigger text, children | Named nonmodal dialog; focus on open, Escape/outside/close; return focus |

Menus use typed `MenuEntry` records with stable IDs, label, optional icon/shortcut, disabled/danger flags and an action. They do not contain arbitrary HTML strings. Portals attach inside an open dialog when their trigger belongs to it; otherwise they attach to the body. Modal use and real focus wrapping are tested in Edge, not inferred from jsdom.

## Application components — 17

Source: `design-system/components/Application.tsx`.

| Component | Main contract | States / responsibility |
| --- | --- | --- |
| CompactStatus | Status union, optional label | idle/running/completed/failed/suspended/attention/cancelled, icon plus text |
| ActivityBarItem | Icon, label, selected, activation | Named activity action with selection and tooltip |
| WorkspaceTab | Stable id, label, selected, select; dirty/close/move callbacks | Tab role; dirty description; Alt+Arrow move, Delete close; does not own session lifetime |
| PanelToolbar | Label, children | Roving enabled button focus with arrows/Home/End; leaves native fields operable |
| PanelHeader | Title, optional detail/actions | Compact contextual heading and named action toolbar |
| Breadcrumb | Labeled items with optional actions | Navigation landmark and ordered path; full segments in titles |
| FileTreeItem | Path, label, depth, directory/expanded, selected/focused, select callback; ARIA positions | One presentational row; owning tree controls keyboard navigation and virtualization |
| ResizableDivider | Label, controlled value/min/max, onChange, controlled panel ID, orientation | Pointer capture, arrows (8 px), Shift+Arrow (32 px), Home/End, bounded ARIA separator |
| StatusBarItem | Children, optional icon and click | Static metadata or genuine button; no fabricated status |
| NotificationItem | `NotificationData`, dismiss by ID | info/warning/success/danger with title/detail/time, individually named dismissal |
| EmptyState | Title, description, optional action | Small useful state; no oversized illustration or marketing headline |
| LoadingIndicator | Label | Busy mark and status role; reduced-motion support |
| ErrorBoundaryView | Title, message and retry | Alert with a named recovery action |
| ViewErrorBoundary | Children and optional onRetry | Catches render failures, isolates view, resets on retry; does not swallow runtime errors as success |
| InlineAlert | Tone, title, children, optional dismiss | Danger is assertive; other states textual and contextual |
| ConfirmationPrompt | Title, description, confirm label, confirm/cancel | Inline alternative for a local meaningful decision |
| ProgressRow | Title, supplied status, optional detail | Task state, never elapsed-time-derived progress |

WorkspaceTab close is a keyboard action rather than a nested interactive button inside a tablist. Actual dock groups have a separate close-active-view button outside the tablist. Application commands must supply any required policy validation; these controls do not authorize operations.

## Agentic presentations — 13

Source: `design-system/patterns/Agentic.tsx` and `domain.ts`.

| Component | Main contract | Responsibility |
| --- | --- | --- |
| UserMessage | `ChatMessage` | Supplied author/time/body; long-body expansion |
| OrchestratorMessage | Message, open-file and review callbacks | File references and result record; no tool-output stream or invented reasoning |
| ResultSummary | Title, file/test counts, provenance, review callback, limitation | Supplied counts with fixture/recorded-evidence distinction; does not verify tests |
| ReferencedFile | `FileReference`, open callback | Full path and changed-line counts; usable file intention |
| MissionStatus | `MissionData` | Derived completed/total task progress and supplied global status |
| CurrentMissionStep | Mission | First running/attention/pending step or completed label |
| AgentIdentity | `AgentData` | Name, role and explicitly supplied CLI model/profile description |
| AgentStatus | Agent | available/working/unavailable/error, text plus icon |
| AttentionNotification | Title, detail, optional actions | Explains decision and permits real caller action |
| TestResult | `TestData` | passed/failed/not-run with supplied command, duration and provenance |
| CodeChangeSummary | File references | Aggregates actual supplied file/line counts |
| ReviewSummary | Status, summary and evidence children | Pending/accepted view; caller owns policy and integration |
| CheckpointState | `{id, commit, verified}` | Shows durable identity and supplied verification flag |

`ChatMessage` and `MissionData` explicitly contain `provenance: fixture | runtime`; `TestData` has the same distinction. File references carry path, language, added/removed lines, manual/agent origin and review state. Mission steps have IDs, status and optional detail. Agent records include identity, role, model/profile string, availability, task and effective permission labels. These are presentation contracts to be adapted to existing Rust entities; they are not new database tables or parallel orchestration models.

P02 fixture data is in a separate `prototypes/fixtures/data.ts` module. Every preview has a persistent fixture/no-runtime banner. Components do not start agents or turn a displayed success label into backend proof.

## Code and tool presentations — 9

Source: `design-system/patterns/Code.tsx`.

| Component | Main contract | Responsibility |
| --- | --- | --- |
| CodeEditorContainer | Name and children | Named editor region with bounded flex geometry |
| EditorTabs | Items, value, change callback | Shared tab presentation and keyboard semantics |
| DiffToolbar | Split/unified mode, mode/previous/next callbacks, provenance | Diff navigation intentions; fixture labeling is explicit |
| FileBreadcrumb | Path | File-segment breadcrumb |
| LanguageLabel | Language string | Compact metadata, no claim of an available server |
| DiagnosticsIndicator | Error and warning counts | Text/ARIA count with distinct icons; caller supplies real counts |
| GitModificationIndicator | modified/added/accepted, manual/agent origin | Compact status with complete accessible description |
| TerminalHeader | Channel, change/clear callbacks, unique panel ID, provenance | Tab-to-panel relationship and explicit clear action; no process ownership |
| DebugToolbar | paused/running/stopped, action callback, provenance | Enables only appropriate supplied actions; no adapter or debug process |

An output panel ID is required so multiple future terminals can have independent accessible relationships. Runtime and fixture labels are selected from provenance, rather than hardcoding prototype language into a production presentation.

## Infrastructure — 2

`Icon` accepts a typed name, size and optional style; the consistent original SVG path family inherits currentColor and is hidden from accessibility trees when the control supplies a name. It does not import a general icon library. `ThemeProvider` accepts children and exposes preference/resolved theme/issue/setPreference through `useTheme`; it owns browser preference only in P02. Semantic CSS, fonts and motion tokens are shared infrastructure, not counted as React components.

## Prototype compositions — 11, excluded from the library count

WorkspacePrototype, ComponentGallery, DockSurface, Explorer, EditorPanel, DiffPanel, Conversation, Mission, Agents, Review and Output compose the library and selected P01 foundations. Their local controller, fixture project tabs, recorded terminal, local draft/buffer saves, agent reassignment and review acceptance are **prototype-only**. Search/preferences/resize/notifications dialogs are implementation details of these compositions and are not separately counted.

Reusable controls are intentionally independent of V1's Tailwind presentation and runtime state. Existing React, TypeScript, Vite, Vitest and Testing Library are reused. Dockview and CodeMirror are reused from P01's selected architecture. The V1 components and production entry points remain in place; no migration wrapper hides an unfinished workspace behind V1 navigation.

## Future integration responsibilities

P03 supplies isolated project identities and context; P04 supplies durable versioned layouts and per-view state; P05/P06 supply real files, conflicts, diagnostics, debugging, terminal and extensions; P07–P12 supply conversations, orchestration, lifecycle, memory and verified Git/review results. P13/P14 must qualify full application accessibility, localization needs, screen readers/IME, native platforms and real workflows. None of these services is simulated by a production P02 component.

## P04 production integration contracts

The original 58-component P02 inventory remains the reusable visual foundation. P04 adds production compositions and services rather than a second design system. They are not counted as 58 new primitives.

| Component / service | Actual contract | Lifetime / accessibility |
| --- | --- | --- |
| WorkspaceShell | WorkspaceModel; real projects/application metadata | Explicit project selection, navigation sessions, connection errors, scoped V1 links; keyboard commands and status announcements |
| Production DockSurface | Model, DockSession, active navigation session | Stable view containers/React portals; public Dockview creation/tab/header APIs; named tabs, context commands, focus restoration |
| WorkspaceModel | Schema-v2 document, revision, pending/conflict/recovery state | Rust CAS persistence; last activation counter; navigation never cancels work |
| DockSession | Navigation-session ID and public DockviewApi | Open/reopen exact view ID; reorder/move/split/resize/hide/maximize; preset transforms preserve drafts |
| LiveView / ViewContent | Project-owned WorkspaceView and model | View-level error boundary; own project header; unavailable and later-phase states are explicit |
| Home / Workflows / Workflow | Scoped project and optional numeric run | Real paginated summaries/tasks/attempts; no invented mission progress |
| Agents / Console | Project-local configuration and numeric AgentSession | Actual session metadata and output; named output regions and reconnect action |
| TerminalView | Explicit project plus existing interactive process-session ID | Lazy xterm; authenticated socket/input/resize; unmount disconnects client without killing process |
| ScratchNote | View state draft/selection/scroll/dirty | Lazy CodeMirror; bounded 65,536-character workspace note, accessible editor, retained draft on close; no file write |
| ResourceCache | Complete project-prefixed API path including query | Deduplicated reads, abort last reader, bounded inactive entries; no active-project global |
| SessionStreams | Project ID plus numeric process session | Ref-counted EventSource, stale-source guard, reconnect status; no execution ownership |

Shared `WorkspaceTab` now accepts an optional `closeLabel` so navigation hiding is explicit. Shared Menu uses an exact action label and separate `aria-keyshortcuts`, while retaining visible shortcut text. Shared Dockview semantic CSS lives in `workspace/shell/dock-theme.css`. The preview consumes it without importing production state. Future editor/diff/diagnostics/extensions contracts exist but their production creation actions are disabled. [Navigation](navigation-contracts.md), [architecture](workspace-architecture.md) and delivery (historical record held privately) contain the actual integration and qualification limits.

## P05 production file components

| Component/service | Contract and states | Accessibility/integration |
| --- | --- | --- |
| `ExplorerPanel` | Backend root catalog, lazy paged directories, bounded/virtualized visible rows, CRUD dialogs, recovery | Tree keyboard navigation, context menu, explicit delete/purge confirmation; real API state |
| `EditorPanel` / `CodeSurface` | Real `FileRef` + recovery document ID; loading, dirty, read-only, recovered, missing, conflicted, failed-save states | CodeMirror native keyboard editing, Save/Save All, confirmation dialogs, themed source surface |
| `DiffPanel` | Exact base/disk/HEAD revision versus current buffer | Both sides read only; source selector and version labels; no staging/publication |
| `FileSearchPanel` | Root, literal/regex content query, path filter, case/hidden options, complete/limited/cancelled results | Keyboard form/results, line navigation, explicit cancellation |
| `DocumentManager` | Shared `Text`, independent panes, revision-aware save, generation guards, recoveries | Separate from React/Dockview lifecycle; project/root keys |
| `FileApi` | Typed immutable project-scoped requests | Existing authentication/CSRF/deadline handling |
| `BrowserDraftJournal` | Versioned, bounded IndexedDB drafts with distinct copied-window ownership | Storage errors retain drafts and produce actionable alerts |
| Language/theme adapters | Lazy bundled Rust/TS/JS/Python/JSON/HTML/CSS/Markdown/TOML/YAML highlighting | Existing semantic tokens and local fonts; plain-text fallback |

These are production P05 implementations. Prototype fixture components remain confined to the P02 preview. LSP/DAP/extension and full Source Control contributions remain later-phase boundaries. A dirty repository-tab close now offers Save, Discard, Keep draft or Cancel; workspace notes keep P04 navigation-only close semantics.

## P06 production IDE components

| Component/service | Actual contract | Accessibility/lifecycle |
| --- | --- | --- |
| `EditorIntelligence` | Actual LSP capabilities, approved server selection, navigation, symbols, rename/format/code-action review | Disabled unsupported commands; F2/F12/Shift-F12/Alt-Shift-F and keyboard context menu; edits stay in P05 buffers |
| `LanguageManager` / `LanguageConnection` | Project/root/client-scoped official CodeMirror transport, versions, cancellation and diagnostics | Shared document synchronization survives pane moves; stopped servers do not disable editing |
| `DiagnosticsPanel` | Real authorized document diagnostics and source/version provenance | Navigable file/line results; unversioned freshness limitation visible |
| `DebugPanel` / `DebugManager` | Confirmed stored launches, threads, breakpoints, stepping, stack, variables, evaluation and output | Explicit Stop; named controls; source navigation through P05; confirmed expression evaluation |
| `TerminalPanel` | Approved shell/root, actual terminal IDs, rename, split view, reconnect, restart and Stop | New terminal confirmation; tab close is presentation-only |
| Shared `TerminalView` | Discriminated agent-console or development-terminal identity | Existing xterm/fit plus search, copy/paste/clear, screen-reader mode; input waits for live socket |
| `ExtensionsPanel` / `ExtensionView` | Local trusted declarative package import, permissions, activation/preferences, host editor and Dockview contributions | Confirmation before install/uninstall; disabled/removed contribution views explain their state |
| `IdeSettings` | Validated profile and launch JSON approval; actual installed tools/configurations | Explicit process trust statement; no automatic install or execution |

All use the P02 semantic themes and P04 docking/navigation, without a new styling framework. P07 chat, P10 suspension and P12 Source Control remain separate. P06 supersedes the earlier diagnostics/extensions boundary statements; no VS Code compatibility is claimed.

## P07 production conversation compositions

| Component/service | Contract | Accessibility/lifecycle |
| --- | --- | --- |
| `ConversationPanel` / `ConversationContent` | Project-owned persisted conversation; paged history, metadata CAS, archive, composer and context | Named conversation selector, Enter/Shift-Enter, Alt-C focus, status announcements, retained failed submissions |
| `MessageRow` | Real persisted role/content/status/provenance; mission/session/file links | Inert model HTML/URLs, readable fenced code, no hidden reasoning or invented activity |
| `MissionForm` | Existing recipe/team choices and actual runtime fields; explicit authored Plan mode | Labelled controls, DAG validation, task-list dependency editing, graph as a second representation of the same plan |
| `ConversationProgress` | Real linked run/task/session/attempt/artifact/delivery records | Separate from message history; explicit scoped cancellation, task counts, expandable evidence, V1 detail links |
| `ConversationStore` / `ConversationApi` | Immutable project/conversation scope; durable SSE cursor and bounded 200-message snapshot | Ref-counted subscription, deduplication, reconnect and reload; no backend execution ownership |
| `DocumentManager.contextSelection` | Current focused or last-focused shared P05 pane selection | No second document buffer; preserves project/root/revision and unsaved content |

The existing Textarea now forwards its ref for composer focus without changing its state contract. All new compositions reuse P02 controls and semantic tokens, CodeMirror and Dockview; no new frontend package is added. A final structured agent response has a processing state until the CLI returns. It is not presented as incremental token output. Conversation history is real project data; controlled agents belong only to isolated qualification fixtures. P08 scheduling, P09 profiles, P10 suspension and P12 complete review remain later-phase work.

Read-only mission snapshot references expose the same project-scoped progress/review links without assigning run ownership to the inspecting command. Default Progress panels follow conversation selection within their navigation session; explicitly selected run views stay fixed. Context selectors and reference labels include a root identity suffix, and persisted references expose read-only status, so equal relative paths in different worktrees are distinguishable.

P07-R1 adds `AttemptEvidence({ attempt: TaskAttempt })` to workflow detail. Its native disclosure identifies task/attempt/status and lazily renders the persisted review decision, task commit, error and captured patch as inert read-only text. Empty evidence is explicitly absent; primary working-tree content is identified as a separate version. The patch has keyboard focus and bounded scrolling. The component makes no request or mutation and reuses the existing scoped `RunDetail` response. Two tests verify lazy rendering, absent evidence/review and HTML-looking patch text.

## P08 additions

Production Mission Progress now includes actual concurrent task stages, immutable revision history, Planner session navigation and integration publication/conflict records. Attempt Evidence exposes bounded Rust-captured Git facts and patches. These reuse existing disclosure controls, compact lists, typography, themes and project-scoped navigation; no second workspace or component system was added.


## P13 production additions

`product/SettingsPanel` provides scoped searchable settings over existing authorities. `product/CommandPalette` and `product/commands` replace duplicated workspace shortcut metadata and use the existing native P02 modal. `product/Notifications` provides the durable project center, shared arrival observer and generic opt-in OS payload. `product/preferences` consumes acknowledged Rust preferences through the existing resource cache. No new palette, typography scale or competing workspace is introduced. Their actual backend routes, loading/error/conflict states and tests are listed in [the product audit](p13-product-audit.md) and [test matrix](p13-test-matrix.md). Historical phase boundaries above describe their delivery date; current consolidated state is in the P13 delivery/qualification ledger.
