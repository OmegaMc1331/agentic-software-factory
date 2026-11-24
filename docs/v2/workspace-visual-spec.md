# Workspace visual and interaction specification — P02

This specification describes the implemented preview. It does not change V1 navigation or claim production project/session ownership. The selected [P01 foundation](adr-ide-foundation.md), CodeMirror plus Dockview and Rust-owned services, is unchanged.

## Structure and geometry

The preview has a 36 px scenario/theme strip above the application chrome. This strip is evaluation tooling. Application chrome consists of a 44 px project/session row, 34 px preset/layout row, 48 px activity rail and 26 px status row at the default root size. The working area fills the remaining viewport. No page-level horizontal scrollbar is used.

Explorer starts at 224 px, or 200 px below 1100 px viewport width. Context panels start at 286–370 px depending on function. Output starts at 165–170 px high. Dockview owns the split tree, sizing, drag/drop, active group, panel lifetime and serialization. It is not replaced with CSS positioning. Content renderers attach React portals to Dockview-owned containers, preserving the single React context tree.

| Preset | Primary view | Secondary arrangement at wide desktop |
| --- | --- | --- |
| Focus | Conversation | Explorer left, compact progress right; output closed |
| Code | Editable fixture buffer | Explorer left, conversation right at 340 px, output below editor at 170 px |
| Agents | Conversation | Profiles right at 370 px, mission below profiles at 275 px |
| Review | Read-only diff | Changed files left, review right at 300 px, output below diff at 165 px; conversation in central tab group |

Preset panel minima are 160 px width for Explorer, 220 px for other views, 100 px height for Output and 120 px for other views. There is no arbitrary fixed maximum in the docking tree: neighboring minima and the viewport bound resizing. The numeric resize dialog bounds requests to the viewport and lets Dockview enforce neighboring constraints. P04 must promote these experimental constraints into versioned workspace policy.

## Narrow desktop

At widths below 1280 px, the session caption and nonessential layout hint disappear. Search remains accessible. Below 1100 px, the controller reapplies the current preset once when entering the constrained range, with a visible status announcement: Code groups conversation with editor; Review groups review/conversation with diff; Agents groups progress with agents; Focus collapses progress. Views remain reachable through the Views menu and activity rail. Expanding the viewport does not silently rebuild the user's layout.

The acceptance matrix covers 1920, 1440, 1280 and 1024 px in both themes. A 720 CSS px/DPR 2 simulation and 150% root text scaling exercise constrained reflow. They are not native OS DPI qualification. Mobile product behavior is not specified. Very small widths may require sequential views; user-created arbitrary combinations can reach Dockview's minimum-size limits. Presets provide recovery.

## Tabs, focus and movement

Projects are local fixture tabs with stable identities; switching them saves and loads separate preview keys. This demonstrates presentation and browser-state separation, not a P03 multi-project runtime. Files and tool views have stable Dockview panel IDs. A file can be reopened without discarding a local buffer; splitting can show two synchronized views of one buffer. Different documents preserve their own buffer/selection/scroll data.

The native Dockview tablist owns selected state and keyboard selection. Custom text-only tab content exposes long titles via the browser title tooltip. A separate group action closes the active view. Delete closes a focused tab. Top-row move-left/right actions reorder the active tab with the public `moveTo` API; drag/drop can move tabs between groups. The split action creates another editor group using the selected file. F6 selects and focuses the next group. Views → Resize active panel offers keyboard sizing. Visible focus is distinct from selected state.

P02 does not implement full P04 per-view editor histories or navigation history. Duplicate views share the durable per-document selection/scroll snapshot; the latest view supplies that snapshot. Independent live views and shared text synchronization are demonstrated. Production group/tab identity and conflict semantics remain P04/P05 work.

## Collapse, restoration and errors

Explorer and Output have direct toggle actions. All compatible views can be hidden/reopened from Views. Presets deliberately change layout while retaining buffers, draft and fixture conversation. Preferences → Restore preset requires a confirmation describing retained state. Closing a file view has no session or runtime side effect.

The controller persists version 1 Dockview JSON, preset, selected file, editor buffers, selection offsets, scroll, conversation messages, composer draft and review choice under `asf.v2.p02.workspace.<fixture-project>.<scenario>`. Theme and density have separate preview keys. Restore validates allowed views, file identities and finite bounded selection metadata. Invalid metadata produces an explanation and a fresh preset. Dockview restoration errors preserve valid buffers. Storage denial is surfaced instead of silently reporting a save.

This is local browser storage, not durable Rust project persistence. Clearing browser data removes preview state. There is no database migration, browser lease, process suspension or crash-recovery claim. P04 will replace this ownership with backend-persisted versioned workspace contracts; P10 handles runtime lifecycle.

## Overflow rules

Long tab titles and filenames use ellipsis with full text available through title, file search or breadcrumb. Explorer has a windowed row list, expanded folders and filter. The long fixture contains 2006 files. Keyboard focus scrolls the active tree item into the rendered window and ARIA positions refer to actual siblings.

The conversation shows a page of 24 messages with explicit earlier/recent navigation; large individual messages expand on request. No hidden messages are discarded. The composer remains below the scroll region. Agent/review/progress panels scroll internally. Terminal recording uses one scrollable, focusable preformatted output region with optional wrapping. CodeMirror scrolls code rather than expanding the page. Tab overflow remains managed by Dockview.

## Theme and design boundary

All surfaces, tabs, separators, editor gutters, syntax colors, diff states, output and overlays use the same semantic tokens in light and dark. Headers use 13 px text with context metadata at 11–12 px. Conversation uses 15 px text and relaxed line height. There is no standalone dashboard or decorative card grid within a working preset.

The preview uses only documented Dockview composition, header/tab factories, events and public APIs. It does not patch Dockview internals or fork CodeMirror. No production layout migration is authorized by these prototypes. [UX evidence and run instructions](ux-prototypes.md) identify fixture actions and remaining integration responsibilities.

## P04 production workspace

`/workspace.html` applies these tokens and interactions to the actual registry. The P02 preview remains separate. Production has a 44 px project bar, 36 px session/command bar, 28 px status bar and 48 px activity rail, with flat surfaces and thin separators.

Focus starts with Orchestrator, Workflows at 232 px left and Mission progress at 270 px right. Code and Review add recorded agent output at 180 px below; existing views are retained when a preset changes. Code puts its truthful editor integration boundary in the main group, Explorer at 232 px and Orchestrator beside it. Agents exposes actual configuration and workflow task state. Review prioritizes recorded artifacts/reviews and Git integration metadata. File editing, diffs and shell services are unavailable until their assigned phases. A preset transforms arrangement while retaining all view records and drafts.

Dockview owns geometry. Group minima are 160 px for Explorer, 210 px for other views and 100 px vertically. Numeric resizing bounds requested dimensions to the viewport. Panel content scrolls internally. Tab names ellipsize with full titles; project strips scroll; technical output wraps or scrolls within its view.

Below 1280 px the session strip collapses into a selector. At 1024 px compact controls retain project/session switching, presets, Views, Layout actions and theme. Rename/hide are also in Layout actions. Optional groups can be hidden, maximized or moved through commands as well as dragging. The layout does not become a mobile dashboard.

Both themes share P02 semantic properties across CodeMirror notes, xterm, menus, dialogs, tabs and separators. Reduced motion disables nonessential transitions. Focus and selection stay distinct. Screenshots, width checks, keyboard actions and manual review are in P04 delivery (historical record held privately).
