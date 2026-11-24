import { useData, refreshData } from "../workspace/production/data";
import type { IdeState } from "./types";
export const useIde = (projectId: string) => useData<IdeState>(projectId, "ide/state");
export const refreshIde = (projectId: string) => refreshData(projectId, "ide/state");
