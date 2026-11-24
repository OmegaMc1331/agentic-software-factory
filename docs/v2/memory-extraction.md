# Controlled memory extraction

Extraction defaults to **disabled**. Project settings support disabled, proposal/review and automatic storage modes, with an explicit eligible-category list. Manual memory management remains available independently. Ordinary conversations are not automatically retained as permanent knowledge.

An explicit extraction request selects at most four typed sources. The completion hook can inspect verified integration evidence within the existing owned workflow operation. It respects the P10 fence and cancellation flag. Manual extraction from mission-owned records requires eligible settled work; it cannot create an untracked invocation for a suspended mission. No extraction starts a mission, resumes a Worker or modifies policy.

Rust resolves and screens sources first, with at most 48 KiB of source content. The existing configured Planner profile receives `ASF_MEMORY_EXTRACTION_V1`, a finite claim schema and labelled untrusted evidence. The existing role/project/profile/host permissions, capacity reservations, process supervision and authentication apply. The extraction deadline is 180 seconds and captured output is capped at 32 KiB. The API also reserves the application operation budget and admits at most two concurrent explicit extractors.

Output contains at most eight claims, each with category, title, content, subject, scope and one to four source-index/exact-quote citations. Unknown fields, invalid categories, fabricated references, missing quotes, changed source fingerprints and oversized content fail validation before candidate storage. Source indexes refer only to the Rust-resolved request evidence. A generated citation string cannot create provenance.

## Acceptance

Review mode stores a bounded proposal queue. Users inspect sources, accept, edit and accept, or reject. Candidate decisions are transactional and idempotent; duplicated acceptance cannot create two records. Exact duplicate claims are exposed for review rather than silently merged. A deterministic lexical overlap heuristic exposes some near duplicates; it is not a semantic-equivalence oracle. Subject/component-scope collisions expose possible contradictions and require explicit resolution. Newer timestamps are not selected as truth.

Automatic mode accepts only eligible categories with unchanged current settings, checked repository evidence, actual excerpts and no detected conflict or review condition. Policy-like/high-impact categories and detected injection attempts remain proposals. Source and secret checks still apply. Automatic acceptance cannot overwrite a user's earlier correction. The policy is rechecked inside the acceptance transaction so a concurrent disable cannot be ignored.

Known configured/environment secrets and common credential/private-key patterns are screened. Detection has false positives and false negatives; it does not establish perfect secret discovery. Malicious source text remains untrusted even if a claim is accepted. The memory subsystem has no operation that grants tools, changes sandbox flags or authorizes publication.

## Failure and restart

Extraction owns a durable source-revision command and a linked real AgentSession. A repeated request for the same source revision reconciles the existing attempt; there is no automatic identical retry. Provider quota refusal remains an external availability category, not a coding defect or permission grant. Malformed output, cancellation, unavailable providers and storage failures retain accurate failure history without turning completed implementation work into a coding failure. Startup records unfinished extraction as interrupted and does not replay uncertain requests.

Successful implementation/review/integration remains authoritative even if optional knowledge extraction fails. Reaching source or retention limits requires explicit user intervention. No provider polling, account switching or credential migration is introduced.


The shared invocation boundary rechecks the durable extraction claim owner, settings revision and captured source references against the actual resolved role/profile read policy before creating the process or MCP grant. The human caller's access does not override a selected extractor's narrower file-read scope. A denied source records a failed prepared session and policy refusal, with no model invocation.
