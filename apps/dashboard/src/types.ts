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

export interface AgentEntry {
  command: string;
  args: string[];
  env: Record<string, string>;
  capabilities?: string[];
}

export interface ConfigData {
  agents: Record<string, AgentEntry>;
  roles: Record<string, { agent: string }>;
}

export interface AgentStatusInfo {
  name: string;
  command: string;
  args: string[];
  available: boolean;
}

export type GraphNodeKind = "agent" | "role" | "run" | "task" | "group" | "note";
export type GraphEdgeKind = "binds" | "uses" | "contains" | "depends" | "custom" | "membership";

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

export interface GroupMeta {
  visualOnly: true;
}

export interface NoteMeta {
  text: string;
  visualOnly: true;
}

export interface GraphNode {
  id: string;
  kind: GraphNodeKind;
  label: string;
  meta: AgentMeta | RoleMeta | RunMeta | TaskMeta | GroupMeta | NoteMeta;
}

export interface GraphEdge {
  id: string;
  source: string;
  target: string;
  kind: GraphEdgeKind;
  editable: boolean;
  semantic: "configuration" | "execution" | "system" | "visual";
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

export interface GraphPosition {
  x: number;
  y: number;
}

export interface WorkspaceNode {
  id: string;
  kind: "group" | "note";
  label: string;
  text?: string;
}

export interface WorkspaceEdge {
  id: string;
  source: string;
  target: string;
  kind: "custom" | "membership";
}

export interface GraphWorkspace {
  version: 1;
  nodes: Record<string, GraphPosition>;
  customNodes: WorkspaceNode[];
  edges: WorkspaceEdge[];
  warning?: string;
}

export interface AgentSession {
  id: number;
  runId: number | null;
  taskId: number | null;
  role: string;
  agent: string;
  command: string;
  status: string;
  startedAt: string;
  finishedAt: string | null;
  exitCode: number | null;
  durationMs: number | null;
  stdout: string | null;
  stderr: string | null;
  workingDirectory: string;
  interactive: boolean;
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

export interface AgentActivity {
  runId: number;
  taskId: number | null;
}

function runNodeId(node: GraphNode): number {
  return Number(node.id.slice("run:".length)) || 0;
}

export function agentActivity(
  agentId: string,
  nodesById: Map<string, GraphNode>,
  edges: GraphEdge[]
): AgentActivity | null {
  const roles = new Set<string>();
  for (const edge of edges) {
    if (edge.kind === "binds" && edge.target === agentId) roles.add(edge.source);
  }
  const relevant = (id: string) => id === agentId || roles.has(id);

  const activeRuns: GraphNode[] = [];
  for (const edge of edges) {
    if (edge.kind !== "uses" || !relevant(edge.target)) continue;
    const runNode = nodesById.get(edge.source);
    if (runNode && runNode.kind === "run" && runMeta(runNode).status === "active") {
      activeRuns.push(runNode);
    }
  }
  if (activeRuns.length === 0) return null;
  activeRuns.sort((a, b) => runNodeId(b) - runNodeId(a));
  const runId = runNodeId(activeRuns[0]);

  let taskId: number | null = null;
  for (const node of nodesById.values()) {
    if (node.kind !== "task") continue;
    const meta = taskMeta(node);
    if (
      meta.runId === runId &&
      meta.state === "running" &&
      (taskId === null || meta.taskId > taskId)
    ) {
      taskId = meta.taskId;
    }
  }
  return { runId, taskId };
}
