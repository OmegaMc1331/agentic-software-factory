# Memory sources and provenance

Each source has an explicit owning project and a finite typed reference. Rust resolves it independently; an extractor cannot supply an arbitrary path, API operation or citation URL. Supported references are manual user notes, primary-root repository files, conversation messages, completed missions, finished task attempts, AgentSession metadata, structured review decisions, reported test-command evidence and recorded integration commits.

Repository references preserve the P05 project/root/path identity and actual document revision. Reads use P05's existing path, root, alias, link and size protections plus repository-context ignore rules and memory-specific secret-file exclusions. Only saved primary repository sources are accepted. Unsaved buffers remain P07 context attachments; P11 does not silently turn an editor buffer into a verified disk fact. File evidence is bounded to 32 KiB.

Mission, attempt, message and session IDs resolve inside the owning project database. A matching numeric ID in a different project is unrelated. Integration commit references must match the recorded run integration and resolve through the existing bounded Git service. Raw AgentSession transcripts and hidden reasoning are not extraction sources. Test-command evidence is explicitly labelled Worker-reported; it does not prove an independent test passed. Integration journals and actual review records retain their existing evidence semantics.

Manual assertions are labelled user-authored, and manual corrections are user-edited. Automatically extracted candidates remain proposed until the applicable acceptance policy succeeds. Repository identity/content checks establish the source and its revision, not the semantic truth of every claim. Extracted claims cite bounded exact excerpts that Rust verifies against the resolved source, and retain source fingerprints and excerpts for inspection.

## Change observation

Source content is rechecked on detail/search inspection and before contextual use. A changed file means the claim requires revalidation; it is not automatic proof that the claim is false. Missing, protected or unavailable sources remain identifiable historically and are labelled unavailable. Active affected records become stale with a new revision; their old claim and provenance are preserved in history. An unrelated Git commit does not invalidate unchanged file content.

There is no unbounded filesystem watcher or assertion that a model continuously reverified all knowledge. Observation is bounded and source-specific. A user can edit the claim, refresh its references and explicitly save a new revision. Re-enabling a disabled record does not authorize reuse of changed evidence without refresh.

P03/P05 revalidation narrows stable path-replacement hazards; it is not a complete hostile-filesystem TOCTOU guarantee or an OS sandbox. These inherited boundaries remain applicable during source validation.
