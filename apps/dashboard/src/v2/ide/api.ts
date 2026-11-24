import { request } from "../../api";
import { projectPath } from "../projects/projectApi";
import type { IdeState } from "./types";
export function ideRequest<T>(
  projectId: string,
  path: string,
  body?: unknown,
  signal?: AbortSignal
): Promise<T> {
  return request<T>(
    projectPath(projectId, `ide/${path}`),
    body === undefined
      ? { signal }
      : {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(body),
          signal,
        },
    { timeoutMs: 125_000, effect: body === undefined ? "read" : "write" }
  );
}
export const ideState = (projectId: string, signal?: AbortSignal) =>
  ideRequest<IdeState>(projectId, "state", undefined, signal);
