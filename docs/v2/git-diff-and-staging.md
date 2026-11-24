# Git versions, diffs and staging

The three local versions are HEAD, the index and the working file. P12 compares working/index, index/HEAD and working/HEAD. Commit comparisons bind full verified commit identities; task reviews use their persisted captured base/head. A current primary file must not be confused with a task or integration snapshot.

P05 and P12 share `ComparisonView`, a read-only CodeMirror MergeView with the existing theme and document presentation. Each Git diff carries base/target labels, path, generation and diff version. Binary/oversized content is labelled unavailable for a complete text comparison. Historical renames resolve their actual original blob. Invalid commit objects fail rather than returning an invented empty version.

## Index transaction

1. Resolve project/root authority and project filesystem/Git/command restrictions.
2. Acquire the common operation lock and Git's standard exclusive `index.lock`.
3. Re-read the expected generation and reject stale state.
4. Copy the actual index to an exclusive temporary directory in Git metadata.
5. Use `GIT_INDEX_FILE` for the typed whole-file or selected-hunk operation.
6. Recheck observed state before publishing the alternate index through the held lock.
7. Flush, replace the index and return newly observed state.

The working file is not an output of staging or unstaging. Unrelated index entries remain present. Lock contention and stale operations fail explicitly. External tools that ignore Git locks or malicious concurrent filesystem/config replacement remain outside a complete race-proof guarantee.

P14 bounds whole-file selections to 10,000 paths and 1 MiB of path bytes. The
authenticated index endpoint alone permits a 1 MiB JSON body; the other Source
Control endpoints retain their 64 KiB body bound. Rust validates every selected
path and filter before writing a NUL-delimited pathspec file in the private
temporary index directory. Git's literal-pathspec mode and
`--pathspec-from-file` / `--pathspec-file-nul` avoid command-line length limits.
Add, reset-to-HEAD and unborn-index removal all operate on the alternate index;
the same common lock, index lock and before-publication generation check apply.
The path list does not accept shell commands or browser-provided patches.

This replaces repeated 256-path transactions for a large selection. It does not
cache an old status, reduce source fingerprint coverage or enable unsupported
paths. Hunks still select exactly one server-generated comparison. See the
official [Git add](https://git-scm.com/docs/git-add),
[Git reset](https://git-scm.com/docs/git-reset) and
[Git rm](https://git-scm.com/docs/git-rm) option contracts.

## Hunks

The server creates a bounded diff from actual before/after bytes. It identifies hunks by a digest of that exact diff/version. The browser submits a hunk ID, never an executable patch or arbitrary Git arguments. Rust regenerates the comparison, checks its version and applies the selected server-generated hunk only to the alternate index. Unstaging reverses the selected hunk.

Renames, conflicts, binary content, missing final newline and CRLF boundaries do not offer hunk staging. New/deleted supported text uses appropriate file-creation/deletion headers. Whole-file operations remain separate and subject to current authorization and fingerprint limits. Hunk staging is never simulated by staging an entire existing file.

Local commits require an expected generation and nonempty message, use the actual staged index and return a verified commit hash. Hooks/signing are not silently bypassed: configured programs require explicit operation trust and remain unavailable under restrictive command ceilings. Commit failures retain working/index evidence; uncertain outcomes require inspection before another request.
