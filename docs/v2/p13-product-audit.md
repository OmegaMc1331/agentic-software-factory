# P13 product audit

Baseline: clean `the historical private baseline`, branch `codex/v2-p13-product-finalization`. This initial inventory is based on the production entry, component implementations, backend routes and P03–P12 reports. Browser observations, measurements and final dispositions are recorded below and in the delivery evidence; implementation inspection is distinct from accessibility certification.

## Product boundaries

`workspace.html` is production; `v2.html` and `prototypes/fixtures` are isolated design previews. V1 remains the default route. Dockview owns layout, P05 owns documents, Rust owns projects/policy/processes, and each existing subsystem retains its persistence. P10 stays manual-resume with the existing 180-second lease and 60-second grace. No external publication or P14 work is authorized.

## Surface inventory

All production views use the P02 semantic dark/light tokens. P04 stores project/session/view identities and layout through Rust CAS. A view's own project identity, including a pinned view, is its request scope. The table records exceptions and the verification still needed.

| Surface / entry | Actual authority and actions | Loading / empty / error behavior | Keyboard and accessibility | Persistence / project behavior / existing tests | Initial finding |
| --- | --- | --- | --- | --- | --- |
| Application shell / `workspace.html` | P03 registry, P04 workspace, P10 presence; select/open project, sessions, presets, docking | API/authentication status, recovery/independent copy, unavailable projects | Native project/session tabs, F6 and layout menu, native dialogs | Rust workspace CAS; project-aware cache/streams; workspace/controller/stream tests | Commands embedded in shell; no palette or notification access; crowded command bar |
| Project manager / `projects.html` | Real registration/create/clone/open/relink APIs | Trust/allowlist errors, unavailable rows, empty registry | Labelled forms and confirmations | Application registry; P03 browser/API isolation tests | First-run workspace sends user to a separate manager without concise setup guidance |
| Explorer / Activity Rail | P05 root catalog/tree, guarded file CRUD | Loading, empty, denied, missing and recovery states | Virtual tree arrows, context menu, native dialogs | Stable project/root/path, lazy tree and P05 tests | Validate long paths and focus after cross-feature navigation |
| Editor / Explorer, Search | P05 shared DocumentManager; save, conflict/recovery, diff | Loading, read-only, missing, stale/recovery/error states | CodeMirror and source shortcuts, save controls | Separate durable drafts, independent pane selections; extensive document/files tests | No centralized editor preferences; global shortcut precedence requires consolidation |
| Search / Activity Rail | P05 bounded root/path/content search | Explicit empty, partial, cancelled and failed results | Labelled search form, keyboard file/line links | Per-view query/root; files/search tests | Quick-open command missing; must reuse authorized file search |
| Orchestrator / default center | P07 messages/commands/SSE and mission form | Loading, paginated history, failed retained submission, reconnect | Enter/Shift-Enter, composer focus, bounded live announcements | Durable conversation/project identity, deduplicated stream; conversation tests | New mission/open conversation need palette entry; result deep links must preserve IDs |
| Missions / Workflows and Progress | P08 runs/claims/tasks/revisions/integration records | Empty list, loading, errors, recorded task counts | Lists/disclosures and alternative task list to graph | Scoped run IDs and view state; scheduler/API/browser tests | Terminal outcomes have no centralized durable notification |
| Agents / Activity Rail | P09 profiles/revisions/capabilities/effective policy/handoffs | Loading, unavailable provider/model, revision conflict, denied replacement | Labelled editor and handoff controls | Project-local immutable snapshots; profile/handoff tests | Settings access fragmented; retain provider availability honesty |
| Mission lifecycle / Progress and session controls | P10 leases, close intent, checkpoints, authorized manual resume | Pending/quiescing/suspended/recovery states | Explicit close confirmation, labelled resume | Durable ownership/generations; lifecycle tests | Settings text incorrectly says suspension arrives later; restoration must remain read-only |
| Memory / Activity Rail | P11 project SQLite/FTS/provenance/candidates/settings | Empty, loading, stale/disputed/disabled, source/revision errors | Search/forms/details/history | Project-local revisions/deletion/current invocation eligibility; memory tests | Settings embedded in panel; investigate write amplification without relaxing validation |
| Source Control / Activity Rail | P12 native Git/index/common locks; status/diff/stage/commit/branches/history | Honest unsupported hunks, stale/busy/denied states | Native actions, CodeMirror comparison | Observed Git revisions, primary writable root only; Git/API/browser tests | Repeated read configuration/subprocess and refresh costs need profiling |
| Review / preset and mission links | P08/P12 attempts/patches/reviews/journals/human anchors | Missing evidence, permission denial, historical/current conflicts | Read-only diff, comments/disclosures, source navigation | Original sessions/commits and versioned comments; real Reviewer evidence | Verify review-to-source and notification-to-run identity; preserve automated approval authority |
| GitHub / Source Control | Existing authenticated gh/provider/delivery coordinator | Unknown/unavailable checks, explicit read refresh, preview/consent errors | Labelled previews/actions | Actual remote ID and durable delivery journal; provider contract tests | Live writes unqualified and unauthorized for this phase; no automatic remote reads on every UI action |
| Development terminal / Activity Rail | P06 approved shell/root and owned terminal ID | No process, connected/disconnected/exited, explicit reconnect | xterm keyboard and screen-reader mode | Process lifetime independent of view; IDE tests | Preserve terminal keys; settings must reuse approved profiles rather than execute commands |
| Agent console / task/session links | Existing scoped AgentSession stream/terminal | Recorded output, reconnect/error | Named output regions | Ref-counted SSE, no execution on restore; stream tests | Distinguish recorded completion from live activity |
| Diagnostics/debug / views and source actions | P06 actual language/debug services | Missing tool, unavailable capability, stale diagnostics | Source navigation, reviewed edits, explicit launch/stop | Approved profiles/launches; eight opt-in native tests | Keep tool setup accessible from Settings; native AT unqualified |
| Extensions / Activity Rail | P06 trusted declarative package service | Empty/unavailable/disabled/failed activation | Native import/trust controls | Actual grants; restoration never grants/activates new capabilities | Settings entry should navigate to this authority; no new extension API |
| Settings / Activity Rail | Workspace theme plus appended P06 IDE process editor; V1 project-config link | Theme applies locally; IDE reads/errors are real | Native forms; static shortcut prose | Theme duplicated between browser preference and workspace JSON | Missing centralized searchable categories, application CAS preferences and effective configuration overview |
| Notifications / absent | P02 NotificationItem exists only as a presentation primitive | No production durable center | No global entry or event-backed announcements | No production notification persistence/tests | Implement backend-derived deduplicated records, read/dismiss state, bounded history and safe targets |
| Command palette / absent | Shell hardcoded shortcuts; separate P06 extension keys | No search/context-disabled states | Static help can drift from handlers | No single command metadata registry | Implement typed commands over existing authorized handlers; detect conflicts and retain editor/terminal precedence |
| Status bar / bottom shell | Project label, active view/session and layout save state | Accurate saved/pending/error text | Existing live region | Derived shell state | Add useful authoritative connection/unread metadata without fabricated readiness |

