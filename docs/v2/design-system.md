# ASF V2 design system

P02 establishes reusable frontend foundations. The [product specification](product-spec.md) remains normative. The implementation is in `apps/dashboard/src/v2/design-system`; the V1 entry point does not import it. The executable preview uses fixture data and no runtime API. See validation and delivery (historical record held privately) for qualification boundaries.

## Visual decisions

The workspace uses flat surfaces, one-pixel separators, compact controls, restrained blue selection and typography to establish hierarchy. The current conversation, editor or diff occupies the largest area. Progress is a short contextual view with task details on request. Agent identity is text, role and availability. Icons communicate actions and states; there are no agent avatars, decorative metrics, gradients or marketing sections.

A panel has one dock tab and a contextual header when its content needs a project, session or execution label. Tool output is kept outside the conversation. Menus and dialogs are the only elevated surfaces. Dense technical content has internal scrolling; the page does not become a horizontal canvas.

## Token implementation

CSS custom properties have three layers. `tokens/primitives.css` contains the raw palette and scales; `tokens/semantic.css` maps meaning to each theme; `tokens/components.css` defines shared component aliases. Components consume semantic or component properties. New raw colors belong in the palette, not component CSS or JSX. The standalone favicon is an asset, not a themed control.

| Semantic token | Light | Dark | Use |
| --- | --- | --- | --- |
| `--color-canvas` | `#fafbfc` | `#111318` | Application and terminal background |
| `--color-panel` | `#ffffff` | `#15181e` | Working surface, editor |
| `--color-panel-muted` | `#f3f5f8` | `#191d24` | Toolbars and inactive tabs |
| `--color-text` | `#1b2430` | `#e9edf2` | Primary content |
| `--color-text-secondary` | `#2d3848` | `#c9d2de` | Secondary content |
| `--color-text-muted` | `#566477` | `#a2adbc` | Metadata and quiet actions |
| `--color-border` | `#d7dde5` | `#343d4a` | Nonessential surface separation |
| `--color-control-border` | `#818d9e` | `#68778b` | Input and control boundaries |
| `--color-accent` | `#2657a4` | `#89aff3` | Primary actions, links, focus context |
| `--color-focus` | `#326ac7` | `#89aff3` | Visible keyboard outline |
| `--color-selection` | `#e9f0fe` | `#243650` | Selected row or tab |
| `--color-selection-text` | `#2657a4` | `#bdd4ff` | Selected content |
| `--color-success` | `#246d4b` | `#79c6a1` | Verified or supplied successful state |
| `--color-warning` | `#80571c` | `#dfbd82` | Attention or uncertainty |
| `--color-danger` | `#a93c3c` | `#efa3a3` | Errors and destructive confirmation |

State foregrounds have separate background tokens. Disabled controls use their own background/text tokens and native disabled semantics. Subtle separators are not used as the sole boundary of a form field. Information uses the blue state pair. Code has independent comment, keyword, string, number and type tokens. Terminal, added diff and removed diff pairs are semantic, with the same palette in application and editor.

## Typography and density

Inter Variable 5.3.0 is the UI font; JetBrains Mono Variable 5.3.0 is used for code, logs, shortcuts and technical values. Both are SIL OFL-1.1 and self-hosted as Latin variable WOFF2 subsets, including French accents. They total about 89 kB before transport compression; exact asset sizes are recorded in the performance evidence. There is no runtime font CDN. Segoe UI/system-ui and Consolas/monospace are fallbacks; other scripts use system fallback rather than claiming coverage from the Latin subsets.

| Role | Size at 16 px root | Weight / line height |
| --- | ---: | --- |
| Metadata | 11–12 px | 400 / 1.45 |
| Interface, panel titles | 13 px | 400, 500 or 600 / 1.45 |
| Body | 14 px | 400 / 1.45 |
| Conversation | 15 px | 400 / 1.7 |
| Section heading | 18 px | 600 / 1.45 |
| Code | 12–13 px | 400 / editor line height |

There is no systematic weight 700. Values use rem units so text scaling propagates. Default controls are 32 px high and tree rows 30 px; compact density uses 28 and 26 px. The 4/8/12/16/20/24/32 px spacing scale is shared. Radii are 3 px for controls, 5 px for overlays, 8 px for the composer. Panel geometry is documented in the [workspace specification](workspace-visual-spec.md).

## Theme ownership

`bootstrap.ts` reads `asf.v2.p02.theme` and resolves `system | light | dark` before loading React content. `ThemeProvider` owns preference and `matchMedia` updates. Explicit selection overrides system changes. Storage denial applies the theme for the session and presents an explanation. Theme changes suppress interpolated control colors briefly so light-theme text is not animated over dark surfaces. This suppresses theme transitions, not useful hover or overlay feedback.

The root `data-asf-theme` selects tokens. Dockview receives a custom theme class with documented CSS variables. CodeMirror uses an appearance compartment for its light/dark flag, semantic `EditorView.theme` rules and semantic `HighlightStyle` colors. Theme switching retains the document, selection and undo state; it does not recreate the editor. Merge views and output surfaces use the same tokens. No inversion filter or V1 global theme overrides are used.

