import type { TaskState } from "./types";

export const STATE_META: Record<TaskState, { label: string; color: string }> = {
  pending: { label: "pending", color: "#9ca3af" },
  ready: { label: "ready", color: "#2563eb" },
  running: { label: "running", color: "#d97706" },
  awaiting_integration: { label: "awaiting integration", color: "#7c3aed" },
  integrating: { label: "integrating", color: "#2563eb" },
  blocked: { label: "blocked", color: "#7c3aed" },
  failed: { label: "failed", color: "#dc2626" },
  completed: { label: "completed", color: "#16a34a" },
  superseded: { label: "superseded", color: "#6b7280" },
};