## Initial corrective priorities

1. Centralize application presentation preferences in the existing application workspace store with typed validation/CAS; navigate project/agent/tool/memory/Git settings to their existing owners. No second policy store.
2. Materialize bounded project notifications from actual persisted runtime events, with immutable event provenance and independently persisted read/dismiss state. Never persist raw agent logs or accept browser-fabricated mission events.
3. Replace duplicated global keyboard knowledge with one typed command map, searchable palette/reference and context-disabled explanations. Existing actions continue through the same backend boundaries.
4. Correct obsolete phase/boundary wording, first-run guidance, deep-link identity and focus/overflow inconsistencies. Keep previews separate.
5. Profile controlled memory writes and native Git paths before optimizing. Preserve source validation, secrets screening, revisions, index locks, policy and worktree ownership.
6. Exercise real product journeys, automated accessibility, keyboard-only flows, both themes/five desktop sizes, resource cycles and full regressions. Explicitly distinguish controlled fixtures, retained genuine evidence, new live model evidence and unavailable native/platform/remote qualification.

## Inherited qualification ledger

P00–P12 matching remote CI remains unverified. Only Windows has native local qualification. Live GitHub writes and cross-provider execution remain unqualified. P11 insertion debt and P12 unoptimized/concurrent-load timings are historical observations, not controlled P13 baselines. Native screen-reader/IME qualification remains open. These are not silently cleared by this audit.

