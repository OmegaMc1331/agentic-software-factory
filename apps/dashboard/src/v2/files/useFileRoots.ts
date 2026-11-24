import { useCallback, useSyncExternalStore } from "react";
import type { FileApi } from "./fileApi";
import type { RootCatalog } from "./types";
export function useFileRoots(api: FileApi, projectId: string) {
  return useSyncExternalStore(
    useCallback((listener) => api.catalogs.subscribe(projectId, listener), [api, projectId]),
    useCallback(() => api.catalogs.snapshot<RootCatalog>(projectId), [api, projectId])
  );
}
