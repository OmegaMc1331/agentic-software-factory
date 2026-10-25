import type {
  AgentPerformanceDetail,
  AgentSession,
  RoutingDecision,
  RoutingPreview,
  AgentStatusInfo,
  ConfigData,
  DeliveryReport,
  ExecutionClass,
  GitHubDeliveryRecord,
  GitHubStatus,
  GraphData,
  GraphWorkspace,
  McpServerConfig,
  McpServerView,
  McpTool,
  EffectiveToolPermission,
  PerformanceFilters,
  PerformanceOverview,
  PrPreview,
  Recipe,
  RecipeRoutingValue,
  RoleArtifact,
  RoleInfo,
  RolePolicyPreset,
  Run,
  RunDetail,
  RunPage,
  RunSummary,
  WorkflowTeam,
} from "./types";

const API_BASE = "/api";
const CSRF_HEADER = "X-Factory-CSRF";
const SESSION_TIMEOUT_MS = 5_000;
const MIN_REQUEST_TIMEOUT_MS = 100;
const MAX_REQUEST_TIMEOUT_MS = 605_000;
const MCP_DISCOVERY_OVERHEAD_MS = 5_000;
const DEFAULT_MCP_DISCOVERY_TIMEOUT_MS = 10_000;

export type ApiTimeoutPolicy = "default" | "discovery" | "externalOperation";

export const API_TIMEOUT_POLICIES = {
  default: 5_000,
  discovery: 15_000,
  externalOperation: 120_000,
} as const satisfies Record<ApiTimeoutPolicy, number>;

export type ApiErrorKind =
  | "http"
  | "network"
  | "timeout"
  | "cancelled"
  | "authentication"
  | "invalid-response"
  | "invalid-request";

export interface ApiRequestOptions {
  timeoutPolicy?: ApiTimeoutPolicy;
  timeoutMs?: number;
  signal?: AbortSignal;
  effect?: "read" | "write";
}

export class FactoryApiError extends Error {
  readonly kind: ApiErrorKind;
  readonly status: number | null;
  readonly uncertain: boolean;

  constructor(
    message: string,
    kind: ApiErrorKind,
    options: { status?: number | null; uncertain?: boolean } = {}
  ) {
    super(message);
    this.name = "FactoryApiError";
    this.kind = kind;
    this.status = options.status ?? null;
    this.uncertain = options.uncertain ?? false;
  }
}

export function isUncertainApiError(error: unknown): error is FactoryApiError {
  return error instanceof FactoryApiError && error.uncertain;
}

export function mcpDiscoveryRequestTimeoutMs(discoveryTimeoutMs?: number): number {
  const configured =
    typeof discoveryTimeoutMs === "number" && Number.isFinite(discoveryTimeoutMs)
      ? Math.max(0, discoveryTimeoutMs)
      : DEFAULT_MCP_DISCOVERY_TIMEOUT_MS;
  return Math.min(
    MAX_REQUEST_TIMEOUT_MS,
    Math.max(API_TIMEOUT_POLICIES.discovery, Math.ceil(configured) + MCP_DISCOVERY_OVERHEAD_MS)
  );
}

function timeoutError(uncertain: boolean): FactoryApiError {
  return new FactoryApiError(
    uncertain
      ? "The request timed out. The operation may still be running or may have completed. Check its status before retrying."
      : "Factory API did not respond before the request deadline. Check that `factory start` is running, then retry.",
    "timeout",
    { uncertain }
  );
}

function connectionError(uncertain: boolean): FactoryApiError {
  return new FactoryApiError(
    uncertain
      ? "Could not connect to the Factory API. Check that `factory start` is running; the operation may still have completed, so check its status before retrying."
      : "Could not connect to the Factory API. Check that `factory start` is running.",
    "network",
    { uncertain }
  );
}

function cancellationError(uncertain: boolean): FactoryApiError {
  return new FactoryApiError(
    uncertain
      ? "Request cancelled. The operation may still be running or may have completed. Check its status before retrying."
      : "Factory API request was cancelled.",
    "cancelled",
    { uncertain }
  );
}

function invalidResponseError(): FactoryApiError {
  return new FactoryApiError("Factory API returned an invalid response.", "invalid-response");
}

