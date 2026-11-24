# P05 editor verification matrix

The executable sources of truth are `factory-files/src/tests.rs`, `factory-api/tests/files.rs`, `src/v2/files/*.test.ts`, the unchanged P03/P04 integration suites, and `scripts/v2_p05/browser.mjs`. Final commands/counts, screenshots and qualification limits are recorded in the delivery (historical record held privately) and `evidence/p05/manifest.json`. Browser inputs are controlled **real repositories and disk files**, not production UI fixtures.

## Filesystem security

| Invariant | Automated evidence | Qualification |
| --- | --- | --- |
| FS-SEC-01 parent traversal | `fs_sec_01_02_06_07_08_paths_are_strict_and_internals_are_protected`; HTTP `fs_sec_01_02_07_08_20…` | Native filesystem + scoped API |
| FS-SEC-02 absolute escape | Same tests, absolute/drive-qualified paths | Native + HTTP |
| FS-SEC-03 intermediate symlink | Unix symlink test; Windows conditional symbolic-link test; native Windows junction case | Windows symbolic-link creation unavailable without privilege; Unix native qualification pending |
| FS-SEC-04 leaf symlink | Same conditional Windows/Unix symbolic-link tests | Not claimed natively exercised on this Windows host when creation is refused |
| FS-SEC-05 Windows junction | `fs_sec_05_windows_junction_is_rejected` | Real `mklink /J`, read/create refusal, outside content preserved |
| FS-SEC-06 ADS | Strict-path test includes `src/main.rs:stream` | Windows-compatible rejection on all platforms |
| FS-SEC-07 `.git` protection | Strict-path + HTTP tests | Protected reads/mutations refused |
| FS-SEC-08 `.factory` protection | Strict-path + HTTP + browser restricted operation | Runtime/database internals preserved |
| FS-SEC-09 cross-project reads | `fs_sec_09_10_19…`; `file_api_exact_writes_and_project_root_collisions_remain_isolated` | Two real roots and scoped HTTP |
| FS-SEC-10 cross-project writes | Same tests; browser two-project saves | Foreign project/root references refused; disk checked independently |
| FS-SEC-11 worktree collisions | `edit_24_agent_worktree_is_real_distinct_and_read_only`; `fs_sec_11_git_reads_refuse_a_worktree_redirected_to_another_project`; browser worktree scenario | Distinct native roots; forged Git redirection refused |
| FS-SEC-12 authentication | `fs_sec_12_13_authentication_csrf_and_origin_are_required` | Unauthenticated file operations refused |
| FS-SEC-13 CSRF/origin | Same test | Missing CSRF and foreign origin refused |
| FS-SEC-14 replacement race | `fs_sec_14_edit_12_external_replacement_before_commit_is_a_conflict`; `fs_sec_14_atomic_replacement_after_last_check_is_rolled_back` | Real replacement before and after final precheck; external bytes retained |
| FS-SEC-15 hardlinks | `fs_sec_15_hardlinks_cannot_alias_external_data` | Real hardlink to outside data refused |
| FS-SEC-16 bounded read | `fs_sec_16_17_edit_21_22…`; browser binary/large-file scenarios | Explicit metadata/read-only preview |
| FS-SEC-17 bounded write | Same native test | Oversized write cannot replace source |
| FS-SEC-18 bounded search | `fs_sec_18_search_results_and_scan_are_bounded`; search/cancel native test; 5,000-file browser benchmark | Real result/time/byte limits; partial results labeled |
| FS-SEC-19 root authority | Native root/project forgery + HTTP wrong-root test | Browser cannot submit an arbitrary root |
| FS-SEC-20 safe errors | HTTP invalid-path/error test | No fixture absolute root in domain error JSON |

These tests do not establish an OS sandbox or eliminate every hostile concurrent namespace mutation. See [file-service limitations](file-service-architecture.md). Windows exclusive-lock, case-only rename, readonly-attribute, recovery-purge, corrupt-sidecar and protected-subtree-move tests provide additional native coverage. Linux executable-bit and symlink tests exist under platform configuration but are not native Windows passes.

