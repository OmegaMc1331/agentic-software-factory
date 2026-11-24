# ASF declarative extension API 1

ASF P06 implements an installed, persistent extension runtime for **validated declarative packages**. It does not execute arbitrary extension JavaScript, support VSIX extensions or claim VS Code API compatibility. Contributions activate as genuine CodeMirror extensions, editor commands or Dockview-compatible React views supplied by the ASF host.

This model enforces its capability vocabulary because packages contain data rather than unrestricted same-realm code. A Worker alone would not make trusted same-origin JavaScript safe; no such sandbox claim is made. New executable extension models require a separate security design.

## Package schema

A UTF-8 JSON package contains `id`, `name`, three-part `version`, `publisher`, `description`, `entrypoint: "contributions"`, `extension_api_version: 1`, `capabilities`, `permissions`, `supported_platforms`, exact-version `dependencies` and `contributions`. Unknown schema fields, unsafe identifiers, incompatible API/platforms and undeclared permissions are refused.

Allowed permissions are `editor`, `commands`, `views`, `settings`, `profiles`. There is no direct filesystem/process/network permission. Packages cannot supply installation scripts, archive paths, symlink entries, executable payloads, remote imports or arbitrary HTML. Imports therefore have no archive-extraction path to traverse. The frontend renders supplied text with React escaping.

| Contribution | Actual host behavior |
| --- | --- |
| `formatters` | Approved bounded text transforms: trim trailing whitespace, ensure final newline, sort lines; modify P05 buffers |
| `diagnostics` | Literal bounded marker rules as real CodeMirror lint sources, severity/source retained |
| `languages` | Map declared suffixes to pinned host syntax modules; unknown parser IDs refused |
| `commands` | Named formatter/view actions, optional editor keybindings/context-menu declaration |
| `views` | Titled text views registered in the existing Dockview workspace |
| `settings` | Declared string preferences with project-scoped durable overrides |
| `lsp_profiles` / `dap_profiles` | References to separately approved tool profiles; no implicit executable installation or launch |

Formatters never write files directly; P05's explicit Save performs revision-aware persistence. A workspace contribution cannot invoke internal Rust state or operate on another project. Unsupported execution APIs are rejected rather than displayed as active compatibility.

## Lifecycle and storage

Extensions → Install trusted local package accepts a bounded JSON file or pasted JSON. The user reviews publisher/version/permissions. Rust validates the package, records a SHA-256 and local provenance, then installs it **disabled**. Enable validates dependencies and activates contributions; Disable removes them. Uninstall removes the registry/artifact/settings, refuses active dependency removal and leaves existing view identities in an explicit unavailable state.

Artifacts are stored as validated JSON in the application IDE SQLite sidecar, outside arbitrary project folders. Activation/settings are partitioned by authoritative project ID. Duplicate IDs are rejected transactionally, including concurrent imports. Exact-version dependencies must already be installed and enabled. No lifecycle action executes a package script. Automatic marketplace/update execution is not implemented; explicit uninstall/import of another version remains subject to validation and dependency checks. There is no implied rollback of arbitrary executable code.

Manifest/load failures are visible and do not become a false successful code contribution. Declarative content is bounded: 128 KiB package, 64 contributions, literal search needles at most 128 characters, at most 200 reported editor diagnostics. Package metadata/provenance does not prove trust in a publisher; installation remains an explicit decision.

## Original qualification packages

* [`extensions/p06-editor/extension.json`](../../extensions/p06-editor/extension.json): ASF Source Hygiene, a real CodeMirror literal diagnostic source and trailing-whitespace formatter/command.
* [`extensions/p06-workspace/extension.json`](../../extensions/p06-workspace/extension.json): ASF Workspace Notes, a real command, keybinding, settings contribution and workspace checklist view.

These original MIT-licensed repository fixtures require explicit local installation. They are not fabricated active extensions in production. See the [integration test matrix](ide-integration-test-matrix.md) and P06 delivery for executed lifecycle evidence and remaining qualification limits.
