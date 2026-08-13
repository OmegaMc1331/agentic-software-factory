import type { AgentStatusInfo, ConfigData, GraphData, RunDetail, RunSummary } from "./types";

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
