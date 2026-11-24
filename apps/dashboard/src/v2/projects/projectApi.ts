import { request, type ApiRequestOptions } from "../../api";
import type { ConfigData, RunPage, RunSummary } from "../../types";
export interface Project {
  project_id: string;
  canonical_root: string;
  display_name: string;
  created_at: string;
  last_opened_at: string | null;
  registry_schema_version: number;
  lifecycle: "closed" | "opening" | "open" | "closing" | "unavailable";
  availability: "available" | "unavailable" | "not_initialized";
  problem: string | null;
}
export interface ApplicationInfo {
  default_project_id: string | null;
  authorized_roots: string[];
  limits: Record<string, number>;
}
export interface ProjectSnapshot {
  projectId: string;
  runs: RunSummary[];
  hasMore?: boolean;
  config: ConfigData;
}
export function projectPath(id: string, suffix = ""): string {
  if (!/^p_[0-9a-f]{32}$/.test(id)) throw new Error("Invalid project identity.");
  return `/projects/${id}${suffix ? `/${suffix}` : ""}`;
}
export const listProjects = (signal?: AbortSignal) =>
  request<Project[]>("/projects", undefined, { signal });
export const applicationInfo = (signal?: AbortSignal) =>
  request<ApplicationInfo>("/application", undefined, { signal });
export function projectMutation<T = Project>(
  path: string,
  method: "POST" | "PUT" | "DELETE",
  body?: unknown
): Promise<T> {
  return request<T>(
    path,
    {
      method,
      headers: body === undefined ? undefined : { "Content-Type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    },
    { timeoutPolicy: "externalOperation" }
  );
}
export async function projectSnapshot(
  projectId: string,
  options: ApiRequestOptions
): Promise<ProjectSnapshot> {
  const [runs, config] = await Promise.all([
    request<RunPage>(projectPath(projectId, "runs?limit=50"), undefined, options),
    request<ConfigData>(projectPath(projectId, "config"), undefined, options),
  ]);
  return { projectId, runs: runs.items, hasMore: runs.hasMore, config };
}
