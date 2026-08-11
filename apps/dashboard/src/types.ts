export type TaskState = "pending" | "ready" | "running" | "blocked" | "failed" | "completed";

export interface TaskCounts {
  pending: number;
  ready: number;
  running: number;
  blocked: number;
  failed: number;
  completed: number;
  total: number;
}

export interface RunSummary {
  id: number;
  objective: string;
  status: string;
  model: string | null;
  totalTokens: number;
  createdAt: string;
  counts: TaskCounts;
}

export interface Run {
  id: number;
  objective: string;
  status: string;
  model: string | null;
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  createdAt: string;
  updatedAt: string;
}

export interface Task {
  id: number;
  runId: number;
  title: string;
  objective: string;
  acceptanceCriteria: string[];
  state: TaskState;
  position: number;
  dependencies: number[];
  worktreePath: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface ModelUsage {
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
}

export interface RunDetail {
  run: Run;
  tasks: Task[];
  usage: ModelUsage;
}
