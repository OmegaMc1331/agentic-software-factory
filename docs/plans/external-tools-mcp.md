# Spec: External Tools / MCP

**Author:** Factory maintainers
**Date:** 2026-08-23
**Status:** Approved
**Reviewers:** Repository owner (approved by implementation request)
**Related specs:** `docs/plans/run-branch-integration.md`

## Context

Factory already mediates role-aware task execution, parallel scheduling,
repository context, policy resolution, recipes, agent routing, evaluation, and
durable evidence. External coding CLIs are deliberately treated as execution
adapters rather than trusted policy authorities. Factory currently has no
standard way to discover or invoke external service tools, so agents either
cannot use them or must rely on provider-local configuration that Factory cannot
audit or constrain.

This feature adds Model Context Protocol (MCP) as a Factory-owned integration
boundary. Project configuration declares trusted server endpoints or structured
local commands. The Policy Engine resolves the permitted subset for each
role/agent/task/recipe context. A central MCP registry discovers tools, classifies
them conservatively, executes bounded calls, and records compact evidence.

Provider-native MCP configuration differs across Codex, Claude Code, OpenCode,
Gemini CLI, Qwen Code, and custom agents. Factory therefore uses a provider-neutral
scoped grant and `factory mcp call` bridge for automated agents. The bridge carries
an opaque, short-lived capability in the child process environment, resolves the
effective policy again at invocation time, and never exposes a raw dashboard tool
execution route. Provider configuration remains outside the authorization boundary.

## Functional Requirements

- FR-1: Factory MUST support project-local MCP server configuration under
  `[mcp.<server>]` in `.factory/config.toml` without changing the behavior of
  existing configurations that omit `mcp`.
- FR-2: Factory MUST support MCP stdio servers launched with a structured
  executable and argument vector, without shell interpretation.
- FR-3: Factory MUST support current MCP Streamable HTTP endpoints.
- FR-4: Factory MUST use stable MCP initialization, `tools/list`, and
  `tools/call` semantics rather than a proprietary upstream protocol.
- FR-5: Factory MUST discover tool names, descriptions, input schemas,
  annotations, and server status; it MUST expose only bounded metadata through
  APIs and agent missions.
- FR-6: Factory MUST centralize configuration validation, discovery, relevance,
  authorization, invocation, timeouts, size limits, result handling, and audit
  in a dedicated MCP integration layer.
- FR-7: Policy scopes MUST accept a `tools` dimension with `allow` and `deny`
  patterns for roles and agents.
- FR-8: Tool policy resolution MUST apply Factory invariants, then role policy,
  then agent restrictions, then recipe restrictions; later layers MUST only
  narrow access and deny MUST override allow.
- FR-9: A role with no explicit tool allow policy MUST receive no external tools.
- FR-10: Recipe tool selectors MUST only intersect/narrow the effective policy
  and MUST NOT grant access absent from role and agent policy.
- FR-11: Factory MUST classify tools as `read`, `write`, `destructive`, or
  `unknown`; a project-local override MAY supply an audited classification.
- FR-12: Consistent MCP annotations MUST be preferred to name heuristics, but
  server-provided annotations or descriptions MUST NOT grant permission.
- FR-13: Tools with missing or conflicting safety metadata MUST be `unknown`;
  unknown and destructive tools MUST require an exact fully-qualified allow rule.
- FR-14: Every tool invocation MUST re-check the scoped grant, tool identity,
  effective policy, recipe restriction, classification, and input schema before
  contacting the server.
- FR-15: Automated agents MUST receive no more than 12 deterministically selected
  relevant permitted tools, rendered in a bounded `AVAILABLE EXTERNAL TOOLS`
  mission section with qualified name, classification, description, and safe
  invocation instructions.
- FR-16: Tool relevance MUST be deterministic from role, `TaskOperation`, task
  objective, recipe selectors, policy, and capability metadata; it MUST NOT call
  another model and MUST NOT affect Intelligent Agent Routing.
