import type {
  AgentPerformanceDetail,
  AgentSession,
  AgentStatusInfo,
  ConfigData,
  DeliveryReport,
  ExecutionClass,
  GitHubDeliveryRecord,
  GitHubStatus,
  GraphData,
  GraphWorkspace,
  PerformanceFilters,
  PerformanceOverview,
  PrPreview,
  RoleArtifact,
  RoleInfo,
  RolePolicyPreset,
  Run,
  RunDetail,
  RunSummary,
  WorkflowTeam,
} from "./types";

const API_BASE = "/api";
const REQUEST_TIMEOUT_MS = 5000;

function timeoutError(): Error {
  return new Error("Factory API did not respond. Check that `factory start` is still running.");
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
  throw new Error(`Factory API request failed (HTTP ${response.status})${suffix}`);
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const controller = new AbortController();
  const timeout = window.setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);

  try {
    let response: Response;
    try {
      response = await fetch(`${API_BASE}${path}`, { ...init, signal: controller.signal });
    } catch {
      if (controller.signal.aborted) throw timeoutError();
      throw new Error(
        "Could not connect to the Factory API. Check that `factory start` is running."
      );
    }

    if (!response.ok) {
      try {
        return await fail(response);
      } catch (error) {
        if (controller.signal.aborted) throw timeoutError();
        throw error;
      }
    }
    if (response.status === 204) return undefined as T;
    try {
      return (await response.json()) as T;
    } catch {
      if (controller.signal.aborted) throw timeoutError();
      throw new Error("Factory API returned an invalid response.");
    }
  } finally {
    window.clearTimeout(timeout);
  }
}

async function get<T>(path: string): Promise<T> {
  return request<T>(path);
}