P02 persistence is browser-local and namespaced. Production preferences and workspace state require Rust-owned versioned contracts during later phases; this preview storage is not a project database migration.

## Controls and interaction

The [component inventory](component-inventory.md) specifies props and obligations. Native form semantics are retained for buttons, selects, checkboxes and radios. Field names are separate from hint/error descriptions. Required or invalid state must be supplied explicitly; components do not infer domain validity. Busy buttons disable repeat activation and expose `aria-busy`.

Tabs use one selected focus stop and arrow/Home/End navigation. Toolbars use roving enabled button focus. Menus skip disabled entries, support arrows/Home/End/typeahead, close on Escape/outside pointer and return focus. Context menus support pointer and Shift+F10. Tooltips appear after 300 ms of intent and dismiss on Escape. A popover is nonmodal, has a name and explicit close action, and returns focus.

Dialogs use native `showModal`, explicit focus wrapping, a visible close control, Escape cancellation, backdrop dismissal and focus restoration. Confirmation has distinct cancellation and confirmation callbacks. Focus trapping includes controls contributed by nested overlays. Confirmation never invokes the dangerous callback during dismissal.

Dockview's custom public tab renderer is text-only. Close actions live in the group header, outside the tab, avoiding nested interactive roles. Delete closes the focused tab. Reordering buttons provide an alternative to drag, and Views → Resize active panel provides numeric width/height controls as an alternative to dragging a sash. F6 cycles groups. Closed views reopen from Views or the activity rail.

## Motion and layers

Feedback uses 120 ms, menu/popover entry 180 ms, and small panel-related transitions 220 ms with `cubic-bezier(0.16, 1, 0.3, 1)`. Dock layout updates remain under Dockview's control; large editor surfaces are not interpolated during typing or resize. No idle animation runs in a working pane. A busy indicator rotates only while the supplied state is busy.

`prefers-reduced-motion: reduce` sets durations to zero and disables animations. Layer tokens are base 0, sticky 10, menus 30, notifications 40, dialogs 50 and tooltips 60. Modal dialogs use the native top layer. Nested menus/popovers are portaled into the active dialog; other overlays use the document body. Popup coordinates are clamped to the viewport.

## Accessibility and verification

The target is WCAG 2.2 AA for applicable interfaces. State is communicated through text and icons as well as color. Focus uses a two-pixel outline with offset; tree focus is distinct from file selection. Virtual tree rows expose level, position and sibling count, keep the active descendant mounted and support arrow/Home/End navigation. Search exposes a combobox/listbox relationship. Read-only diffs are keyboard focusable. Errors have named recovery controls; notifications have individual dismissal labels.

Automated checks exercise WCAG A/AA tags, semantic contrast pairs and actual browser keyboard behavior. The retained evidence includes incomplete axe findings for manual assessment. Passing axe is not a screen-reader certification. Native Windows screen-reader, IME, physical keyboard and macOS/Linux browser qualification remain pending. CSS/DPR and text-scaling tests are explicitly distinguished from native OS zoom. The screenshots and delivery report record visible defects found and corrected.

## Scope and future use

The 58 exported reusable components/providers are ready for local reuse under their typed contracts; their integration into production screens is future work. Workspace compositions are prototype-only. Mission and review components display supplied domain records with provenance; the preview never treats fixture tests as executed evidence. The output view is a read-only recording, not a terminal. LSP, DAP, filesystem writes, agents, Git publication and multi-project runtime remain outside P02.

Reference checks, 2026-10-27: [WAI modal dialog pattern](https://www.w3.org/WAI/ARIA/apg/patterns/dialog-modal/), [CodeMirror reference](https://codemirror.net/docs/ref/) (web fetch returned 403; API signatures were verified in pinned official package declarations), [Dockview](https://github.com/mathuo/dockview) (public composition APIs verified in pinned package declarations), [Fontsource](https://github.com/fontsource/fontsource). Package and security evidence is retained with the delivery; dependency metadata is not substituted for a demonstrated interaction.

## P05 production refinements

Dialog footers wrap their actions within the existing 32rem container; destructive-choice dialogs must keep Cancel and every recovery/save action visible. CodeMirror search controls use semantic surfaces without the upstream light button gradient, with 26px action targets and reserved close-button space. Read-only source/diff text remains keyboard focusable. Workspace focus prefers inspectable text and skips disabled controls; a clean editor's disabled Save button must not break repeated tab shortcuts. The P05 delivery retains browser evidence for these refinements.


## P13 production consolidation

The production Settings, Commands and Notifications surfaces reuse P02 semantic tokens, native labelled controls/modal focus behavior, compact lists and existing icons. There is no second color/token system. Settings adapts to docked-container width as well as viewport width; new notification content wraps within bounded panels. Existing density tokens are applied from acknowledged application preferences. Automated theme/contrast findings, keyboard tests, manually inspected screenshots and unqualified native AT remain distinguished in [the accessibility report](accessibility-report.md).