- FR-17: Automated agent execution MUST receive a short-lived opaque tool grant;
  only its hash MAY be durable, and stdout/stderr persistence MUST redact the
  plaintext grant if an agent prints it.
- FR-18: `factory mcp call` MUST require a valid active scoped grant and MUST NOT
  accept caller-supplied role, agent, task, run, or policy identities.
- FR-19: MCP results MUST be bounded and wrapped as explicitly untrusted external
  data; content MUST NOT be interpreted as Factory instructions or repository
  truth.
- FR-20: Every call MUST have a timeout and bounded response size. Workflow or
  agent cancellation MUST revoke the grant and terminate or invalidate active
  tool work where the transport permits.
- FR-21: Optional discovery and tool infrastructure failures MUST be classified as
  `server_unavailable`, `timeout`, `cancelled`, `permission_denied`,
  `invalid_schema`, `protocol_error`, `response_too_large`, or `tool_failure` and
  MUST NOT crash the Factory process.
- FR-22: Factory MUST persist compact `ToolInvocation` records with run, task,
  attempt, agent, role, server, tool, classification, status, timing, bounded
  summaries, and error classification; it MUST NOT persist credentials or grant
  tokens.
- FR-23: Tool server execution MUST be concurrency-safe, use per-server limits,
  and MUST NOT hold SQLite transactions while waiting on an MCP server.
- FR-24: MCP server environment and HTTP header values in committed config MUST be
  environment references of the form `${NAME}`; resolved values MUST be filtered,
  redacted from API/dashboard/session/audit output, and passed only to the selected
  server.
- FR-25: The dashboard MUST add a compact External Tools / MCP Settings section
  that lists status and tool count and supports add, edit, remove, connection test,
  and tool inspection without offering arbitrary tool execution.
- FR-26: Role and agent inspectors MUST show compact effective Allowed/Denied
  external-tool permissions.
- FR-27: The API MUST provide semantic server CRUD, test, and tool-list endpoints
  and MUST NOT expose raw MCP JSON-RPC, generic shell execution, or an unrestricted
  dashboard invocation endpoint.
- FR-28: External tool operational metrics MUST report calls, successes, failures,
  and median duration separately from agent-quality scores; infrastructure failures
  MUST NOT reduce agent-quality measurements.
- FR-29: Repository Context Engine output augmented from an MCP tool MUST remain
  visibly external and MUST NOT be represented as an approved worktree or local
  repository fact.
- FR-30: Active tool state MAY be surfaced as inspector metadata, but this version
  MUST NOT add decorative permanent MCP nodes or edges to Agent Graph.

## Non-Functional Requirements

- NFR-1: Default discovery timeout MUST be at most 10 seconds and default tool
  timeout MUST be at most 60 seconds; both MUST be configurable within validated
  bounds of 100 milliseconds to 10 minutes.
- NFR-2: Default serialized tool output MUST be at most 256 KiB and configurable
  only within 1 KiB to 1 MiB; summaries MUST be at most 2 KiB each.
- NFR-3: A server MUST default to at most four concurrent calls, configurable from
  one through 32, without a global MCP mutex.
- NFR-4: An automated mission MUST expose at most 12 tools, descriptions MUST be
  truncated to 500 characters, and schemas MUST not be dumped into the mission.
- NFR-5: Server IDs and tool names MUST be validated as bounded identifiers; stdio
  commands MUST use structured arguments and relative working directories MUST
  resolve within the repository root.
- NFR-6: HTTP URLs MUST be absolute `http` or `https` URLs; loopback HTTP is allowed
  for local development, while credentials MUST enter only through environment
  references.
- NFR-7: All new persisted tables and configuration fields MUST be additive and
  backward-compatible with projects created before this feature.
- NFR-8: Rust formatting, workspace tests, dashboard tests, dashboard lint, and
  dashboard build MUST pass on the supported Windows/Linux/macOS CI matrix.

## Acceptance Criteria

### AC-1: Backward-compatible configuration (FR-1, NFR-7)
Given an existing `.factory/config.toml` with no `mcp` table
When Factory loads, validates, and atomically writes the configuration
Then no MCP server is configured and all existing fields retain their behavior.

