# Production LSP integration

`factory-ide::lsp::LanguageServer` owns a real language-server process, initialization response, negotiated capabilities, document map, cancellation map and bounded edit provenance. Ownership is `(project_id, root_id, profile_id, client_id, server_id)`; `client_id` combines the workspace identity and the P05 draft journal's claimed window owner. Copied sessionStorage is disambiguated through the existing BroadcastChannel mechanism before connecting. Independent windows therefore do not share unsaved language documents. Compatible panes in one workspace/window share the same server and P05 document. No LSP process starts merely because a project is registered.

## Ordering and synchronization

Rust sends `initialize`, waits for its response, checks position encoding, then sends `initialized` and approved settings. The CodeMirror client connects to a transport facade using those actual capabilities; this does not initialize the external process a second time. Document and feature calls go through the initialized Rust service.

The custom official-client `Workspace` reads `DocumentManager` buffers and attached views. Hiding, moving or duplicating a pane does not own document lifetime. A server receives `didOpen`, negotiated full/incremental `didChange`, verified `didSave` and document closure. Per-document browser synchronization is serialized. Disk revisions, P05 buffer generations and monotonically increasing LSP versions are distinct. Reconnection seeds LSP counters from the server's observed document versions, so a restored P05 generation of zero cannot reset an existing server's counter.

Unsaved content is transmitted from the shared buffer. The backend does not substitute disk bytes for the buffer. A restart creates a new server instance and the browser resends current documents, including drafts. P05 saving remains independent of language-service availability.

## Positions

The production CodeMirror connection advertises UTF-16 and rejects a server selecting an unsupported encoding. JavaScript/CodeMirror offsets count UTF-16 code units within lines; they are not UTF-8 byte positions. Rust's edit preflight converts negotiated positions with `position::offset` and rejects positions splitting a surrogate pair, invalid lines, overlap and out-of-range edits. The converter also has UTF-8 and UTF-32 differential tests; that is not a claim that the current browser transport negotiates those encodings. CRLF, Unicode, emoji and combining marks are exercised separately.

## Features and events

The official client supplies completion, completion resolution, hover, diagnostics and signature help. ASF provides definition/reference navigation, symbols, rename, formatting and code-action review. Controls use the real advertised capability flags. Unsupported methods fail explicitly; an LSP error does not disable ordinary text editing or saving.

Events are project/server scoped, with cursor/gap metadata. Diagnostics are associated with open file identities and optional server document versions. Versioned stale diagnostics are refused. A server omitting diagnostic versions cannot prove freshness; consumers must retain this distinction rather than assert a current-version validation result. Navigation resolves returned URIs through P05; dependency sources outside the authorized workspace are not silently exposed.

The shared Workspace resets each pane's official `LSPPlugin` mapping after synchronization using its public `clear()` API. It retains actual incremental changes where a pane has the matching base, rather than treating every keystroke as an unrelated full replacement. Old out-of-range diagnostics cannot throw during mapping after shortening a buffer. Delayed toolbar navigation/edit proposals verify the currently focused project before changing presentation state.

Server requests for settings/progress are handled with bounded responses. Unknown requests and dynamic registration are refused. `workspace/applyEdit` is not automatically accepted: it is reported as a proposal and responds `applied: false`. Direct edit-bearing code actions are reviewable; command-only actions and arbitrary `workspace/executeCommand` are unsupported. Server process capabilities are not a sandbox; a trusted server can still perform OS operations independently of its LSP messages.

## Workspace edit contract

Rename/format/code-action responses capture the versions present when the request began. Edit fingerprints retain this provenance in a bounded queue. API preflight rejects an edit without an issued matching response, a changed buffer, unsupported resource operation, outside-project URI, protected path, read-only worktree, incorrect document version or changed disk revision.

Affected files must be open and synchronized before the request that produces an applicable edit. When additional files are identified, ASF loads them and repeats the language request before review; it does not treat a fresh disk read as proof of what an earlier language request observed. Resource create/rename/delete operations from an LSP edit are refused; use P05's explicit file operations.

Applying a reviewed edit changes P05 buffers. It never silently saves over an unrelated dirty buffer. Save/Save All subsequently uses P05's expected disk revisions and atomic replacement. Multi-file disk saving is not an atomic transaction, and P05 reports conflicts per file. A manual external change at any stage preserves the draft and requires revalidation.

## Profiles and setup

Use Settings → IDE tools to approve a JSON tool profile. The executable must be installed locally. Example shape:

```json
{"id":"typescript","name":"TypeScript","kind":"lsp","transport":"stdio","executable":"<absolute node executable>","arguments":["<absolute typescript-language-server cli.mjs>","--stdio"],"languages":["typescript","javascript"],"file_patterns":["*.ts","*.js"],"initialization_options":{},"settings":{},"timeout_ms":15000,"enabled":true}
```

Optional tools are not bundled into ASF or downloaded by opening a project. The qualification tooling records exact versions separately. Rust needs its toolchain/rust-src; TypeScript/Pyright need a compatible Node runtime. ASF itself launches without Node or Python.

Primary references retrieved 2026-10-29: [LSP 3.17](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/), [official CodeMirror client source](https://code.haverbeke.berlin/codemirror/lsp-client), [TypeScript language server](https://github.com/typescript-language-server/typescript-language-server), [Pyright](https://github.com/microsoft/pyright), [rust-analyzer installation](https://rust-analyzer.github.io/book/installation.html). The archived GitHub mirror explicitly points to the maintainer's relocated repository; it is not evidence of abandoned maintenance. The pinned package declarations/LICENSE were used where the website/new forge could not be fetched by the browsing tool.