function resolveTimeoutMs(options?: ApiRequestOptions): number {
  const timeout = options?.timeoutMs ?? API_TIMEOUT_POLICIES[options?.timeoutPolicy ?? "default"];
  if (
    typeof timeout !== "number" ||
    !Number.isFinite(timeout) ||
    timeout < MIN_REQUEST_TIMEOUT_MS ||
    timeout > MAX_REQUEST_TIMEOUT_MS
  ) {
    throw new FactoryApiError(
      `Invalid Factory API timeout: expected ${MIN_REQUEST_TIMEOUT_MS}-${MAX_REQUEST_TIMEOUT_MS} ms.`,
      "invalid-request"
    );
  }
  return Math.floor(timeout);
}

function operationOptions(
  options: ApiRequestOptions | undefined,
  timeoutPolicy: ApiTimeoutPolicy,
  effect: "read" | "write"
): ApiRequestOptions {
  return {
    ...options,
    timeoutPolicy: options?.timeoutPolicy ?? timeoutPolicy,
    effect: options?.effect ?? effect,
  };
}

interface RequestDeadline {
  signal: AbortSignal;
  timedOut: () => boolean;
  cancelled: () => boolean;
  cleanup: () => void;
}

function createDeadline(
  timeoutMs: number,
  signals: Array<AbortSignal | null | undefined> = []
): RequestDeadline {
  const controller = new AbortController();
  const externalSignals = [
    ...new Set(
      signals.filter((signal): signal is AbortSignal => signal !== null && signal !== undefined)
    ),
  ];
  let timedOut = false;
  let cleaned = false;
  const onAbort = () => controller.abort();

  for (const signal of externalSignals) {
    if (signal.aborted) {
      controller.abort();
    } else {
      signal.addEventListener("abort", onAbort, { once: true });
    }
  }

  const timeout = window.setTimeout(() => {
    timedOut = true;
    controller.abort();
  }, timeoutMs);

  return {
    signal: controller.signal,
    timedOut: () => timedOut,
    cancelled: () => externalSignals.some((signal) => signal.aborted),
    cleanup: () => {
      if (cleaned) return;
      cleaned = true;
      window.clearTimeout(timeout);
      for (const signal of externalSignals) {
        signal.removeEventListener("abort", onAbort);
      }
    },
  };
}

function abortFailure(deadline: RequestDeadline, uncertain: boolean): FactoryApiError | null {
  if (deadline.cancelled()) return cancellationError(uncertain);
  if (deadline.timedOut()) return timeoutError(uncertain);
  return null;
}

function waitForSignal<T>(promise: Promise<T>, signal?: AbortSignal | null): Promise<T> {
  if (!signal) return promise;
  if (signal.aborted) return Promise.reject(cancellationError(false));

  return new Promise<T>((resolve, reject) => {
    const onAbort = () => {
      signal.removeEventListener("abort", onAbort);
      reject(cancellationError(false));
    };
    signal.addEventListener("abort", onAbort, { once: true });
    promise.then(
      (value) => {
        signal.removeEventListener("abort", onAbort);
        resolve(value);
      },
      (error) => {
        signal.removeEventListener("abort", onAbort);
        reject(error);
      }
    );
  });
}

let csrfToken: string | null = null;
let sessionRequest: Promise<string> | null = null;

async function establishSession(): Promise<string> {
  if (sessionRequest) return sessionRequest;

  const request = (async () => {
    const deadline = createDeadline(SESSION_TIMEOUT_MS);
    try {
      let response: Response;
      try {
        response = await fetch(`${API_BASE}/auth/session`, {
          method: "POST",
          credentials: "same-origin",
          headers: { "Content-Type": "application/json" },
          body: "{}",
          signal: deadline.signal,
        });
      } catch {
        if (deadline.timedOut()) throw timeoutError(false);
        throw connectionError(false);
      }

      if (!response.ok) {
        void response.body?.cancel().catch(() => {});
        throw new FactoryApiError(
          `Factory API session could not be established (HTTP ${response.status}). Restart \`factory start\` and reload the dashboard.`,
          "authentication",
          { status: response.status }
        );
      }

      let payload: { csrfToken?: unknown };
      try {
        payload = (await response.json()) as { csrfToken?: unknown };
      } catch {
        if (deadline.timedOut()) throw timeoutError(false);
        throw invalidResponseError();
      }
      if (typeof payload.csrfToken !== "string" || payload.csrfToken.length === 0) {
        throw invalidResponseError();
      }
      csrfToken = payload.csrfToken;
      return csrfToken;
    } finally {
      deadline.cleanup();
    }
  })();

  sessionRequest = request;
  void request
    .catch(() => {})
    .finally(() => {
      if (sessionRequest === request) sessionRequest = null;
    });
  return request;
}