### AC-2: Stdio discovery (FR-2, FR-4, FR-5)
Given a fake local stdio MCP server with one schema-bearing tool
When Factory tests the server
Then it completes MCP initialization and `tools/list` and returns Connected with
the qualified tool metadata.

### AC-3: Streamable HTTP discovery (FR-3, FR-4, FR-5)
Given a fake local Streamable HTTP MCP endpoint
When Factory tests the server
Then it discovers the endpoint tools without using legacy HTTP+SSE semantics.

### AC-4: Deny wins (FR-7, FR-8, FR-9)
Given a role allow pattern that matches a tool and any applicable deny pattern
that also matches it
When effective permissions are resolved
Then the tool is denied.

### AC-5: Only-narrowing composition (FR-8, FR-10)
Given role, agent, and recipe tool policies
When the effective set is resolved
Then agent and recipe allows intersect earlier access and their denies union, and
neither layer can introduce a tool the role did not allow.

### AC-6: Conservative classification (FR-11, FR-12, FR-13)
Given tools with consistent read-only annotations, additive-write annotations,
destructive annotations, missing annotations, and conflicting annotations
When Factory classifies them
Then their classifications are read, write, destructive, unknown, and unknown,
respectively, unless a project override exists.

### AC-7: Unauthorized call rejected locally (FR-14, FR-18)
Given a scoped grant whose policy denies a tool
When the agent bridge requests that tool
Then Factory returns `permission_denied`, records the denied audit entry, and sends
no `tools/call` request to the MCP server.

### AC-8: Relevant bounded catalog (FR-6, FR-15, FR-16, NFR-4)
Given more than 12 policy-permitted discovered tools
When a mission is built for a role, operation, objective, and recipe
Then no more than 12 deterministically relevant tools appear and no full input
schema or unrelated denied tool appears.

### AC-9: Secret and grant redaction (FR-17, FR-22, FR-24)
Given environment references resolving to secret values and an active grant token
When discovery, invocation, audit, API serialization, and agent output persistence
run
Then no resolved secret or plaintext grant occurs in SQLite, dashboard JSON,
AgentSession output, or audit summaries.

### AC-10: Untrusted bounded result (FR-19, FR-29, NFR-2)
Given an MCP result containing prompt-injection text
When the call succeeds
Then the text is returned only inside an `UNTRUSTED EXTERNAL TOOL DATA` envelope,
is size-bounded, and does not alter mission policy sections.

### AC-11: Failure and cancellation behavior (FR-20, FR-21)
Given fake servers that hang, crash, return malformed protocol data, or exceed the
response limit
When Factory calls them or the workflow is cancelled
Then each call terminates or becomes invalid within its bound, receives the correct
failure class, and does not crash the Factory process.

### AC-12: Parallel calls (FR-23, NFR-3)
Given calls to two servers and multiple calls to one limited server
When tasks invoke them concurrently
Then different servers progress independently, the per-server limit is honored,
and SQLite remains available while external I/O waits.

### AC-13: Semantic UI/API (FR-25, FR-26, FR-27, FR-30)
Given configured connected and unavailable servers
When Settings and an inspector load
Then they show status, counts, tool details, and effective allow/deny state; CRUD and
test actions use semantic endpoints and no arbitrary invoke action is present.

### AC-14: Operational metrics attribution (FR-28)
Given successful, tool-failure, and infrastructure-failure invocation records
When performance data is summarized
Then tool call counts/success/failure/median duration are reported separately and
infrastructure failures do not change agent quality scores.

## Edge Cases and Error Scenarios

- EC-1: Missing stdio executable -> `server_unavailable`; no shell fallback.
- EC-2: Stdio child exits during initialization or a call -> `server_unavailable`
  or `tool_failure` according to whether initialization completed.
- EC-3: Malformed JSON-RPC or invalid `tools/list` schema -> `protocol_error` or
  `invalid_schema`; metadata is not cached as healthy.
