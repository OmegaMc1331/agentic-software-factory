import type {
  AgentSession,
  AgentStatusInfo,
  ConfigData,
  GraphData,
  GraphWorkspace,
  Run,
  RunDetail,
  RunSummary,
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

async function remove(path: string): Promise<void> {
  return request<void>(path, { method: "DELETE" });
}

export function fetchRuns(): Promise<RunSummary[]> {
  return get<RunSummary[]>("/runs");
}

export function fetchRun(id: number): Promise<RunDetail> {
  return get<RunDetail>(`/runs/${id}`);
}

export function createWorkflow(objective: string): Promise<Run> {
  return post<Run>("/runs", { objective });
}

export function startWorkflow(id: number): Promise<{ worker: string; reviewer: string }> {
  return post<{ worker: string; reviewer: string }>(`/runs/${id}/start`);
}

export function cancelWorkflow(id: number): Promise<void> {
  return post<void>(`/runs/${id}/cancel`);
}

export function retryTask(id: number): Promise<{ runId: number }> {
  return post<{ runId: number }>(`/tasks/${id}/retry`);
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
