# Project memory search and agent context

Interactive search and contextual selection share project SQLite storage but have different purposes. Interactive search supports title/content tokens, category, lifecycle status, source kind, update date and cursor pagination. SQLite FTS5 is a derived local index. Results always join current authoritative memory identity/revision; stale index text cannot become returned knowledge. The explicit rebuild replaces only derived search data. An unavailable index uses an honestly labelled bounded literal fallback.

Contextual retrieval considers a bounded candidate set for the current conversation objective, task or mission. It excludes disabled, stale, disputed, superseded and deleted records; applies project category/enable settings; revalidates sources; and checks referenced paths against the selected agent's existing effective read permissions. It does not relax role restrictions to improve recall.

Ranking uses deterministic lexical relevance, title/subject emphasis, applicable review categories and source quality. Numeric internals measure lexical selection only; no confidence percentage is displayed. This is ordinary local retrieval, not an embedding model or proof that all relevant knowledge has been found. Limits and truncation notices are explicit.

Default context is at most six records and 12,000 serialized JSON bytes; configurable limits are 1-12 records and 512-32,000 bytes. Candidate queries are bounded. Each invocation receives a labelled `ASF_PROJECT_MEMORY_V1` JSON section with schema/project identity, record IDs, revisions, categories, statuses and provenance. Omitted or oversized records are not represented as remembered. Current tests and current code evidence remain authoritative.

## Selection and audit

Selection is reconstructed for each new automated invocation. There is no persistent prompt cache in a checkpoint or handoff. A short immediate transaction checks the current memory generation, revisions and eligibility before recording the supplied IDs/revisions/content hashes and context hash against the actual AgentSession. A conflicting edit/delete causes bounded reselection rather than launching with an obsolete selected record.

This transaction is the selection boundary. A later edit or deletion cannot unsend context already selected for a running provider invocation; it affects subsequent selections. Audit proves the context was supplied, not that the model followed or causally relied on every record. Historical audits retain references/hashes without retaining a second copy of deleted memory content.

P07 supplies the current message as retrieval objective. P08 initial planning, replanning, Workers and Reviewers use the shared invocation path. P09 replacement attempts independently select current context and keep the old attempt's audit intact. P10 resumption first validates lifecycle/checkpoint/policy and then new invocations select current memory; completed attempts are not reconfigured or rerun.

The Memory view includes a current retrieval preview. Agent output details expose the actual invocation audit separately. Preview is not proof of an agent invocation and does not call a model.
