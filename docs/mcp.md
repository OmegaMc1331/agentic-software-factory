# External Tools / MCP

Factory can connect workflows to external tools through the
[Model Context Protocol (MCP)](https://modelcontextprotocol.io/). MCP is an
integration boundary, not a second agent runtime: Factory remains the authority for
discovery, visibility, authorization, invocation limits, auditing, and cancellation.

```text
Task
  -> role and agent
  -> effective Factory policy
  -> deterministic tool resolver
  -> scoped, expiring grant
  -> Factory MCP client
  -> configured MCP server
  -> bounded, untrusted result
  -> task evidence and operational audit
```

External-tool access is denied by default. Configuring a server does not grant any
role access to it, and server-provided descriptions, annotations, schemas, or results
can never change Factory policy.

## Supported transports

Factory uses the official Rust MCP SDK and stable MCP client semantics:

- `stdio`: Factory starts the configured executable directly with structured
  arguments. It does not use `sh -c`, `cmd /c`, or interpolate a shell command.
- `http`: MCP Streamable HTTP over an absolute `http://` or `https://` URL. Legacy
  standalone HTTP+SSE transport is not implemented.

Each connection performs normal MCP initialization and `tools/list` discovery before
calling `tools/call`. Discovery and calls have separate timeouts. Responses are
bounded while they are read, not only after JSON parsing.

## Server configuration

Servers are project-local entries in `.factory/config.toml` and can also be managed
under **Settings -> External Tools / MCP**.

```toml
[mcp.github]
transport = "stdio"
command = "github-mcp-server"
args = []
timeout_ms = 60000
discovery_timeout_ms = 10000
max_response_bytes = 262144
max_concurrency = 4

[mcp.github.env]
GITHUB_TOKEN = "${GITHUB_TOKEN}"

[mcp.postgres]
transport = "stdio"
command = "postgres-mcp"
args = []
cwd = "tools/postgres"

[mcp.docs]
transport = "http"
url = "http://127.0.0.1:3001/mcp"

[mcp.docs.headers]
Authorization = "${DOCS_AUTHORIZATION}"
```

For `stdio`, `command` is required; `args`, an existing repository-contained `cwd`,
and `env` are optional. Lexical traversal and symlinks resolving outside the
repository are rejected. For `http`, `url` is required and `headers` is optional.
URLs cannot contain credentials. Server IDs use letters, digits, `_`, or `-`.

Limits are validated when configuration is saved:

| Setting | Default | Accepted range |
| --- | ---: | ---: |
| `timeout_ms` | 60,000 | 100-600,000 |
| `discovery_timeout_ms` | 10,000 | 100-600,000 |
| `max_response_bytes` | 262,144 | 1,024-1,048,576 |
| `max_concurrency` | 4 | 1-32 |

Existing projects remain compatible: no `[mcp]` entries means no servers and no
external-tool access.

## Secrets

Configuration accepts only whole environment references such as `${GITHUB_TOKEN}`
for server environment values and HTTP header values. Plaintext values are rejected.
Factory resolves them only when connecting, redacts their values from captured
output, and never serializes the resolved values to TOML, SQLite, the dashboard,
agent sessions, missions, or audit summaries.

The task agent receives only two Factory-owned variables when tools are available:
the repository root and an opaque, scoped grant. The grant is stored hashed, expires,
is revoked when the agent session ends, and cannot authorize a tool outside the
catalog created for that task. It is removed from the bridge process environment
before an MCP child is launched.

The existing environment policy still applies. If it denies Factory's scoped grant
variable, external tools are not exposed to that agent session.

## Discovery and classification

Factory discovers tool names, descriptions, JSON input schemas, MCP annotations, and
server status. Lightweight metadata is cached in local SQLite and refreshed by a
connection test or when an allowed server is needed. The Settings inspector can show
the discovered schema, but task missions receive only a small relevant catalog.

Every discovered tool has one Factory classification:

- `read`: MCP metadata coherently declares the tool read-only;
- `write`: metadata declares non-read-only and non-destructive behavior;
- `destructive`: metadata declares destructive or otherwise privileged behavior;
- `unknown`: metadata is absent, ambiguous, or contradictory.

MCP annotations are untrusted hints, not authorization. A project owner can override
a classification when the server metadata is incomplete:

```toml
[mcp.github.classifications]
search_code = "read"
create_issue = "write"
delete_issue = "destructive"
```

Unknown receives conservative treatment. `write`, `destructive`, and `unknown`
tools require the role policy to name the exact qualified tool; a broad wildcard is
not sufficient. Explicit classification is a security decision and should be
reviewed when the server changes.

## Permissions

External tools are a dimension of the existing Policy Engine:

```toml
[policies.roles.worker.tools]
allow = ["github.read_*", "docs.*", "github.create_issue"]
deny = ["github.delete_*"]

[policies.agents.codex.tools]
allow = ["github.read_*", "docs.search"]
deny = ["github.read_private"]
```

Patterns match qualified `server.tool` names with `*` and `?`. Resolution follows
these rules:

1. Factory safety invariants are always highest.
2. A role must explicitly allow the tool; an absent role tool policy allows none.
3. Agent rules form an additional intersection and can only narrow the role.
4. A Recipe tool selection forms another intersection and can only narrow access.
5. Every matching deny wins over every allow.
6. MCP server text and model output never participate in permission decisions.

The Role and Agent inspectors show effective decisions for discovered tools. Recipe
entries use the same qualified patterns:

```toml
[recipes.backend]
name = "Backend Development"
roles = ["planner", "worker", "reviewer"]
tools = ["docs.*", "postgres.inspect_schema"]
```

A Recipe selects from access already allowed by role and agent policy; it cannot
elevate access. The built-in Security Sensitive and Documentation recipes select
small read-oriented tool sets.

## Agent use and relevance

Before an invocation, Factory chooses visible tools deterministically from:

- the effective role, agent, and Recipe policy;
- the task's `TaskOperation`;
- objective terms;
- discovered capability metadata.

The resolver has a fixed maximum catalog size and does not call another model. Agent
routing continues to choose agents only; it does not choose or authorize tools.
Input schemas remain in the registry instead of being dumped into every prompt.

An eligible mission receives a section like:

```text
AVAILABLE EXTERNAL TOOLS

github.search_code
Read-only
Search repository code.
```

Coding agents invoke a catalogued tool through the provider-neutral command printed
in that section:

```bash
factory mcp call github.search_code --input-json '{"query":"ToolRegistry"}'
```

The command cannot be used outside an active scoped task grant. This Factory-mediated
bridge was chosen after inspecting native MCP controls for
[Codex](https://developers.openai.com/codex/mcp/),
[Claude Code](https://docs.anthropic.com/en/docs/claude-code/mcp),
[OpenCode](https://opencode.ai/v2/docs/mcp-servers),
[Gemini CLI](https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/mcp-server.md),
and [Qwen Code](https://qwenlm.github.io/qwen-code-docs/en/users/features/mcp/).
Those clients support useful MCP configuration and filtering, but Factory must retain
one provider-independent policy and audit boundary. Provider-native configuration is
therefore not the source of Factory authorization.

## Results and repository context

MCP results are visibly enclosed in `UNTRUSTED EXTERNAL TOOL DATA` markers and remain
external context. They cannot override system rules, role instructions, permissions,
repository boundaries, workflow invariants, or output contracts. Tool descriptions
are normalized to bounded single-line metadata, and results are size-limited.

Documentation, issue-tracker, or schema tools may augment a task's understanding, but
their output is never treated as local repository truth, an approved worktree, or Git
evidence. The agent remains responsible for verifying relevant claims locally.

## Auditing and evaluation

SQLite stores a compact record for each invocation attempt: run, task, attempt,
session, agent, role, server, tool, classification, status, start time, duration, and
bounded input/output/error summaries. Input key names and values are omitted; output
content and credentials are not persisted in the audit.

Statuses distinguish `success`, `server_unavailable`, `timeout`, `cancelled`,
`permission_denied`, `invalid_schema`, `protocol_error`, `response_too_large`, and
`tool_failure`. Operational metrics report call counts, success/failure counts, and
median duration separately from agent-quality evaluation. MCP usage is not a quality
signal, and infrastructure outages do not lower an agent's evaluation.

## Concurrency and cancellation

Each server has an independent concurrency limit backed by short SQLite lease
operations. There is no global MCP mutex, and Factory never holds a SQLite lock while
waiting for a server. Different servers and available slots may run concurrently.

Dropping or cancelling an invocation tears down its client/child process where the
transport permits, releases its server slot, and marks a still-running audit record
cancelled. A server crash or an optional discovery failure affects that tool/server;
it does not automatically terminate the Factory process or unrelated workflow work.

## Local API and dashboard

The local API exposes semantic management and inspection operations:

| Method | Route | Purpose |
| --- | --- | --- |
| `GET` / `POST` | `/api/mcp/servers` | List or add configured servers |
| `PUT` / `DELETE` | `/api/mcp/servers/:id` | Edit or remove one server |
| `POST` | `/api/mcp/servers/:id/test` | Initialize and discover tools |
| `GET` | `/api/mcp/servers/:id/tools` | Inspect cached/discovered tools |
| `GET` | `/api/mcp/invocations` | Inspect compact invocation audits |
| `GET` | `/api/mcp/metrics` | Inspect operational metrics |
| `GET` | `/api/roles/:id/tools` | Effective role decisions |
| `GET` | `/api/agents/:agent/tools` | Effective agent decisions by role |

There is deliberately no raw MCP JSON-RPC, arbitrary shell, or unrestricted manual
tool-call endpoint. The dashboard supports server configuration, test connection,
tool inspection, and permission inspection only.

MCP nodes are not permanently added to Agent Graph. This keeps the graph faithful to
real workflow/session topology and avoids decorative tool clutter; active use is
visible through the task/session evidence and invocation audit.

## Security limitations

Factory's policy layer controls access through its scoped bridge, but it is not an OS
sandbox. A coding CLI still runs with the invoking user's filesystem and network
permissions and might have separately configured provider-native tools. Do not give
untrusted agents independent MCP configuration or credentials outside Factory.

Only configure server executables and URLs you trust. A malicious server can return
false or adversarial data, consume resources until a configured limit, or perform any
operation its own process/credentials allow once an authorized tool is called.
Factory validates authorization and schemas, bounds output, and prevents response
text from becoming policy, but it cannot make a malicious external service truthful.

## Troubleshooting

- **Unavailable:** use **Test connection**. Check that the executable is on Factory's
  `PATH`, the HTTP URL is reachable, and every referenced environment variable exists.
- **Permission denied:** inspect the role and agent effective tool permissions. Add an
  explicit role allow; for write/destructive/unknown tools, name the exact tool.
- **Tool not shown:** run discovery, then check policy, Recipe selection, task
  operation, and objective relevance. Catalogs are intentionally bounded.
- **Invalid schema:** update the MCP server or its advertised JSON Schema. Factory
  refuses malformed schemas and inputs that do not validate.
- **Response too large:** reduce the server result. Raise `max_response_bytes` only
  after reviewing the exposure and never above the validated 1 MiB ceiling.
- **Timeout:** inspect server logs and adjust the call/discovery timeout within the
  configured bounds. Optional tool failures do not by themselves crash Factory.

Not included: a marketplace, automatic server downloads, a cloud proxy, general OAuth
brokerage, remote Factory agents, autonomous destructive approvals, or model-based
permission decisions.
