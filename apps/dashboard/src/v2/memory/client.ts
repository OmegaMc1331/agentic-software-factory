import { useEffect, useState } from "react";
import { request } from "../../api";
import { projectPath } from "../projects/projectApi";
import type { MemoryInput, MemoryRecord } from "./types";

export function mutation<T>(
  project: string,
  path: string,
  method: "POST" | "PATCH" | "PUT" | "DELETE",
  body?: unknown
) {
  return request<T>(
    projectPath(project, `memory${path}`),
    {
      method,
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body ?? {}),
    },
    { timeoutPolicy: "externalOperation" }
  );
}
export function saveMemory(
  project: string,
  memory: MemoryInput,
  reason: string,
  previous?: MemoryRecord
) {
  return mutation<MemoryRecord>(
    project,
    previous ? `/${encodeURIComponent(previous.id)}` : "",
    previous ? "PATCH" : "POST",
    {
      memory,
      reason,
      expectedRevision: previous?.revision ?? null,
    }
  );
}
export function useMemoryData<T>(project: string, path: string, version = 0) {
  const key = projectPath(project, `memory${path}`);
  const [state, setState] = useState<{ key: string; value?: T; error?: string }>({ key });
  useEffect(() => {
    const abort = new AbortController();
    void request<T>(key, undefined, { signal: abort.signal }).then(
      (value) => {
        if (!abort.signal.aborted) setState({ key, value });
      },
      (error: unknown) => {
        if (!abort.signal.aborted) setState({ key, error: String(error) });
      }
    );
    return () => abort.abort();
  }, [key, version]);
  return state.key === key ? state : { key };
}
