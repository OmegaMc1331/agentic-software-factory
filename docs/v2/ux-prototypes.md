# Executable UX prototypes — P02

The preview is a separate Vite entry at `v2.html`. The production `App.tsx`, `main.tsx`, `index.css` and V1 navigation remain unchanged. The preview never contacts the Rust API or starts agents, terminals, language servers or debug adapters. A persistent banner says **Fixture preview · no runtime**. Browser storage is namespaced `asf.v2.p02.*`.

## Run

From `apps/dashboard`:

```powershell
npm ci
npm run dev:v2
```

Open `http://127.0.0.1:5182/v2.html`. For validation against built assets, stop the development server before using the same strict port:

```powershell
npm run build:v2
npm run preview:v2
```

The output directory is `.factory/v2/p02/dist`, separate from the V1 `apps/dashboard/dist`. No existing release workflow is changed. Node is development tooling; the V2 assets are static and do not add a Node backend or final-user toolchain requirement. Production embedding remains a later integration task.

Run executable browser checks while the built preview is serving:

```powershell
npm run test:v2:browser
```

At repository root:

```powershell
node scripts/v2_p02/benchmark.mjs
node scripts/v2_p02/validate.mjs frontend
node scripts/v2_p02/validate.mjs rust
node scripts/v2_p02/validate.mjs compatibility
```

Browser tooling uses the installed Edge channel through pinned Playwright. It does not download a browser. A runner without Edge must provision a compatible browser before claiming a pass. `ASF_P02_URL` can point the harness at another local preview URL. Tests write raw results and screenshots beneath `.factory/v2/p02`; retained delivery evidence is in evidence/p02 (historical record held privately). Build first, wait for completion, then run browser checks; rebuilding while a browser loads hashed assets invalidates measurements.

## Scenario matrix

| UX | URL / selection | Demonstrated content and interaction |
| --- | --- | --- |
| UX-01 | `?scenario=empty` | Small empty project/conversation state; composer and explore-code action |
| UX-02 | `?scenario=conversation` | Realistic user/orchestrator fixture messages, file references, result preview, draft, local submission |
| UX-03 | `?scenario=running` | Supplied mission title/status/current task, 2 of 5 completed tasks, task details, local pause/resume |
| UX-04 | `?scenario=attention` | Preserved checkpoint, unavailable agent, explicit replacement decision, permission-limited reassignment dialog |
| UX-05 | `?scenario=code` | CodeMirror editing, Rust/TypeScript/Python syntax, explorer, multiple files, synchronized split, output |
| UX-06 | `?scenario=review` | Actual CodeMirror merge of fixture before/after text, read-only diff, change navigation, test records, local acceptance; Publish disabled |
| UX-07 | Theme → Light | Same controls, presets, code/diff/output/overlays; saved preference |
| UX-08 | Theme → Dark | Same scenarios and semantic structure; no inversion filter |
| UX-09 | Resize to 1920/1440/1280/1024 px | Graceful grouping of secondary panels; editor/conversation remain reachable |
| UX-10 | `?scenario=long` | 2006 files, long path/title, 120 messages, expandable large body, many tabs and 1800 recorded output lines |

`?gallery=components` opens the interactive component gallery. It has the same tokens, theme provider and typography. Every enabled control updates local state, opens a real overlay or emits a visible local intention. Debug controls manipulate an explicitly marked gallery state; they do not simulate a connected adapter. The failure action deliberately throws through a real error boundary, then recovery restores that view.

## Interaction map

Theme selection supports system preference, manual override and persistence. Project tabs select Atlas/Fieldnotes fixture keys; they do not create runtimes. Presets reconstruct Dockview layouts while preserving local buffers/drafts. Real Dockview drag/drop, tab selection, resizing and splitting operate on actual panels. Top actions reorder the active tab. Views can hide/reopen tools and resize an active panel through numeric controls. F6 changes groups; Delete closes a focused dock tab. Ctrl/Mod+K opens file/preset search. Search arrows/Enter select; Escape clears then closes. Ctrl/Mod+Enter records a composer request. Ctrl/Mod+S in an editor saves only browser preview state.