- EC-4: Duplicate qualified tool names after discovery -> reject the conflicting
  server metadata rather than choose one nondeterministically.
- EC-5: Tool result exceeds the byte limit -> cancel/close the client, record
  `response_too_large`, and do not persist a partial secret-bearing body.
- EC-6: Timeout -> cancel or close the client and record `timeout` without retrying
  a write/destructive/unknown call.
- EC-7: Workflow cancellation -> revoke its active grants; late bridge calls are
  denied and active transports are cancelled where possible.
- EC-8: Environment reference is absent -> server is Unavailable with only the
  variable name in diagnostics, never a value.
- EC-9: Configured stdio `cwd` escapes the repository -> validation fails before
  process launch.
- EC-10: HTTP URL has a non-HTTP scheme or embedded credentials -> validation fails.
- EC-11: Server metadata claims injected instructions or safety permissions -> it
  remains description/data, cannot change policy, and is truncated for display.
- EC-12: Agent prints the capability token -> persisted/broadcast session output
  substitutes `[REDACTED]`.
- EC-13: Grant expires or invocation happens after session completion -> local
  `permission_denied`; server is not contacted.
- EC-14: Recipe selects a denied tool -> tool remains denied.
- EC-15: Metadata cache is absent/stale -> live discovery is attempted within its
  timeout; failure is optional infrastructure failure, not an agent failure.
- EC-16: Server removal while a call is active -> current call may finish under its
  immutable grant snapshot, but no new call can start.

## API Contracts

All responses use camelCase JSON. Validation errors use the existing API error
shape and appropriate 400/404/409/500/503 status codes.

```typescript
type McpTransport = "stdio" | "http";
type McpServerStatus = "connected" | "unavailable" | "unknown";
type ToolClassification = "read" | "write" | "destructive" | "unknown";

interface McpServerInput {
  id: string;                         // 1..64 safe identifier characters
  transport: McpTransport;
  command?: string;                   // stdio only
  args?: string[];                    // stdio only, structured
  cwd?: string;                       // repository-relative, stdio only
  url?: string;                       // absolute http(s), HTTP only
  env?: Record<string, "${ENV_NAME}">;
  headers?: Record<string, "${ENV_NAME}">;
  timeoutMs?: number;
  maxResponseBytes?: number;
  maxConcurrency?: number;
  classifications?: Record<string, ToolClassification>;
}

interface McpServerView {
  id: string;
  transport: McpTransport;
  command?: string;
  args: string[];
  cwd?: string;
  url?: string;
  env: Record<string, string>;         // references only, never resolved values
  headers: Record<string, string>;     // references only, never resolved values
  timeoutMs: number;
  maxResponseBytes: number;
  maxConcurrency: number;
  status: McpServerStatus;
  toolCount: number;
  lastError?: string;
}

interface McpToolView {
  qualifiedName: string;              // <server>.<tool>
  server: string;
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
  classification: ToolClassification;
  allowed?: boolean;                  // inspector context only
  denialReason?: string;
}

interface ToolMetrics {
  calls: number;
  successes: number;
  failures: number;
  medianDurationMs?: number;
}
```

- `GET /api/mcp/servers` -> `200 McpServerView[]`.
- `POST /api/mcp/servers` with `McpServerInput` -> `201 McpServerView`.
- `PUT /api/mcp/servers/:id` with `McpServerInput` -> `200 McpServerView`;
  path ID is immutable and wins.
- `DELETE /api/mcp/servers/:id` -> `204`; unknown ID -> `404`.
- `POST /api/mcp/servers/:id/test` -> `200 McpServerView` for a completed test,
  including Unavailable as a diagnostic state; malformed config remains `400`.
- `GET /api/mcp/servers/:id/tools` -> `200 McpToolView[]`; discovery failure ->
  `503` with a classified, redacted message.
- `GET /api/roles/:id/tools` -> `200 McpToolView[]` with effective role policy.
- `GET /api/agents/:id/tools?role=<role>` -> `200 McpToolView[]` with combined
  role/agent policy. Unknown role/agent -> `404`.

