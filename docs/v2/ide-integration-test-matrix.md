# P06 integration and security evidence

This matrix maps contracts to executable tests. Actual exits, counts, native platform and unresolved gates are recorded in the delivery (historical record held privately) and its evidence manifest. An ignored test is not a pass: the real-tool group must also be run explicitly with `ASF_IDE_TEST_TOOLS`. The adversarial Python peer is a deterministic security fixture, not a substitute for language qualification.

## Language services

| Requirement | Evidence |
| --- | --- |
| LSP-01 TypeScript initialization, diagnostics and hover | `factory-ide/tests/services.rs::real_typescript_lsp`; P06 browser TypeScript diagnostic/hover journeys |
| LSP-02 unsaved content | Real TypeScript/Pyright tests synchronize corrected unsaved types; browser proves disk retains the old type until Save |
| LSP-03 completion and definition | Real TypeScript protocol definitions plus browser acceptance of an actual completion and navigation to declaration line |
| LSP-04 Python semantics | `real_python_lsp`: Pyright initialization, diagnostics, hover, symbols and definition |
| LSP-05 Rust semantics | `real_rust_lsp`: rust-analyzer initialization, hover, symbols and definition on a real Cargo project |
| LSP-06 edits | Real server formatting result goes through `prepare_verified_edit`; stale issued edits refused; browser rename, formatting and selectable code-action review update P05 buffers |
| LSP-07 external changes | Each real-language test changes the file externally and verifies edit preflight refusal; P05 save/conflict regressions remain authoritative |
| LSP-08 restart | Each real-language test kills its owned peer, starts a different PID and resynchronizes the retained unsaved text |
| LSP-09 project/window isolation | `real_multi_project_language_and_debug_isolation` runs two actual TS servers; API scoped-ID tests; browser copied-window ownership test |
| LSP-10 positions | `position` Unicode/CRLF/surrogate/overlap tests, including 128 differential inputs; actual TypeScript hover/definition after emoji using UTF-16 positions |
| References/symbols/signatures | Browser reference list, document symbols and real signature tooltip; backend allowlisted workspace-symbol transport |
| Cancellation/framing/server requests | `controlled_protocol_cancellation_crash_bounds_and_server_requests`: actual cancellation frame, oversize/malformed refusal, event-ring gap, crash and refused `workspace/applyEdit` |
| Documentation content | Frontend tests preserve code markup while stripping executable/resource attributes, SVG/scripts and oversized/deep content |

CodeMirror negotiates UTF-16 in production; UTF-8/32 converter tests do not qualify an unimplemented browser encoding mode. Unversioned diagnostics explicitly disclose that freshness is unverifiable. Dynamic server registrations, arbitrary execute-command actions and workspace resource operations are refused.

## Debuggers

| Requirement | Evidence |
| --- | --- |
| DAP-01/02 Python breakpoint, step, stack, variables and termination | `real_python_dap`; browser stopped Python view verifies 40 then 42 after stepping |
| DAP-03 JavaScript | `real_javascript_dap` using official js-debug's actual target channel and a real Node program |
| DAP-04 Rust | `real_rust_dap` compiles a debug-info Cargo binary and uses actual CodeLLDB |
| DAP-05 launch/attach authorization | `root_uri_and_debug_configuration_refuse_escape_and_attach`; each real DAP test refuses unconfirmed start, arbitrary attach and unconfirmed evaluation |
| DAP-06 cleanup | Real DAP tests explicitly disconnect and verify the peer is dead; Windows Job Objects own descendants; backend/browser restart exercises service cleanup |
| DAP-07 two projects | `real_multi_project_language_and_debug_isolation`: same configuration ID in two projects, distinct processes, foreign ID refusal; A termination leaves B's stack/variables and value 140 usable |
| Frontend async ownership | Frontend debugger test rejects late frame inspection and keeps execution-line state separate by complete file/project identity |

Generic PID attach and reverse terminal launch are deliberately unavailable. The approved js-debug opaque target handshake does not grant arbitrary process attach. Native Linux/macOS debugger qualification remains pending.