## Functional contracts

| Scenario | Evidence |
| --- | --- |
| EDIT-01 real Rust file | Browser lazy tree/source journey |
| EDIT-02 exact save | Native byte comparison, HTTP read-after-write, browser independent `fs.readFile` |
| EDIT-03 Unicode/newlines/BOM | Native UTF-8/CRLF/BOM/no-final-newline matrix and Unicode Python browser save |
| EDIT-04 multiple documents | Browser real Rust/TS/Python tabs; document-model ownership tests |
| EDIT-05 splits and movement | Browser shared-buffer split; unchanged P04 docking movement suite; independent cached pane states |
| EDIT-06 unsaved project switching | Browser A/B switch retaining A's dirty text |
| EDIT-07 reload | Browser restored draft without a disk write |
| EDIT-08 Rust restart | Browser stop/start and restored scoped views; P04 restart regression |
| EDIT-09 clean/dirty external changes | Native revisions; document-model refresh tests; browser external writer |
| EDIT-10 conflict | Native/API stale revision refusal and browser manual resolution |
| EDIT-11 simultaneous saves | Native barrier-controlled two writers, one successful incompatible update |
| EDIT-12 agent-like writer | Native/browser controlled external writer, retained base/local/disk versions |
| EDIT-13 create | Native/HTTP/browser exclusive file create; native directory create |
| EDIT-14 rename | Native/HTTP/browser rename with open dirty document |
| EDIT-15 move | Native/HTTP/browser in-root move, source identity retained |
| EDIT-16 delete | Explicit confirmation, native/HTTP/browser deletion and non-overwriting recovery |
| EDIT-17 filename search | Browser real path result and navigation |
| EDIT-18 content search | Native/HTTP/browser real match and line number |
| EDIT-19 search bounds | Native hit limit/cancel and large-repository measurement with completeness metadata |
| EDIT-20 diff | Browser disk/local and committed HEAD/local; native committed blob version test |
| EDIT-21 binary | Native metadata-only and browser non-editable file metadata |
| EDIT-22 oversized text | Native bounded preview and browser read-only large file |
| EDIT-23 two projects | Native/HTTP/browser identical paths, separate saves |
| EDIT-24 two worktrees | Native Git worktree, browser repository/agent root, forbidden manual worktree save |
| EDIT-25 internals | Native/HTTP/browser protected paths |
| EDIT-26 failed save | Native read-only/exclusive-lock cases, document failure retention, browser real filesystem refusal |
| EDIT-27 interrupted prepare | Native injected failure before exchange, unchanged source and cleaned temporary file |
| EDIT-28 concurrent replacement/rename | Native same-name replacement refuses old revision; missing-file document retains its draft |
| EDIT-29 workspace restoration | Browser split/draft reload plus 30 additional real-file tabs restored |
| EDIT-30 V1 | Existing Rust execution suites and P03/P04/V1 browser regressions |

## Document and browser ownership

Additional deterministic frontend cases cover edits during an in-flight save, edits during reload, pending-journal discard, shared panes, root/project partitioning, corrupt foreign drafts, quota refusal, recovery without root discovery, canonical aliases, Save All partial failure and an explicit independent recovery fork. The copied-window browser scenario checks different owner IDs, two retained local versions, server draft CAS and later disk conflict in the other window.

Accessibility evidence combines keyboard explorer/menu navigation, dirty-close dialog use, P02/P04 keyboard regressions and axe results. Automated incomplete/manual-check results remain in the evidence; this is not a certification of every assistive technology. No native Linux/macOS or remote exact-commit CI pass is implied.

Restored document-ID collisions are separately tested for project, worktree root and relative path. File-bound lookup refuses a foreign cached buffer before rendering, scoped save intents refuse it, and invalid view closure cannot discard or save the owning document. Canonical aliases are invalidated on move and release. DOM regressions cover keyboard focus after a clean editor's disabled Save action; the browser benchmark asserts each measured tab switch changes the visible document. Non-text restored diffs and conflicted shared-draft discard have dedicated regressions.
