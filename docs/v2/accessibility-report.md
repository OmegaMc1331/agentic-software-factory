# Accessibility report

P13 targets applicable WCAG 2.2 AA behavior using the existing P02 design system, native controls/dialogs, CodeMirror, xterm and Dockview. This report distinguishes code inspection, automated browser checks, keyboard journeys, manual visual inspection and native assistive-technology qualification. It is not a blanket conformance certificate.

## Implementation

- One searchable command registry exposes supported actions, disabled explanations and actual shortcuts. Exact modifiers and focus context preserve editor, terminal, browser and composing input.
- The command palette has a labelled combobox, controlled listbox/active option, keyboard selection and focus restoration through the existing modal primitive. Async command errors reach the application error state.
- Search receives initial focus after the native dialog opens. A keyboard-only journey caught the previous close-button focus; the component regression and production browser journey retain that correction.
- Settings provides labelled controls, searchable scoped categories, explicit save/validation/revision-conflict feedback and retained rejected drafts. Category selection focuses the actual content control.
- Notifications use readable severity/status text, durable history and a concise live announcement/toast; token/heartbeat traffic is not announced. Counts unavailable from the backend are unknown, not fabricated zero.
- Existing view actions expose keyboard move/resize/dock alternatives. Restored resources and preserved editor buffers retain their original permissions and content.
- Both themes use existing semantic tokens and visible focus; reduced-motion styles remain active. New settings/notification layouts wrap long labels and adapt to narrow docked panels without global horizontal scrolling.

## Validation method

`scripts/v2_p13/browser.mjs` loads the production embedded application, invokes keyboard palette/focus restoration, quick-open/edit/save, real Source Control staging and scoped notification navigation. It captures 1920×1080, 1600×900, 1440×900, 1280×800 and 1024×768 in both themes, with reduced motion. Axe runs applicable WCAG tags against actual Settings, Notifications and palette views. Raw violations, incomplete checks, passes and screenshot hashes are retained. A generated screenshot is not considered manually reviewed until the delivery identifies the inspected images and dispositions.

Existing P02 dialog/menu tests and P03–P12 browser regressions cover additional trees, editor/diff, forms and docking. They are regression evidence, not substitutes for a complete AT audit. The final delivery records executed checks, corrected findings and remaining limitations.

The P13 harness also performs a keyboard-only project switch, file search, editor modification/save, settings search/save, conversation draft and notification-to-mission navigation. It uses Tab, native shortcuts, typing, arrows and Enter, without mouse clicks or programmatic focus to reach those controls. Dialog focus and asynchronous rendering are awaited. Final evidence records the actual outcome.

Final build 10 passed all 11 product journeys and all 104 retained browser regressions. Its 12 Axe snapshots report zero violations while preserving incomplete checks. Twenty-five selected images were individually inspected, including both themes, narrow Settings, actual source hunks, genuine Reviewer evidence, keyboard completion, native terminal/extension states and memory provenance. The explicit image list and dispositions are in `evidence/p13/visual-review.json`; other generated images are not silently counted as manually reviewed.

Twelve Axe snapshots cover Settings in both themes, notifications, palette, Explorer, Agents, Memory, Source Control, Extensions, Progress, Orchestrator and terminal. Incomplete checks are retained by ID and target, including contrast, label/name matching and selected third-party ARIA checks. Zero reported violations applies to those snapshots only. Staged Git content and saved preferences are independently verified beyond screenshot appearance.

## Native assistive technology and limits

Read-only host inspection found Windows Narrator present; NVDA was not in PATH or its standard installation locations. No native screen-reader spoken-output journey has been qualified in this phase so far. Axe cannot establish NVDA/Narrator behavior, physical keyboard/IME correctness, terminal speech usability or OS focus behavior. These require real native AT acceptance during P14. Automated contrast only covers rendered/tested states; diagrams, dynamic code content, terminal output and third-party Dockview interactions retain their explicit qualification limits.

Do not interpret zero automated violations as universal WCAG compliance. Keep native AT, Linux/macOS and final integrated release-source checks in the qualification ledger.