## Terminals

| Requirement | Evidence |
| --- | --- |
| TERM-01/02 actual authorized shells | Native cmd and PowerShell tests, plus browser command with real output; PowerShell test independently verifies a written file and filtered environment |
| TERM-03 resize | Native ConPTY resize API and browser terminal resize; bounded server dimensions |
| TERM-04 multiple processes/views | Browser launches a second distinct PID, switches terminals, splits/reconnects views; split view of one terminal remains the same process |
| TERM-05 reconnect | Native snapshot subscription and browser reload/reconnect; replay marked bounded |
| TERM-06 termination | Native/API/browser explicit Stop, exit state and owned process cleanup; unrelated terminals continue |
| TERM-07 backpressure | Existing Runtime slow/full/bounded subscriber tests exercise the shared output engine; the real-console benchmark requests 512 KiB, records actual VT bytes/explicit subscriber eviction and recovers the completion marker from bounded replay without reexecuting the command |
| TERM-08 scope/security | API `development_terminal_ownership_and_authorized_roots`; browser foreign-project WebSocket refusal and terminal ID ownership |
| Managed roots | P05 `edit_24_agent_worktree_is_real_distinct_and_read_only` additionally refuses `process_root` for the real managed worktree |

A shell has the OS user's permissions. Initial cwd, project IDs and logical policies are not an OS sandbox. One input writer per process prevents an unbounded queue behind a blocked PTY; explicit Stop does not acquire that writer lock. Abrupt Unix server-death/process cleanup remains a separate native qualification gate.

## Extensions and persistence

| Requirement | Evidence |
| --- | --- |
| EXT-01 original editor package | Browser imports `extensions/p06-editor/extension.json`, enables its real buffer formatter; frontend executes its actual CodeMirror diagnostic contribution |
| EXT-02 disable/re-enable/uninstall | Browser lifecycle for the editor package; disabled contribution disappears and reinstall requires explicit activation |
| EXT-03 original workspace package | Browser opens the actual contributed Dockview checklist and exercises disable/re-enable/uninstall |
| EXT-04 malformed/traversal package | Strict manifest tests reject unsafe entrypoints, unknown scripts and malformed fields; no archive extraction/JS execution path exists |
| EXT-05 API/platform version | Rust/API tests reject unsupported API/platform/version contracts |
| EXT-06 backend permission bypass | API `declarative_extensions_reject_backend_permissions_and_isolate_activation`; arbitrary filesystem/process permission refused |
| EXT-07 restart | Rust restored project activation and browser backend-restart metadata check; restoration does not launch tools |
| EXT-08 failure isolation | `extension_activation_and_integrity_failure_are_isolated`: dependency failure, blocked dependency uninstall, hash corruption, failed state and unaffected P05 read |
| Application metadata corruption | API `corrupt_ide_metadata_does_not_disable_files_or_v1_workflows` retains the corrupt sidecar while files and V1 runs remain available |
| Cross-project activation | Shared-store extension tests and protected API tests keep activation/preferences under authoritative project IDs |

The two original packages use enforced host contributions, not a fictional list of loaded JavaScript extensions. API 1 is intentionally declarative and has no VS Code compatibility, arbitrary code execution, online marketplace or automatic updater.

## Regression and platform gates

`validate.mjs` records exact commands for formatting, Clippy, full Rust serial/default-threaded tests, targeted files/projects/workspaces/IDE suites, real-tool tests, frontend default/bounded workers and embedded builds. The unchanged P03/P04/P05 browser harnesses are rerun against the same embedded P06 executable. Their prior-phase reports are preserved; new raw results are copied into P06 evidence.

Local Windows browser evidence covers dark/light/narrow layouts, keyboard CodeMirror interactions, dialogs, terminal sockets, extensions and axe checks. It does not replace a native screen-reader/IME audit or Linux/macOS/Windows-on-other-machines qualification. Exact-commit remote CI remains unverified until an actual run exists; no push is authorized.