/**
 * Establishes the same-origin browser session. API mutations call this
 * automatically; a cached token avoids a redundant bootstrap.
 */
export async function ensureFactorySession(signal?: AbortSignal | null): Promise<string> {
  if (csrfToken) return csrfToken;
  return waitForSignal(establishSession(), signal);
}

/**
 * Rotates the browser session before a long-lived transport connects. This
 * recovers cleanly from an expired session or a Factory restart even when a
 * CSRF token is still cached in memory.
 */
export async function refreshFactorySession(signal?: AbortSignal | null): Promise<string> {
  csrfToken = null;
  return waitForSignal(establishSession(), signal);
}

async function fail(response: Response): Promise<never> {
  let detail = "";
  try {
    const text = await response.text();
    try {
      detail = JSON.parse(text).error ?? text;
    } catch {
      detail = text;
    }
  } catch {
    detail = "";
  }
  const suffix = detail ? `: ${detail}` : "";
  throw new FactoryApiError(
    `Factory API request failed (HTTP ${response.status})${suffix}`,
    response.status === 401 || response.status === 403 ? "authentication" : "http",
    { status: response.status }
  );
}

async function request<T>(
  path: string,
  init?: RequestInit,
  options?: ApiRequestOptions
): Promise<T> {
  return (await requestFull<T>(path, init, options)).value;
}

async function requestFull<T>(
  path: string,
  init?: RequestInit,
  options?: ApiRequestOptions
): Promise<{ value: T; headers: Headers }> {
  const method = (init?.method ?? "GET").toUpperCase();
  const mutating = !["GET", "HEAD", "OPTIONS"].includes(method);
  const effect = options?.effect ?? (mutating ? "write" : "read");
  const uncertain = effect === "write";
  const timeoutMs = resolveTimeoutMs(options);
  const callerSignals = [init?.signal, options?.signal];
  const sessionSignal = options?.signal ?? init?.signal;

  for (let attempt = 0; attempt < 2; attempt += 1) {
    const headers = new Headers(init?.headers);
    if (mutating) headers.set(CSRF_HEADER, await ensureFactorySession(sessionSignal));

    const deadline = createDeadline(timeoutMs, callerSignals);
    try {
      let response: Response;
      try {
        response = await fetch(`${API_BASE}${path}`, {
          ...init,
          method,
          headers,
          credentials: "same-origin",
          signal: deadline.signal,
        });
      } catch {
        throw abortFailure(deadline, uncertain) ?? connectionError(uncertain);
      }

      if ((response.status === 401 || response.status === 403) && attempt === 0) {
        void response.body?.cancel().catch(() => {});
        csrfToken = null;
        await waitForSignal(establishSession(), sessionSignal);
        continue;
      }

      if (!response.ok) {
        try {
          return await fail(response);
        } catch (error) {
          throw abortFailure(deadline, uncertain) ?? error;
        }
      }
      if (response.status === 204) return { value: undefined as T, headers: response.headers };
      try {
        const value = (await response.json()) as T;
        return { value, headers: response.headers };
      } catch {
        throw abortFailure(deadline, uncertain) ?? invalidResponseError();
      }
    } finally {
      deadline.cleanup();
    }
  }

  throw new FactoryApiError("Factory API session could not be established.", "authentication");
}

async function get<T>(path: string, options?: ApiRequestOptions): Promise<T> {
  return request<T>(path, undefined, options);
}

async function put<T>(path: string, body: unknown, options?: ApiRequestOptions): Promise<T> {
  return request<T>(
    path,
    {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    },
    options
  );
}

