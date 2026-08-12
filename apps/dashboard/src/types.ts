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
  plannerAgent: string | null;
  createdAt: string;
  counts: TaskCounts;
}

export interface Run {
  id: number;
  objective: string;
  status: string;
  plannerAgent: string | null;
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

export interface RunDetail {
  run: Run;
  tasks: Task[];
}

export type GraphNodeKind = "agent" | "role" | "run" | "task";
export type GraphEdgeKind = "binds" | "uses" | "contains" | "depends";

export interface AgentMeta {
  command: string;
  available: boolean;
  roles: string[];
}

export interface RoleMeta {
  agent: string;
}

export interface RunMeta {
  objective: string;
  status: string;
  plannerAgent: string | null;
  createdAt: string;
  counts: TaskCounts;
}

export interface TaskMeta {
  taskId: number;
  runId: number;
  objective: string;
  state: TaskState;
  position: number;
  dependencies: number[];
  worktreePath: string | null;
}

export interface GraphNode {
  id: string;
  kind: GraphNodeKind;
  label: string;
  meta: AgentMeta | RoleMeta | RunMeta | TaskMeta;
}

export interface GraphEdge {
  source: string;
  target: string;
  kind: GraphEdgeKind;
}

export interface GraphMetadata {
  runs: number;
  tasks: number;
  agents: number;
  missingAgents: number;
  roles: number;
}

export interface GraphData {
  nodes: GraphNode[];
  edges: GraphEdge[];
  metadata: GraphMetadata;
}

export function agentMeta(node: GraphNode): AgentMeta {
  return node.meta as AgentMeta;
}

export function roleMeta(node: GraphNode): RoleMeta {
  return node.meta as RoleMeta;
}

export function runMeta(node: GraphNode): RunMeta {
  return node.meta as RunMeta;
}

export function taskMeta(node: GraphNode): TaskMeta {
  return node.meta as TaskMeta;
}