async function put<T>(path: string, body: unknown): Promise<T> {
  return request<T>(path, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}

async function post<T>(path: string, body?: unknown): Promise<T> {
  return request<T>(path, {
    method: "POST",
    headers: body === undefined ? undefined : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
}

async function remove<T = void>(path: string): Promise<T> {
  return request<T>(path, { method: "DELETE" });
}

export function fetchRuns(): Promise<RunSummary[]> {
  return get<RunSummary[]>("/runs");
}

export function fetchRun(id: number): Promise<RunDetail> {
  return get<RunDetail>(`/runs/${id}`);
}

export function createWorkflow(objective: string, team?: WorkflowTeam): Promise<Run> {
  return post<Run>("/runs", team === undefined ? { objective } : { objective, team });
}

/** Imports a GitHub Issue as a workflow. Planning starts; execution does not. */
export function createWorkflowFromIssue(issue: string, team?: WorkflowTeam): Promise<Run> {
  return post<Run>("/runs/from-issue", team === undefined ? { issue } : { issue, team });
}

// --- GitHub delivery ---------------------------------------------------------

/** `gh auth status` + remote detection. Semantic read only. */
export function fetchGithubStatus(): Promise<GitHubStatus> {
  return get<GitHubStatus>("/github/status");
}

export function fetchDelivery(runId: number): Promise<DeliveryReport> {
  return get<DeliveryReport>(`/runs/${runId}/delivery`);
}

/** The editable pull request preview shown before creation. */
export function fetchPrPreview(runId: number): Promise<PrPreview> {
  return get<PrPreview>(`/runs/${runId}/pr-preview`);
}

/**
 * The Factory-owned delivery action: pushes `factory/run-<id>` and creates
 * (or links an existing) pull request. Titles and bodies are passed as JSON
 * values, never interpolated into commands.
 */
export function createPullRequest(
  runId: number,
  request: { title?: string; body?: string; draft?: boolean }
): Promise<GitHubDeliveryRecord> {
  return post<GitHubDeliveryRecord>(`/runs/${runId}/pull-request`, request);
}

export function updateWorkflowTeam(runId: number, team: WorkflowTeam): Promise<WorkflowTeam> {
  return put<WorkflowTeam>(`/runs/${runId}/team`, team);
}

export function startWorkflow(id: number): Promise<WorkflowTeam> {
  return post<WorkflowTeam>(`/runs/${id}/start`);
}

export function cancelWorkflow(id: number): Promise<void> {
  return post<void>(`/runs/${id}/cancel`);
}

export function retryTask(id: number): Promise<{ runId: number }> {
  return post<{ runId: number }>(`/tasks/${id}/retry`);
}

export function fetchRunArtifacts(runId: number): Promise<RoleArtifact[]> {
  return get<RoleArtifact[]>(`/runs/${runId}/artifacts`);
}

export function fetchTaskArtifacts(taskId: number): Promise<RoleArtifact[]> {
  return get<RoleArtifact[]>(`/tasks/${taskId}/artifacts`);
}

export function fetchGraph(): Promise<GraphData> {
  return get<GraphData>("/graph");
}

export function fetchGraphWorkspace(): Promise<GraphWorkspace> {
  return get<GraphWorkspace>("/graph/workspace");
}

export function saveGraphWorkspace(workspace: GraphWorkspace): Promise<void> {
  const body: GraphWorkspace = {
    version: workspace.version,
    nodes: workspace.nodes,
    customNodes: workspace.customNodes,
    edges: workspace.edges,
  };
  return put<void>("/graph/workspace", body);
}

export function fetchConfig(): Promise<ConfigData> {
  return get<ConfigData>("/config");
}

export function saveConfig(config: ConfigData): Promise<void> {
  return put<void>("/config", config);
}

export function fetchAgents(): Promise<AgentStatusInfo[]> {
  return get<AgentStatusInfo[]>("/agents");
}

export function fetchRoles(): Promise<RoleInfo[]> {
  return get<RoleInfo[]>("/roles");
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

export function createRole(request: RoleCreateRequest): Promise<RoleInfo> {
  return post<RoleInfo>("/roles", request);
}

export function updateRole(
  id: string,
  body: {
    name: string;
    description: string;
    executionClass: ExecutionClass;
    instructions?: string;
    policyPreset?: RolePolicyPreset;
  }
): Promise<RoleInfo> {
  return put<RoleInfo>(`/roles/${encodeURIComponent(id)}`, body);
}

/**
 * Sets (or clears with null) a role's policy preset. Works for core and custom
 * roles: the policy is Factory's enforcement boundary, independent of the
 * role's instructions.
 */
export function setRolePolicy(roleId: string, preset: RolePolicyPreset | null): Promise<RoleInfo> {
  return put<RoleInfo>(`/roles/${encodeURIComponent(roleId)}/policy`, { preset });
}

export function deleteRole(id: string): Promise<void> {
  return remove(`/roles/${encodeURIComponent(id)}`);
}

export function addRoleAssignment(
  roleId: string,
  agent: string,
  preferred = false
): Promise<RoleInfo> {
  return post<RoleInfo>(`/roles/${encodeURIComponent(roleId)}/assignments`, { agent, preferred });
}

export function removeRoleAssignment(roleId: string, agent: string): Promise<RoleInfo> {
  return remove<RoleInfo>(
    `/roles/${encodeURIComponent(roleId)}/assignments/${encodeURIComponent(agent)}`
  );
}

export function setPreferredAssignment(roleId: string, agent: string): Promise<RoleInfo> {
  return put<RoleInfo>(`/roles/${encodeURIComponent(roleId)}/preferred`, { agent });
}

export function fetchAgentSessions(agent: string): Promise<AgentSession[]> {
  return get<AgentSession[]>(`/agents/${encodeURIComponent(agent)}/sessions`);
}

export function startInteractiveAgentSession(
  agent: string,
  size: { cols: number; rows: number } = { cols: 100, rows: 28 }
): Promise<AgentSession> {
  return post<AgentSession>(`/agents/${encodeURIComponent(agent)}/sessions`, size);
}

export function stopInteractiveAgentSession(id: number): Promise<void> {
  return remove(`/sessions/${id}`);
}

export function fetchAgentSession(id: number): Promise<AgentSession> {
  return get<AgentSession>(`/sessions/${id}`);
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
  filters: PerformanceFilters = {}
): Promise<PerformanceOverview> {
  return get<PerformanceOverview>(`/performance/agents${performanceQuery(filters)}`);
}

export function fetchAgentPerformance(
  agent: string,
  filters: PerformanceFilters = {}
): Promise<AgentPerformanceDetail> {
  return get<AgentPerformanceDetail>(
    `/performance/agents/${encodeURIComponent(agent)}${performanceQuery(filters)}`
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
