import type { Task } from "./types";

export const NODE_WIDTH = 210;
export const NODE_HEIGHT = 44;
export const LEVEL_GAP = 70;
export const VERTICAL_GAP = 16;

export interface NodePosition {
  id: number;
  x: number;
  y: number;
  level: number;
}

export interface GraphEdge {
  source: number;
  target: number;
}

export interface GraphLayout {
  nodes: NodePosition[];
  edges: GraphEdge[];
  width: number;
  height: number;
}

export function computeLayout(tasks: Task[]): GraphLayout {
  const byId = new Map(tasks.map((t) => [t.id, t]));
  const level = new Map<number, number>();

  const assignLevel = (id: number): number => {
    const cached = level.get(id);
    if (cached !== undefined) return cached;
    const task = byId.get(id);
    if (!task) return 0;
    let max = 0;
    for (const dep of task.dependencies) {
      max = Math.max(max, assignLevel(dep) + 1);
    }
    level.set(id, max);
    return max;
  };

  for (const task of tasks) {
    assignLevel(task.id);
  }

  const byLevel = new Map<number, number[]>();
  for (const task of tasks) {
    const current = byLevel.get(level.get(task.id) ?? 0);
    const bucket = current ?? [];
    bucket.push(task.id);
    byLevel.set(level.get(task.id) ?? 0, bucket);
  }

  const nodes: NodePosition[] = [];
  for (const [bucketLevel, ids] of byLevel) {
    ids.forEach((id, index) => {
      nodes.push({
        id,
        level: bucketLevel,
        x: bucketLevel * (NODE_WIDTH + LEVEL_GAP),
        y: index * (NODE_HEIGHT + VERTICAL_GAP),
      });
    });
  }

  const edges: GraphEdge[] = [];
  for (const task of tasks) {
    for (const dep of task.dependencies) {
      edges.push({ source: dep, target: task.id });
    }
  }

  const levels = Array.from(byLevel.values());
  const maxNodes = Math.max(1, ...levels.map((ids) => ids.length));
  const maxLevel = Array.from(byLevel.keys()).reduce((acc, value) => Math.max(acc, value), 0);
  const width = (maxLevel > 0 ? maxLevel * (NODE_WIDTH + LEVEL_GAP) : 0) + NODE_WIDTH;
  const height = maxNodes * (NODE_HEIGHT + VERTICAL_GAP) - VERTICAL_GAP;

  return { nodes, edges, width, height };
}

export function truncate(value: string, max = 26): string {
  if (value.length <= max) return value;
  return `${value.slice(0, max - 1)}…`;
}
