import type { AgentStatusInfo, ConfigData, GraphData, RunDetail, RunSummary } from "./types";

const API_BASE = "/api";

async function fail(response: Response, path: string): Promise<never> {
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
  throw new Error(detail || `request to ${path} failed with ${response.status}`);
}

async function get<T>(path: string): Promise<T> {
  const response = await fetch(`${API_BASE}${path}`);
  if (!response.ok) {
    return fail(response, path);
  }
  return response.json() as Promise<T>;
}

async function put<T>(path: string, body: unknown): Promise<T> {
  const response = await fetch(`${API_BASE}${path}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!response.ok) {
    return fail(response, path);
  }
  if (response.status === 204) return undefined as T;
  return response.json() as Promise<T>;
}

export function fetchRuns(): Promise<RunSummary[]> {
  return get<RunSummary[]>("/runs");
}

export function fetchRun(id: number): Promise<RunDetail> {
  return get<RunDetail>(`/runs/${id}`);
}

export function fetchGraph(): Promise<GraphData> {
  return get<GraphData>("/graph");
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

export function progress(counts: { completed: number; total: number }): number {
  if (counts.total === 0) return 0;
  return counts.completed / counts.total;
}
