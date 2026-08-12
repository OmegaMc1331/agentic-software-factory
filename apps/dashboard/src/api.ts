import type { GraphData, RunDetail, RunSummary } from "./types";

const API_BASE = "/api";

async function get<T>(path: string): Promise<T> {
  const response = await fetch(`${API_BASE}${path}`);
  if (!response.ok) {
    throw new Error(`request to ${path} failed with ${response.status}`);
  }
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

export function progress(counts: { completed: number; total: number }): number {
  if (counts.total === 0) return 0;
  return counts.completed / counts.total;
}