async function post<T>(path: string, body?: unknown, options?: ApiRequestOptions): Promise<T> {
  return request<T>(
    path,
    {
      method: "POST",
      headers: body === undefined ? undefined : { "Content-Type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    },
    options
  );
}

async function remove<T = void>(path: string, options?: ApiRequestOptions): Promise<T> {
  return request<T>(path, { method: "DELETE" }, options);
}

export const RUN_PAGE_SIZE = 25;

export function fetchRuns(options?: ApiRequestOptions): Promise<RunSummary[]> {
  return get<RunSummary[]>("/runs", options);
}

export interface RunPageRequest {
  limit?: number;
  before?: number | null;
}

export function fetchRunsPage(
  page: RunPageRequest = {},
  options?: ApiRequestOptions
): Promise<RunPage> {
  const params = new URLSearchParams({ limit: String(page.limit ?? RUN_PAGE_SIZE) });
  if (page.before != null) params.set("before", String(page.before));
  return get<RunPage>(`/runs?${params.toString()}`, options);
}

export function fetchRun(id: number, options?: ApiRequestOptions): Promise<RunDetail> {
  return get<RunDetail>(`/runs/${id}`, options);
}

export function createWorkflow(
  objective: string,
  team?: WorkflowTeam,
  recipe?: string,
  options?: ApiRequestOptions
): Promise<Run> {
  const body: Record<string, unknown> = { objective };
  if (team !== undefined) body.team = team;
  if (recipe !== undefined) body.recipe = recipe;
  return post<Run>("/runs", body, options);
}

/** Imports a GitHub Issue as a workflow. Planning starts; execution does not. */
export function createWorkflowFromIssue(
  issue: string,
  team?: WorkflowTeam,
  recipe?: string,
  options?: ApiRequestOptions
): Promise<Run> {
  const body: Record<string, unknown> = { issue };
  if (team !== undefined) body.team = team;
  if (recipe !== undefined) body.recipe = recipe;
  return post<Run>(
    "/runs/from-issue",
    body,
    operationOptions(options, "externalOperation", "write")
  );
}

// --- Recipes -------------------------------------------------------------------

/** The editable recipe form shared by create/update and the Settings editor. */
export interface RecipeSaveRequest {
  id?: string;
  name: string;
  description?: string;
  roles?: string[];
  routing?: RecipeRoutingValue;
  maxParallelTasks?: number | null;
  policyPresets?: Record<string, string>;
  plannerGuidance?: string;
  tools?: string[];
  github?: { draft: boolean } | null;
}

export function fetchRecipes(options?: ApiRequestOptions): Promise<Recipe[]> {
  return get<Recipe[]>("/recipes", options);
}

export function createRecipe(
  request: RecipeSaveRequest,
  options?: ApiRequestOptions
): Promise<Recipe> {
  return post<Recipe>("/recipes", request, options);
}

export function updateRecipe(
  id: string,
  request: RecipeSaveRequest,
  options?: ApiRequestOptions
): Promise<Recipe> {
  return put<Recipe>(`/recipes/${encodeURIComponent(id)}`, request, options);
}

export function deleteRecipe(id: string, options?: ApiRequestOptions): Promise<void> {
  return remove(`/recipes/${encodeURIComponent(id)}`, options);
}

/**
 * "Save as Recipe": derives a reusable recipe from a workflow's current
 * configuration (roles, routing, concurrency, policy presets, guidance).
 * Tasks, DAG edges, attempts, and sessions are never copied.
 */
export function saveRunAsRecipe(
  runId: number,
  request: { name: string; description?: string; id?: string },
  options?: ApiRequestOptions
): Promise<Recipe> {
  return post<Recipe>(`/runs/${runId}/recipe`, request, options);
}

// --- GitHub delivery ---------------------------------------------------------

/** `gh auth status` + remote detection. Semantic read only. */
export function fetchGithubStatus(options?: ApiRequestOptions): Promise<GitHubStatus> {
  return get<GitHubStatus>(
    "/github/status",
    operationOptions(options, "externalOperation", "read")
  );
}

export function fetchDelivery(runId: number, options?: ApiRequestOptions): Promise<DeliveryReport> {
  return get<DeliveryReport>(`/runs/${runId}/delivery`, options);
}

/** The editable pull request preview shown before creation. */
export function fetchPrPreview(runId: number, options?: ApiRequestOptions): Promise<PrPreview> {
  return get<PrPreview>(
    `/runs/${runId}/pr-preview`,
    operationOptions(options, "externalOperation", "read")
  );
}

/**
 * The Factory-owned delivery action: pushes `factory/run-<id>` and creates
 * (or links an existing) pull request. Titles and bodies are passed as JSON
 * values, never interpolated into commands.
 */
export function createPullRequest(
  runId: number,
  request: { title?: string; body?: string; draft?: boolean },
  options?: ApiRequestOptions
): Promise<GitHubDeliveryRecord> {
  return post<GitHubDeliveryRecord>(
    `/runs/${runId}/pull-request`,
    request,
    operationOptions(options, "externalOperation", "write")
  );
}

export function updateWorkflowTeam(
  runId: number,
  team: WorkflowTeam,
  options?: ApiRequestOptions
): Promise<WorkflowTeam> {
  return put<WorkflowTeam>(`/runs/${runId}/team`, team, options);
}

export function startWorkflow(id: number, options?: ApiRequestOptions): Promise<WorkflowTeam> {
  return post<WorkflowTeam>(`/runs/${id}/start`, undefined, options);
}

export function cancelWorkflow(id: number, options?: ApiRequestOptions): Promise<void> {
  return post<void>(`/runs/${id}/cancel`, undefined, options);
}

export function retryTask(id: number, options?: ApiRequestOptions): Promise<{ runId: number }> {
  return post<{ runId: number }>(`/tasks/${id}/retry`, undefined, options);
}

export function fetchRunArtifacts(
  runId: number,
  options?: ApiRequestOptions
): Promise<RoleArtifact[]> {
  return get<RoleArtifact[]>(`/runs/${runId}/artifacts`, options);
}

export function fetchTaskArtifacts(
  taskId: number,
  options?: ApiRequestOptions
): Promise<RoleArtifact[]> {
  return get<RoleArtifact[]>(`/tasks/${taskId}/artifacts`, options);
}

// --- Routing ---------------------------------------------------------------

export function fetchRoutingPreview(
  taskId: number,
  options?: ApiRequestOptions
): Promise<RoutingPreview> {
  return get<RoutingPreview>(`/tasks/${taskId}/routing-preview`, options);
}

export function fetchRoutingDecisions(
  taskId: number,
  options?: ApiRequestOptions
): Promise<RoutingDecision[]> {
  return get<RoutingDecision[]>(`/tasks/${taskId}/routing-decisions`, options);
}

/** Pins (`agent`) or clears (`null`) a task's manual routing override. */
export function setTaskRouting(
  taskId: number,
  agent: string | null,
  options?: ApiRequestOptions
): Promise<{ taskId: number; agentOverride: string | null }> {
  return put(`/tasks/${taskId}/routing`, { agent }, options);
}

export function fetchGraph(options?: ApiRequestOptions): Promise<GraphData> {
  return get<GraphData>("/graph", options);
}

export function fetchGraphWorkspace(options?: ApiRequestOptions): Promise<GraphWorkspace> {
  return get<GraphWorkspace>("/graph/workspace", options);
}

export function saveGraphWorkspace(
  workspace: GraphWorkspace,
  options?: ApiRequestOptions
): Promise<void> {
  const body: GraphWorkspace = {
    version: workspace.version,
    nodes: workspace.nodes,
    customNodes: workspace.customNodes,
    edges: workspace.edges,
  };
  return put<void>("/graph/workspace", body, options);
}

// --- Configuration ------------------------------------------------------------

/**
 * A configuration snapshot together with the server revision it was loaded
 * at. The revision backs the If-Match precondition on full-document saves.
 */
export interface ConfigSnapshot {
  config: ConfigData;
  revision: string;
}

let configRevision: string | null = null;

function revisionFromHeaders(headers: Headers): string | null {
  return headers.get("ETag") ?? headers.get("etag");
}

/** Loads the configuration and retains its revision for the next save. */
export async function fetchConfigWithRevision(
  options?: ApiRequestOptions
): Promise<ConfigSnapshot> {
  const { value, headers } = await requestFull<ConfigData>("/config", undefined, options);
  const revision = revisionFromHeaders(headers);
  if (!revision) {
    throw new FactoryApiError(
      "Factory API returned no configuration revision.",
      "invalid-response"
    );
  }
  configRevision = revision;
  return { config: value, revision };
}

/** The revision of the most recently loaded configuration, if any. */
export function currentConfigRevision(): string | null {
  return configRevision;
}

export function fetchConfig(options?: ApiRequestOptions): Promise<ConfigData> {
  return fetchConfigWithRevision(options).then((snapshot) => snapshot.config);
}

/**
 * Whether an error is a configuration revision conflict (HTTP 412) or a
 * missing-precondition rejection (HTTP 428). Callers must reload the latest
 * configuration and reapply their changes instead of retrying blindly.
 */
export function isConfigConflictError(error: unknown): boolean {
  return error instanceof FactoryApiError && (error.status === 412 || error.status === 428);
}

/**
 * Replaces the full configuration. The server enforces the revision captured
 * by the last `fetchConfig`/`fetchConfigWithRevision` call: a stale snapshot
 * is rejected with HTTP 412 instead of silently overwriting newer changes.
 */
export async function saveConfig(config: ConfigData, options?: ApiRequestOptions): Promise<void> {
  const revision = configRevision;
  if (!revision) {
    throw new FactoryApiError("Load the configuration before saving it.", "invalid-request");
  }
  const { headers } = await requestFull<void>(
    "/config",
    {
      method: "PUT",
      headers: { "Content-Type": "application/json", "If-Match": revision },
      body: JSON.stringify(config),
    },
    options
  );
  const next = revisionFromHeaders(headers);
  if (next) configRevision = next;
}

// --- External tools / MCP ----------------------------------------------------

export function fetchMcpServers(options?: ApiRequestOptions): Promise<McpServerView[]> {
  return get<McpServerView[]>("/mcp/servers", options);
}

export function createMcpServer(
  id: string,
  config: McpServerConfig,
  options?: ApiRequestOptions
): Promise<McpServerView> {
  return post<McpServerView>("/mcp/servers", { id, ...config }, options);
}

export function updateMcpServer(
  id: string,
  config: McpServerConfig,
  options?: ApiRequestOptions
): Promise<McpServerView> {
  return put<McpServerView>(`/mcp/servers/${encodeURIComponent(id)}`, config, options);
}

export function deleteMcpServer(id: string, options?: ApiRequestOptions): Promise<void> {
  return remove(`/mcp/servers/${encodeURIComponent(id)}`, options);
}

export function testMcpServer(id: string, options?: ApiRequestOptions): Promise<McpServerView> {
  return post<McpServerView>(
    `/mcp/servers/${encodeURIComponent(id)}/test`,
    undefined,
    operationOptions(options, "discovery", "read")
  );
}

export function fetchMcpTools(id: string, options?: ApiRequestOptions): Promise<McpTool[]> {
  return get<McpTool[]>(
    `/mcp/servers/${encodeURIComponent(id)}/tools`,
    operationOptions(options, "discovery", "read")
  );
}

export function fetchRoleTools(
  roleId: string,
  options?: ApiRequestOptions
): Promise<EffectiveToolPermission[]> {
  return get<EffectiveToolPermission[]>(`/roles/${encodeURIComponent(roleId)}/tools`, options);
}

export function fetchAgentTools(
  agent: string,
  options?: ApiRequestOptions
): Promise<EffectiveToolPermission[]> {
  return get<EffectiveToolPermission[]>(`/agents/${encodeURIComponent(agent)}/tools`, options);
}

export function fetchAgents(options?: ApiRequestOptions): Promise<AgentStatusInfo[]> {
  return get<AgentStatusInfo[]>("/agents", options);
}

export function fetchRoles(options?: ApiRequestOptions): Promise<RoleInfo[]> {
  return get<RoleInfo[]>("/roles", options);
}

export interface RoleCreateRequest {
  id?: string;
  name: string;
  description: string;
  executionClass: ExecutionClass;
  instructions?: string;
  agents?: string[];
  preferredAgent?: string;
  /** Policy preset: what Factory permits the role to do (not its instructions). */
  policyPreset?: RolePolicyPreset;
}

export function createRole(
  request: RoleCreateRequest,
  options?: ApiRequestOptions
): Promise<RoleInfo> {
  return post<RoleInfo>("/roles", request, options);
}

export function updateRole(
  id: string,
  body: {
    name: string;
    description: string;
    executionClass: ExecutionClass;
    instructions?: string;
    policyPreset?: RolePolicyPreset;
  },
  options?: ApiRequestOptions
): Promise<RoleInfo> {
  return put<RoleInfo>(`/roles/${encodeURIComponent(id)}`, body, options);
}

/**
 * Sets (or clears with null) a role's policy preset. Works for core and custom
 * roles: the policy is Factory's enforcement boundary, independent of the
 * role's instructions.
 */
export function setRolePolicy(
  roleId: string,
  preset: RolePolicyPreset | null,
  options?: ApiRequestOptions
): Promise<RoleInfo> {
  return put<RoleInfo>(`/roles/${encodeURIComponent(roleId)}/policy`, { preset }, options);
}

export function deleteRole(id: string, options?: ApiRequestOptions): Promise<void> {
  return remove(`/roles/${encodeURIComponent(id)}`, options);
}

export function addRoleAssignment(
  roleId: string,
  agent: string,
  preferred = false,
  options?: ApiRequestOptions
): Promise<RoleInfo> {
  return post<RoleInfo>(
    `/roles/${encodeURIComponent(roleId)}/assignments`,
    { agent, preferred },
    options
  );
}

export function removeRoleAssignment(
  roleId: string,
  agent: string,
  options?: ApiRequestOptions
): Promise<RoleInfo> {
  return remove<RoleInfo>(
    `/roles/${encodeURIComponent(roleId)}/assignments/${encodeURIComponent(agent)}`,
    options
  );
}

export function setPreferredAssignment(
  roleId: string,
  agent: string,
  options?: ApiRequestOptions
): Promise<RoleInfo> {
  return put<RoleInfo>(`/roles/${encodeURIComponent(roleId)}/preferred`, { agent }, options);
}

export function fetchAgentSessions(
  agent: string,
  options?: ApiRequestOptions
): Promise<AgentSession[]> {
  return get<AgentSession[]>(`/agents/${encodeURIComponent(agent)}/sessions`, options);
}

export function startInteractiveAgentSession(
  agent: string,
  size: { cols: number; rows: number } = { cols: 100, rows: 28 },
  options?: ApiRequestOptions
): Promise<AgentSession> {
  return post<AgentSession>(`/agents/${encodeURIComponent(agent)}/sessions`, size, options);
}

export function stopInteractiveAgentSession(
  id: number,
  options?: ApiRequestOptions
): Promise<void> {
  return remove(`/sessions/${id}`, options);
}

export function fetchAgentSession(id: number, options?: ApiRequestOptions): Promise<AgentSession> {
  return get<AgentSession>(`/sessions/${id}`, options);
}

export function agentSessionStreamUrl(id: number): string {
  return `${API_BASE}/sessions/${id}/stream`;
}

export function agentTerminalSocketUrl(id: number): string {
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  return `${protocol}//${window.location.host}${API_BASE}/sessions/${id}/terminal`;
}

export function progress(counts: { completed: number; total: number }): number {
  if (counts.total === 0) return 0;
  return counts.completed / counts.total;
}

// --- Performance evaluation --------------------------------------------------

export function fetchPerformanceOverview(
  filters: PerformanceFilters = {},
  options?: ApiRequestOptions
): Promise<PerformanceOverview> {
  return get<PerformanceOverview>(`/performance/agents${performanceQuery(filters)}`, options);
}

export function fetchAgentPerformance(
  agent: string,
  filters: PerformanceFilters = {},
  options?: ApiRequestOptions
): Promise<AgentPerformanceDetail> {
  return get<AgentPerformanceDetail>(
    `/performance/agents/${encodeURIComponent(agent)}${performanceQuery(filters)}`,
    options
  );
}

function performanceQuery(filters: PerformanceFilters): string {
  const params = new URLSearchParams();
  for (const key of ["window", "role", "operation", "language"] as const) {
    const value = filters[key];
    if (value) params.set(key, value);
  }
  const query = params.toString();
  return query ? `?${query}` : "";
}