## Final disposition

The following changes address the initial findings. Exact execution identities and retained failures are recorded in the delivery (historical record held privately) and the evidence manifest (historical record held privately).

| Finding | Final disposition |
| --- | --- |
| Fragmented settings and duplicate theme authority | Centralized searchable scoped Settings uses revisioned application preferences in the existing WorkspaceStore. Project, profile, tool, memory and Git authorities remain unchanged. Both project-manager and workspace theme controls use the acknowledged preference. |
| No production event history | Project-owned durable notifications derive from actual persisted mission, handoff, lifecycle, integration and delivery transitions, plus typed refused-save/service failures. Read/dismiss state, retention, deduplication and privacy are tested. |
| Duplicated shortcut knowledge and no quick-open | One typed command registry drives palette, keyboard reference and exact-modifier dispatch. Context-disabled reasons and terminal/editor precedence remain explicit. Keyboard acceptance exposed initial focus on the palette close button; search now receives focus after the native dialog opens. |
| Difficult first-run configuration | The empty production workspace opens real project management and global Settings. Existing users restore acknowledged state. New mission creates a scoped conversation but submits no objective without the explicit action. |
| Reentrant/stale deep links | Notification navigation resolves the original project/run, fences superseded callbacks, opens once and acknowledges only after success. Mission links reuse actual Review/Source Control resources. |
| Layout restoration pixel drift | Dockview is laid out and captured after attachment to its current container. Original P04 dimension checks pass without weakening assertions. |
| Corrupt workspace recovery became unnecessarily dirty | Application-theme mirroring/migration skips conflicted fallback documents. The original replacement confirmation is retained for actual user changes. |
| Busy project closure appeared unreliable | Only the backend's definite live-request 409 is retried, six times at bounded intervals. Unknown outcomes are never retried; no force close bypasses Rust ownership. |
| Editor font preference did not affect the actual editor | Acknowledged preferences reconfigure the existing compartment with correct theme/CSS precedence. Browser checks inspect computed font size and independently verify saved disk content. |
| Large mission objectives overwhelmed lists | Lists and headings use a bounded explicit preview; native disclosure retains complete original context. Historical evidence is not rewritten. |
| Inconsistent spacing and target size | Invalid token references were replaced by P02 spacing/type tokens. Checkbox targets are 24 pixels. Both themes and five desktop widths are checked for overflow. |
| P11 insertion debt | Query-plan inspection led to indexed FTS/duplicate improvements with migration, provenance, revision, deletion and rebuild regressions. Controlled gains and slower samples remain separate; release-scale qualification is open. |
| P12 Git cost | Adaptive bounded attribute-check batches retain complete path/filter validation, index locks and stale/worktree safeguards. Native late-batch denial and exact-index commits are tested. |
| Prototype remnants / misleading statuses | Obsolete future-phase copy and static shortcut prose were removed. Previews/fixtures remain isolated. Unsupported tools, publication and native qualification remain explicitly unavailable or pending. |

Remaining limits are native AT/IME, Linux/macOS, matching remote CI, cross-provider live qualification, authorized live GitHub writes, release-profile performance and long-duration resource budgets. These remain in the [qualification ledger](v2-release-qualification-ledger.md); P13 does not perform P14 or change the release NO-GO.
