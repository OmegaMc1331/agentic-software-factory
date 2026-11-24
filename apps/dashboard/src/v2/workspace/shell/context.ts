import { createContext, useContext, useSyncExternalStore } from "react";
import type { WorkspaceController } from "../controller";

export const WorkspaceContext = createContext<WorkspaceController | null>(null);
export function useWorkspace() {
  const model = useContext(WorkspaceContext);
  if (!model) throw new Error("WorkspaceContext is required");
  return model;
}
export function useWorkspaceState() {
  const model = useWorkspace();
  return useSyncExternalStore(model.subscribe, model.getSnapshot);
}
