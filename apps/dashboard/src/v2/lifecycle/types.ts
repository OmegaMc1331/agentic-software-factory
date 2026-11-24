export interface MissionLifecycle {
  schemaVersion: number;
  projectId: string;
  workspaceSessionId: string;
  runId: number;
  generation: number;
  state:
    | "active"
    | "suspend_requested"
    | "quiescing"
    | "suspended"
    | "reconciling"
    | "recovery_required"
    | "settled";
  reason: string | null;
  checkpointId: number | null;
  problem: string | null;
  updatedAt: string;
}
export interface SessionLifecycle {
  projectId: string;
  sessionId: string;
  generation: number;
  closed: boolean;
  missions: MissionLifecycle[];
  events?: Array<{ eventType: string; generation: number; timestamp: string }>;
}