File trees filter/collapse and provide pointer/Shift+F10 context actions. Two views of one file synchronize text; different files retain their own buffers. CodeMirror undo/search/selection work locally. Save preview persists text, never writes the repository. Dialogs trap focus and return it on close. Menus and tooltips work from keyboard. Notifications dismiss individually. Preferences set compact density and offer confirmed preset recovery. Invalid stored data and unavailable storage produce visible explanations.

## What is real and what is supplied

Real: React controls, local state, theme/media handling, semantic styles/fonts, native dialogs/menus/focus, CodeMirror editor/history/search, CodeMirror merge computation, Dockview tree/movement/size/serialization, windowed explorer, local storage and measured browser rendering.

Supplied fixtures: project tree and file contents, conversation history, mission/task states, agent profiles/availability/permissions, checkpoint identity/commit, Git origin/line counts, test names/durations/results and terminal recording. These are never counted as tests executed during P02. Local review/agent/mission actions change preview state and say so. No shell, Git write, publication, extension activation or agent process is launched. P01's real protocol experiments remain separate and are not relabeled P02 production features.

## Visual evidence and review

Representative retained captures:

| State | Light | Dark |
| --- | --- | --- |
| Conversation / Focus | Light (historical record held privately) | Dark (historical record held privately) |
| Code | Light (historical record held privately) | Dark (historical record held privately) |
| Agents | Light (historical record held privately) | Dark (historical record held privately) |
| Review | Light (historical record held privately) | Dark (historical record held privately) |
| Empty | Light (historical record held privately) | Dark (historical record held privately) |
| Running mission | Light (historical record held privately) | Dark (historical record held privately) |
| Attention | Light (historical record held privately) | Dark (historical record held privately) |
| Long conversation | Light (historical record held privately) | Dark (historical record held privately) |
| Narrow Code | Light (historical record held privately) | Dark (historical record held privately) |
| Gallery | Light (historical record held privately) | Dark (historical record held privately) |

Additional captures cover compact density, long code/tabs/output, dialogs, keyboard context menu, 150% text and 200%-equivalent reflow. They are browser captures, not AI-generated mockups or screenshot baselines with platform-dependent pixel thresholds. Geometric assertions and deterministic fixtures provide stable regression checks.

Visual and keyboard review found and corrected: two simultaneously selected file rows; redundant contextual headings; a misleading static session chevron; Tab wrapping through the dialog boundary; nested close controls in tabs; unfocusable read-only diffs; temporary low contrast during theme interpolation; unsupported accessible names on generic indicators; and oversized Explorer in narrow grouped presets. See the delivery for final audit results and raw evidence. Remaining limits concern full product integration and platform qualification, not hidden prototype backend behavior.

## Accessibility, motion and performance limits

The browser harness exercises real Tab/Shift+Tab/Escape, menu navigation, tooltips, dialogs, search, context actions, roving toolbars, splitter keys, reduced motion and persistence. WCAG-tagged axe checks retain violations and incomplete findings; semantic foreground/background pairs are measured independently. Browser AX tree checks verify that names and roles reach the accessibility tree. This is not an NVDA/VoiceOver or physical-input certification.

The 720 CSS px/DPR 2 test is a 200%-equivalent reflow simulation, not native browser or OS zoom. A separate 150% root-text test checks scaling. Linux/macOS/Safari/Firefox qualification remains pending. Font and browser differences are not masked with fragile screenshot comparisons.

Performance results use built assets with five browser-cache-cold contexts and warm reloads per normal/long fixture, no throttling, and no simultaneous Rust regression run. Actions include two animation frames; automation-mediated pointer/theme timings include transport overhead and are not INP. Layout timing does not include a first lazy editor download; resource timings retain it separately. Source bundle assets and actual loaded resource names are recorded. Production large-repository indexing, live terminal throughput, thousands of open editors and streaming conversations require later service integration and profiling.
