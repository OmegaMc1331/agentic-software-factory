# Project filesystem API

All routes below are under `/api/projects/:projectId`. They use the P03 immutable project dispatcher, authenticated local browser session, same-origin/Host protection and CSRF for every mutation/POST. The production client always uses explicit project URLs, and no browser-selected absolute root is accepted. The shared P03 legacy dispatcher can also bind these handlers to its immutable server-default project; without a default it refuses the request. A standalone V1 `ApiState` without a P03 context refuses the new file service. Browser project switching cannot change any of these server-side bindings. Ordinary V1 routes are unchanged.

`FileRef = { project_id: "p_<32 hex>", root_id: "r_<32 hex>", path: "src/main.rs" }`. Request/response models live in `factory-files` and `src/v2/files/types.ts`. Extra fields in domain mutation models are refused. JSON bodies are limited to 6 MiB; normal framework JSON/schema rejection is separate from the domain error envelope.

| Method | Suffix | Input / output |
| --- | --- | --- |
| GET | `/files/roots` | Backend catalog, limits, worktree read-only policy |
| GET | `/files/tree` | Query `root_id,path,hidden,after`; deterministic directory page |
| GET | `/files/content` | Query `root_id,path`; bounded file snapshot |
| PUT | `/files/content` | `{file, expected_revision, content}`; acknowledged saved snapshot |
| POST | `/files/entries` | `{file,directory}`; exclusive create |
| POST | `/files/move` | `{file,expected_revision,destination}`; scoped from/to identities |
| DELETE | `/files/entries` | `{file,expected_revision,confirmed:true}`; recovery operation |
| GET | `/files/git-head` | Query `root_id,path`; bounded read-only committed blob with commit/hash revision |
| POST | `/files/changes` | Up to 8 `{file,revision}` items; per-file status/revision |
| POST | `/files/search` | Bounded search request; results with completeness and resource counters |
| POST | `/files/search/:id/cancel` | Advisory cancellation of this project's `q_<32 hex>` request |
| GET | `/files/recovery` | Outstanding prepared/uncertain/deleted operations |
| GET | `/files/recovery/:id/content` | Bounded read-only retained backup/prepared content, if available |
| POST | `/files/recovery/:id/restore` | Restore deletion/interrupted case-rename without overwriting |
| POST | `/files/recovery/:id/purge` | `{confirmed:true}`; explicit permanent recovery cleanup |
| GET | `/editor/drafts/:id` | `d_<32 hex>`; draft envelope with revision |
| PUT | `/editor/drafts/:id` | `{base_revision,document}`; compare-and-swap recovery persistence |
| DELETE | `/editor/drafts/:id` | `{revision,confirmed:true}`; conditional explicit removal |

A snapshot includes `file, revision, content, size, modified, encoding, bom, newline, read_only, reason, mode, durability`. `content:null` never means an empty editable file. `mode:preview` is read only. Modification metadata is epoch milliseconds when available. `durability` is populated only by a confirmed save and documents platform limits.

Directory entries include relative path/name, type, size, hidden/ignored/protected state, revision and actual available Git status. Protected internals are omitted, unsafe links are blocked, and unavailable Git status is labeled rather than fabricated. Pagination cursors are opaque ordering positions, not authorization.

## Error envelope

Filesystem domain errors return `{code,error}` with a stable code and a safe message. Absolute OS paths and raw native error details are not serialized. Framework authentication/JSON/body-limit errors retain ASF's existing responses.

| HTTP | Codes |
| --- | --- |
| 400 | `invalid_file_reference` |
| 403 | `file_path_forbidden`, `worktree_read_only`, `file_access_denied` |
| 404 | `file_not_found`, `unknown_workspace_root`, `git_version_unavailable`; P03 unknown project remains separate |
| 409 | `file_exists`, `file_revision_conflict`, `draft_revision_conflict`, `directory_not_empty`, `file_write_uncertain` |
| 413 | `file_limit` / framework body limit |
| 415 | `unsupported_file_content` |
| 429 | `file_resource_busy` |
| 503 | `editor_storage_unavailable`, `filesystem_error` |

Consult `factory-files/src/error.rs` for the authoritative exact code strings. Saves/moves/deletes must include an observed revision; a stale or replaced resource is refused. A 409 uncertain outcome can mean the OS mutation occurred: retain the draft, inspect recovery/current disk and reconcile before retrying. An HTTP interruption after submission also cannot establish that a mutation did not occur.

No endpoint starts a terminal, language server, adapter, extension, agent, Git push or PR. Source permissions are local-user/root authority, not authorization to publish. [Architecture and security limits](file-service-architecture.md) and [search semantics](file-search.md) are part of this contract.
