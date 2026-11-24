import { useEffect, useState } from "react";
import { request } from "../../api";
import { projectPath } from "../projects/projectApi";

export function sourceRequest<T>(
  project: string,
  path: string,
  body?: unknown,
  signal?: AbortSignal
) {
  return request<T>(
    projectPath(project, `source-control/${path}`),
    body === undefined
      ? undefined
      : {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(body),
        },
    { signal, timeoutPolicy: "externalOperation" }
  );
}
export function useSourceData<T>(
  project: string,
  path: string,
  version = 0,
  enabled = true
): { key: string; version?: number; value?: T; error?: string; loading: boolean } {
  const key = `${project}:${path}`;
  const [state, setState] = useState<{ key: string; version: number; value?: T; error?: string }>({
    key,
    version: -1,
  });
  useEffect(() => {
    if (!enabled) return;
    const controller = new AbortController();
    void sourceRequest<T>(project, path, undefined, controller.signal).then(
      (value) => {
        if (!controller.signal.aborted) setState({ key, version, value });
      },
      (error) => {
        if (!controller.signal.aborted)
          setState((current) => ({
            key,
            version,
            value: current.key === key ? current.value : undefined,
            error: String(error),
          }));
      }
    );
    return () => controller.abort();
  }, [project, path, key, version, enabled]);
  return state.key === key && enabled
    ? { ...state, loading: state.version !== version }
    : { key, loading: enabled };
}
