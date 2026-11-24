# Orchestrator interaction contract

P09 extends the historical configuration described below with project-owned profiles. The orchestrator uses the same profile-aware `Agents` resolution as Planner, Worker and Reviewer. A profile selects a trusted installation and bounded model/mode configuration. Its instructions form a separate bounded section beneath application and project policy. Actual invocations retain immutable execution snapshots and separate native model observations. Editing a profile affects future selections, not an active conversation subprocess. No model gateway, account sharing or silent model substitution is introduced. See [provider/model configuration](provider-model-configuration.md) and [permission governance](agent-permission-governance.md).

P07 reuses the preferred configured `planner` role as its orchestration profile. Configure its agent, executable, argument vector, authentication and role/agent policies through existing project settings. No model gateway, credential migration, new arbitrary-command endpoint or P09 agent-profile system is introduced. The status endpoint resolves the actual CLI and reports missing or incompatible configuration.

## One finite action

The CLI receives `ASF_ORCHESTRATOR_V1` instructions and returns one JSON object. A single JSON code fence is tolerated; unrelated prose, malformed JSON, unknown fields and unknown actions cannot execute a backend action.

```json
{"action":"respond","content":"A concise answer"}
```

Other variants are `create_mission` with `objective` and optional existing `recipe`, `inspect_mission` with an existing project-owned `run_id`, `reference_files` with bounded relative `paths` and `content`, `clarify`, and `unavailable`. None accepts a project override, shell command, executable, URL to invoke, policy edit or GitHub-publication action. Inspection reads the owning project's database, never another project's matching numeric ID. It persists a read-only snapshot reference without acquiring run ownership or cancellation rights. References are revalidated through P05. Recipes are resolved through Factory's existing catalog, team and policy restrictions; an agent-proposed recipe cannot create or elevate a policy.

The application does not use keyword heuristics to turn every question into a run. The real configured agent classifies intent. Rust validates the finite action and its ownership; semantic classification remains a model decision and is not an authorization boundary.

## Context assembly

User messages, previous agent messages and selected repository snapshots are encoded as untrusted data after fixed instructions. The latest 24 messages are considered newest-first within 32,000 bytes, then restored to chronological order. Older/oversized items can be omitted; `history_is_bounded` is explicit. Four associated mission summaries, ten recent project mission IDs/statuses/objectives (objectives capped at 500 characters) and a bounded existing-recipe catalog supply structured context. Older missions can be inspected by an explicit project-local run ID. The assembled data is redacted against known credentials before invocation. This is bounded reconstruction, not native CLI-session continuity or P11 memory.

File context preserves project, root, relative path, base disk revision, current disk revision, saved/unsaved/stale state, read-only status and optional UTF-8-byte selection. It uses actual shared P05 buffers, not an implicit disk substitution. CodeMirror offsets are converted across UTF-16 JavaScript strings and the document's actual LF/CRLF serialization. Invalid byte boundaries are rejected. The selected pane's most recent focused selection is retained independently of the chat focus.

P05 path authorization and repository ignore rules are applied. Git/Factory internals, common secret filenames, unsupported content and oversized selections are refused. Current diagnostics can be attached explicitly; their P06 freshness labels remain part of their text. Known configured/environment credential values are redacted before message/event persistence. These checks are not general secret detection for arbitrary source text.

## Process and trust boundaries

The existing CommandAgent launch/cancellation/capture path and Factory policy/environment/session audit are reused. Classification has a three-minute deadline and a smaller output capture limit. Agent output is parsed only after successful process completion and output-limit validation. The persisted, redacted agent session supplies the response; raw tool output and stderr do not become chat messages.

The chosen CLI returns final structured output. P07 streams genuine accepted/processing/completed/error and mission events; it does not animate a finished answer as invented token deltas. A CLI with a separately validated incremental response adapter could extend this later.

Repository text cannot expand the typed action schema, choose another project's API context, grant MCP permissions or invoke the delivery API. However, a configured coding CLI still has its actual OS/CLI privileges and may have its own tools. Prompt wording and logical Factory policies are not an OS sandbox. A malicious or over-privileged CLI cannot be contained merely by the response schema. P06 shell/profile restrictions are unchanged.

## Presentation

Agent-generated answers are labelled Orchestrator. Authoritative completion messages are generated from structured runtime records and labelled Mission result. Test commands and review decisions remain inspectable evidence; neither is converted into a blanket assertion that all tests passed. There is no model-generated completion summary that can overwrite authoritative outcome, commit or publication fields.

Rendering supports text paragraphs and fenced code. Model HTML and Markdown links remain inert text; executable HTML and unsafe URL schemes are not rendered. File, workflow and session actions are constructed from scoped backend records. The composer retains text and the durable command key after an uncertain network response. Explicit retry reconciles that key; copying a failed objective to the composer does not execute it.

The native qualification results, including any provider-limit or CLI permission refusal, are recorded in the delivery. A successful question is not proof that the same CLI is authorized to edit files.
## Process failures and provider availability

A nonzero conversational CLI exit uses the existing `AgentProcess/ExecutionFailed` category and includes its persisted agent-session ID. It is distinct from `PolicyBlocked`, which represents an actual policy refusal before execution. An arbitrary CLI's exit code does not reliably distinguish authentication, quota and transient network problems; the redacted session record retains the provider-specific cause. No such failure is automatically retried. Configuration/agent-resolution failures and cancellation retain their existing categories.

Qualification modes remain separate: an original controlled process can validate real workflow mechanics without establishing model capability. The retained native Codex questions/research artifact are genuine provider evidence. An earlier refresh hit its provider usage limit. A renewed 2026-10-30 run reached genuine questions, intent classification and planning after provider access returned, but worker tool requests were policy-refused: three reviews requested changes, no files or tests were produced and the actual run failed. Code-writing acceptance remains unqualified. No alternate agent, policy override or weaker sandbox is used to reinterpret these observations as passing model tests.


## P11 project memory integration

P11 project memory supplies selected facts to conversations and missions through the existing invocation boundary. No relevant saved result means no claimed recall. Corrections can reference memory management; ambiguous statements do not mutate unrelated facts. See [governance](memory-governance.md).