There is intentionally no dashboard `tools/call` endpoint. The internal CLI shape is:

```text
factory mcp call <server.tool> --input-json <JSON object>
```

It requires Factory-injected `FACTORY_MCP_ROOT` and opaque grant environment values.

## Data Models

### McpServerConfig (TOML, not SQLite)

| Field | Type | Constraints |
|---|---|---|
| id | string map key | 1..64, safe identifier, immutable through path API |
| transport | enum | `stdio` or `http` |
| command | string? | required only for stdio; no control characters |
| args | string[] | structured; bounded; no control characters |
| cwd | string? | repository-relative and contained |
| url | string? | absolute http(s), no userinfo |
| env | map<string,string> | values must be `${NAME}` references |
| headers | map<string,string> | values must be `${NAME}` references |
| timeout_ms | u64 | 100..600000, default 60000 |
| max_response_bytes | usize | 1024..1048576, default 262144 |
| max_concurrency | usize | 1..32, default 4 |
| classifications | map<string,enum> | explicit project-owned overrides |

### ToolMetadata (SQLite cache)

| Field | Type | Constraints |
|---|---|---|
| server_id | text | server config ID, indexed |
| tool_name | text | MCP name, composite unique with server_id |
| description | text | bounded/sanitized for control characters |
| input_schema | JSON text | valid object schema, bounded |
| annotations | JSON text? | protocol metadata, untrusted |
| classification | text | read/write/destructive/unknown |
| discovered_at | RFC3339 text | UTC |

### ToolGrant (SQLite)

| Field | Type | Constraints |
|---|---|---|
| id | text | random public identifier |
| token_hash | text | SHA-256 only; plaintext never stored |
| run_id | integer? | FK-compatible audit scope |
| task_id | integer? | FK-compatible audit scope |
| attempt_id | integer? | FK-compatible audit scope |
| session_id | integer? | agent session scope |
| agent | text | immutable snapshot |
| role | text | immutable snapshot |
| operation | text? | immutable snapshot |
| recipe_tools | JSON text | only-narrowing selector snapshot |
| expires_at | RFC3339 text | short-lived |
| revoked_at | RFC3339 text? | set on completion/cancellation |

### ToolInvocation (SQLite)

| Field | Type | Constraints |
|---|---|---|
| id | integer | primary key |
| run_id/task_id/attempt_id/session_id | integer? | attribution only |
| agent/role | text | immutable attribution |
| server/tool | text | qualified target |
| classification | text | call-time classification |
| status | text | success or classified failure |
| started_at | RFC3339 text | UTC |
| duration_ms | integer? | bounded non-negative |
| input_summary | text | redacted, <= 2048 chars |
| output_summary | text? | redacted, <= 2048 chars |
| error | text? | redacted classification-safe message |

### ToolServerLease (SQLite)

| Field | Type | Constraints |
|---|---|---|
| server_id | text | primary key |
| active_calls | integer | updated only in short transactions |
| updated_at | RFC3339 text | stale-lease recovery evidence |

## Out of Scope

- OS-1: Public MCP marketplace or automatic server downloads/execution; project
  owners must configure executable paths or URLs explicitly.
- OS-2: Cloud MCP proxy and remote Factory agents; this feature is project-local.
- OS-3: Automatic OAuth for every provider; environment-backed stdio variables and
  HTTP headers are supported, while interactive OAuth requires a future reviewed
  design.
- OS-4: Provider-native MCP configuration as the security boundary; provider support
  is heterogeneous and user-level config could expose unapproved servers.
- OS-5: Autonomous approval of destructive tools; exact policy permission is still
  required and no model makes the decision.
- OS-6: LLM-based tool selection; relevance is deterministic.
- OS-7: Dashboard arbitrary invocation, raw JSON-RPC, or generic shell execution.
- OS-8: Permanent Agent Graph nodes for every server/tool; inspectors provide the
  initial UI to avoid visual noise.
- OS-9: MCP resources and prompts as first-class agent capabilities; initial support
  is tools only, while ordinary tool results may augment context as external data.
