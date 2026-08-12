import type { GraphEdge, GraphEdgeKind, GraphNode, GraphNodeKind } from "./types";
import { taskMeta } from "./types";

export const NET_NODE_HEIGHT = 46;
export const NET_VERTICAL_GAP = 54;
export const NET_LANE_GAP = 84;
export const NET_MARGIN = 40;

export const NODE_WIDTH: Record<GraphNodeKind, number> = {
  agent: 190,
  role: 150,
  run: 260,
  task: 230,
};

export const LANE_ORDER: GraphNodeKind[] = ["agent", "role", "run", "task"];

export const LANE_LABEL: Record<GraphNodeKind, string> = {
  agent: "agents",
  role: "roles",
  run: "runs",
  task: "tasks",
};

export interface NetworkNodePos {
  id: string;
  kind: GraphNodeKind;
  x: number;
  y: number;
  cx: number;
  cy: number;
  width: number;
}

export interface NetworkEdgePos {
  source: string;
  target: string;
  kind: GraphEdgeKind;
  path: string;
}

export interface NetworkLane {
  kind: GraphNodeKind;
  label: string;
  x: number;
}

export interface NetworkLayout {
  nodes: NetworkNodePos[];
  edges: NetworkEdgePos[];
  lanes: NetworkLane[];
  width: number;
  height: number;
}

function runIdOf(node: GraphNode): number {
  return Number(node.id.slice("run:".length)) || 0;
}

export function jitter(id: string, salt: number): number {
  let hash = 0;
  for (let i = 0; i < id.length; i += 1) {
    hash = (hash * 31 + id.charCodeAt(i)) >>> 0;
  }
  const span = salt === 2 ? 9 : 17;
  return ((hash + salt) % span) - Math.floor(span / 2);
}

function sortKind(a: GraphNode, b: GraphNode): number {
  if (a.kind === "task" && b.kind === "task") {
    const am = taskMeta(a);
    const bm = taskMeta(b);
    if (am.runId !== bm.runId) return am.runId - bm.runId;
    return am.position - bm.position;
  }
  if (a.kind === "run" && b.kind === "run") {
    return runIdOf(a) - runIdOf(b);
  }
  return a.label.localeCompare(b.label);
}

export function computeNetworkLayout(nodes: GraphNode[], edges: GraphEdge[]): NetworkLayout {
  const byKind = new Map<GraphNodeKind, GraphNode[]>();
  for (const kind of LANE_ORDER) {
    byKind.set(kind, []);
  }
  for (const node of nodes) {
    byKind.get(node.kind)?.push(node);
  }

  const axis: Record<GraphNodeKind, number> = { agent: 0, role: 0, run: 0, task: 0 };
  let cursor = NET_MARGIN;
  for (const kind of LANE_ORDER) {
    axis[kind] = cursor + NODE_WIDTH[kind] / 2;
    cursor += NODE_WIDTH[kind] + NET_LANE_GAP;
  }
  const width = cursor - NET_LANE_GAP + NET_MARGIN;

  let height = 0;
  for (const kind of LANE_ORDER) {
    const count = byKind.get(kind)?.length ?? 0;
    const laneHeight = count * (NET_NODE_HEIGHT + NET_VERTICAL_GAP) - NET_VERTICAL_GAP;
    height = Math.max(height, laneHeight);
  }
  height = Math.max(height, NET_NODE_HEIGHT);

  const positioned: NetworkNodePos[] = [];
  const byId = new Map<string, NetworkNodePos>();
  for (const kind of LANE_ORDER) {
    const lane = byKind.get(kind) ?? [];
    lane.sort(sortKind);
    const laneHeight = lane.length * (NET_NODE_HEIGHT + NET_VERTICAL_GAP) - NET_VERTICAL_GAP;
    const top = Math.max(0, (height - laneHeight) / 2);
    lane.forEach((node, index) => {
      const x = axis[kind] - NODE_WIDTH[kind] / 2 + jitter(node.id, 1);
      const y = top + index * (NET_NODE_HEIGHT + NET_VERTICAL_GAP) + jitter(node.id, 2);
      const pos: NetworkNodePos = {
        id: node.id,
        kind: node.kind,
        x,
        y,
        cx: x + NODE_WIDTH[kind] / 2,
        cy: y + NET_NODE_HEIGHT / 2,
        width: NODE_WIDTH[kind],
      };
      positioned.push(pos);
      byId.set(node.id, pos);
    });
  }

  function edgePath(source: NetworkNodePos, target: NetworkNodePos): string {
    if (source.kind === "task" && target.kind === "task") {
      const midY = (source.cy + target.cy) / 2;
      return `M ${source.cx},${source.cy} Q ${source.cx + 26},${midY} ${target.cx},${target.cy}`;
    }
    const bend = Math.max(40, Math.abs(target.cx - source.cx) * 0.4);
    return `M ${source.cx},${source.cy} C ${source.cx + bend},${source.cy} ${
      target.cx - bend
    },${target.cy} ${target.cx},${target.cy}`;
  }

  const edgePositions: NetworkEdgePos[] = [];
  for (const edge of edges) {
    const source = byId.get(edge.source);
    const target = byId.get(edge.target);
    if (!source || !target) continue;
    edgePositions.push({
      source: edge.source,
      target: edge.target,
      kind: edge.kind,
      path: edgePath(source, target),
    });
  }

  const lanes: NetworkLane[] = LANE_ORDER.map((kind) => ({
    kind,
    label: LANE_LABEL[kind],
    x: axis[kind],
  }));

  return {
    nodes: positioned,
    edges: edgePositions,
    lanes,
    width,
    height,
  };
}

export function neighborsOf(
  nodes: NetworkNodePos[],
  edges: NetworkEdgePos[],
  id: string
): string[] {
  const result = new Set<string>();
  for (const edge of edges) {
    if (edge.source === id) result.add(edge.target);
    if (edge.target === id) result.add(edge.source);
  }
  return Array.from(result).filter((neighbor) => nodes.some((node) => node.id === neighbor));
}
