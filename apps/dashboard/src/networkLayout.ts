import type { GraphEdge, GraphEdgeKind, GraphNode, GraphNodeKind } from "./types";
import { taskMeta } from "./types";

export interface NetworkNodePos {
  id: string;
  kind: GraphNodeKind;
  x: number;
  y: number;
  cx: number;
  cy: number;
  width: number;
  height: number;
}

export interface NetworkEdgePos {
  source: string;
  target: string;
  kind: GraphEdgeKind;
  path: string;
}

export interface NetworkLayout {
  nodes: NetworkNodePos[];
  edges: NetworkEdgePos[];
  width: number;
  height: number;
}

export const MIN_CANVAS_WIDTH = 960;
export const MIN_CANVAS_HEIGHT = 620;
export const BOUNDS_PADDING = 150;

const EDGE_REST: Record<GraphEdgeKind, number> = {
  binds: 150,
  plans: 175,
  works: 150,
  reviews: 165,
  contains: 135,
  depends: 105,
  custom: 190,
  membership: 130,
  originates: 150,
  delivers: 150,
};

const ANCHOR: Record<GraphNodeKind, number> = {
  agent: 0.115,
  role: 0.08,
  run: 0.17,
  task: 0.05,
  group: 0.04,
  note: 0.05,
  github_issue: 0.06,
  github_pr: 0.06,
};

const ITERATIONS = 180;
const REPULSION = 26000;
const GRAVITY = 0.004;

const AGENT_RING_RADIUS = 165;
const AGENT_RING_CY = 10;
const ROLE_BAND_Y = -160;
const RUN_BAND_Y = 175;
const RUN_BAND_GAP = 300;
const TASK_FAN_RADIUS = 140;

function nodeSize(node: GraphNode): { width: number; height: number } {
  switch (node.kind) {
    case "agent":
      return { width: 130, height: 110 };
    case "role":
      return { width: Math.max(86, node.label.length * 6 + 28), height: 24 };
    case "run":
      return { width: Math.max(178, Math.min(250, node.label.length * 6.2 + 56)), height: 88 };
    case "task":
      return { width: 134, height: 22 };
    case "group":
      return { width: 230, height: 150 };
    case "note":
      return { width: 154, height: 72 };
    case "github_issue":
      return { width: Math.max(150, Math.min(220, node.label.length * 6 + 40)), height: 54 };
    case "github_pr":
      return { width: 128, height: 54 };
  }
}

export function jitter(id: string, spread = 1): number {
  let hash = 0;
  for (let i = 0; i < id.length; i += 1) {
    hash = (hash * 31 + id.charCodeAt(i)) >>> 0;
  }
  const span = 19;
  return ((((hash + spread * 7) % span) - (span - 1) / 2) / 10) * spread;
}

function runIdOf(node: GraphNode): number {
  if (node.kind === "run") return Number(node.id.slice("run:".length)) || 0;
  if (node.kind === "task") return taskMeta(node).runId;
  if (node.kind === "github_issue" || node.kind === "github_pr") {
    return Number(node.id.split(":")[1]) || 0;
  }
  return 0;
}

interface Body {
  id: string;
  kind: GraphNodeKind;
  x: number;
  y: number;
  width: number;
  height: number;
  houseX: number;
  houseY: number;
}

function homes(nodes: GraphNode[], edges: GraphEdge[]): Map<string, { x: number; y: number }> {
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const result = new Map<string, { x: number; y: number }>();

  const agents = nodes.filter((n) => n.kind === "agent");
  agents.forEach((node, index) => {
    const angle = (Math.PI * 2 * index) / Math.max(1, agents.length) + jitter(node.id) * 0.22 - 0.5;
    result.set(node.id, {
      x: Math.cos(angle) * AGENT_RING_RADIUS,
      y: AGENT_RING_CY + Math.sin(angle) * AGENT_RING_RADIUS * 0.72,
    });
  });

  const agentLookup = new Map<string, { x: number; y: number }>();
  for (const [agentId, pos] of result) {
    if (byId.get(agentId)?.kind === "agent") agentLookup.set(agentId, pos);
  }

  for (const node of nodes) {
    if (node.kind !== "role") continue;
    const boundAgent = edges.find((e) => e.source === node.id && e.kind === "binds")?.target;
    const agentPos = boundAgent ? agentLookup.get(boundAgent) : undefined;
    if (agentPos) {
      result.set(node.id, {
        x: agentPos.x + jitter(node.id) * 100,
        y: ROLE_BAND_Y + jitter(node.id, 3) * 20,
      });
    } else {
      const angle = Math.PI * (0.08 + 0.84 * jitter(node.id)) + 0.8;
      result.set(node.id, {
        x: Math.cos(angle) * (AGENT_RING_RADIUS + 60),
        y: ROLE_BAND_Y + Math.sin(angle) * 40,
      });
    }
  }

  const runs = nodes.filter((n) => n.kind === "run").sort((a, b) => runIdOf(a) - runIdOf(b));
  const runLookup = new Map<string, { x: number; y: number }>();
  runs.forEach((node, index) => {
    const x = (index - (runs.length - 1) / 2) * RUN_BAND_GAP + jitter(node.id) * 46;
    const y = RUN_BAND_Y + jitter(node.id, 3) * 16;
    result.set(node.id, { x, y });
    runLookup.set(node.id, { x, y });
  });

  for (const node of nodes) {
    if (node.kind !== "task") continue;
    const runHome = runLookup.get(`run:${taskMeta(node).runId}`);
    if (!runHome) {
      result.set(node.id, { x: 0, y: 120 });
      continue;
    }
    const siblings = nodes
      .filter((n) => n.kind === "task" && taskMeta(n).runId === taskMeta(node).runId)
      .sort((a, b) => taskMeta(a).position - taskMeta(b).position);
    const index = siblings.findIndex((s) => s.id === node.id);
    const t = siblings.length <= 1 ? 0.5 : (index + 1) / (siblings.length + 1);
    const angle = Math.PI * (0.18 + 0.64 * t);
    result.set(node.id, {
      x: runHome.x + Math.cos(angle) * TASK_FAN_RADIUS + jitter(node.id) * 20,
      y: runHome.y + Math.sin(angle) * TASK_FAN_RADIUS + jitter(node.id, 5) * 20,
    });
  }

  // Compact external GitHub nodes hug their run: the imported Issue sits
  // above the workflow it seeded, the delivered PR beside it.
  for (const node of nodes) {
    if (node.kind !== "github_issue" && node.kind !== "github_pr") continue;
    const runHome = runLookup.get(`run:${runIdOf(node)}`);
    if (!runHome) {
      result.set(node.id, { x: jitter(node.id) * 90, y: 60 + jitter(node.id, 4) * 20 });
      continue;
    }
    if (node.kind === "github_issue") {
      result.set(node.id, {
        x: runHome.x - 12 + jitter(node.id) * 24,
        y: runHome.y - 128 + jitter(node.id, 2) * 12,
      });
    } else {
      result.set(node.id, {
        x: runHome.x + 158 + jitter(node.id) * 20,
        y: runHome.y - 128 + jitter(node.id, 2) * 12,
      });
    }
  }

  const groups = nodes.filter((node) => node.kind === "group");
  groups.forEach((node, index) => {
    result.set(node.id, {
      x: (index % 2 === 0 ? -1 : 1) * (310 + Math.floor(index / 2) * 150),
      y: -20 + jitter(node.id, 8) * 18,
    });
  });
  const notes = nodes.filter((node) => node.kind === "note");
  notes.forEach((node, index) => {
    result.set(node.id, {
      x: (index % 2 === 0 ? -1 : 1) * (245 + Math.floor(index / 2) * 110),
      y: -235 + jitter(node.id, 5) * 22,
    });
  });

  return result;
}

function edgeEndpoints(
  source: { x: number; y: number; width: number; height: number },
  target: { x: number; y: number; width: number; height: number }
): [number, number, number, number] {
  const pointOn = (
    node: { x: number; y: number; width: number; height: number },
    dx: number,
    dy: number
  ): [number, number] => {
    if (dx === 0 && dy === 0) return [node.x + node.width / 2, node.y];
    const rx = node.width / 2;
    const ry = node.height / 2;
    const scale = 1 / Math.sqrt((dx / rx) ** 2 + (dy / ry) ** 2);
    return [node.x + node.width / 2 + dx * scale, node.y + node.height / 2 + dy * scale];
  };
  const [x1, y1] = pointOn(
    source,
    target.x + target.width / 2 - (source.x + source.width / 2),
    target.y + target.height / 2 - (source.y + source.height / 2)
  );
  const [x2, y2] = pointOn(
    target,
    source.x + source.width / 2 - (target.x + target.width / 2),
    source.y + source.height / 2 - (target.y + target.height / 2)
  );
  return [x1, y1, x2, y2];
}

function bendSign(sourceId: string, targetId: string): number {
  let hash = 0;
  const key = `${sourceId}>${targetId}`;
  for (let i = 0; i < key.length; i += 1) {
    hash = (hash * 31 + key.charCodeAt(i)) >>> 0;
  }
  return hash % 2 === 0 ? 1 : -1;
}

export function computeNetworkLayout(nodes: GraphNode[], edges: GraphEdge[]): NetworkLayout {
  if (nodes.length === 0) {
    return { nodes: [], edges: [], width: MIN_CANVAS_WIDTH, height: MIN_CANVAS_HEIGHT };
  }

  const house = homes(nodes, edges);
  const bodies: Body[] = nodes.map((node) => {
    const size = nodeSize(node);
    const home = house.get(node.id) ?? { x: 0, y: 0 };
    return {
      id: node.id,
      kind: node.kind,
      x: home.x + jitter(node.id, 4) * 16,
      y: home.y + jitter(node.id, 7) * 16,
      width: size.width,
      height: size.height,
      houseX: home.x,
      houseY: home.y,
    };
  });
  const indexById = new Map(bodies.map((body, index) => [body.id, index]));

  const resolveOverlap = (a: Body, b: Body): boolean => {
    const dx = b.x + b.width / 2 - (a.x + a.width / 2);
    const dy = b.y + b.height / 2 - (a.y + a.height / 2);
    const overlapX = (a.width + b.width) / 2 - Math.abs(dx);
    const overlapY = (a.height + b.height) / 2 - Math.abs(dy);
    if (overlapX <= 0 || overlapY <= 0) return false;
    if (overlapX < overlapY) {
      const push = (overlapX + 0.5) / 2;
      const sign = dx === 0 ? 1 : Math.sign(dx);
      a.x -= sign * push;
      b.x += sign * push;
    } else {
      const push = (overlapY + 0.5) / 2;
      const sign = dy === 0 ? 1 : Math.sign(dy);
      a.y -= sign * push;
      b.y += sign * push;
    }
    return true;
  };

  const moveX = new Array<number>(bodies.length).fill(0);
  const moveY = new Array<number>(bodies.length).fill(0);

  for (let iter = 0; iter < ITERATIONS; iter += 1) {
    const progress = iter / ITERATIONS;
    const limit = 6 + 20 * (1 - progress);
    moveX.fill(0);
    moveY.fill(0);

    for (let i = 0; i < bodies.length; i += 1) {
      for (let j = i + 1; j < bodies.length; j += 1) {
        const a = bodies[i];
        const b = bodies[j];
        const dx = b.x + b.width / 2 - (a.x + a.width / 2);
        const dy = b.y + b.height / 2 - (a.y + a.height / 2);
        const dist = Math.hypot(dx, dy) || 1;
        const force = REPULSION / (dist * dist + dist * 6 + 60);
        const fx = (-dx / dist) * force;
        const fy = (-dy / dist) * force;
        moveX[i] += fx;
        moveY[i] += fy;
        moveX[j] -= fx;
        moveY[j] -= fy;
      }
    }

    for (const edge of edges) {
      const ai = indexById.get(edge.source);
      const bi = indexById.get(edge.target);
      if (ai === undefined || bi === undefined) continue;
      const dx = bodies[bi].x - bodies[ai].x;
      const dy = bodies[bi].y - bodies[ai].y;
      const dist = Math.hypot(dx, dy) || 1;
      const stretch = (dist - EDGE_REST[edge.kind]) * 0.02;
      const fx = (dx / dist) * stretch;
      const fy = (dy / dist) * stretch;
      moveX[ai] += fx;
      moveY[ai] += fy;
      moveX[bi] -= fx;
      moveY[bi] -= fy;
    }

    for (let i = 0; i < bodies.length; i += 1) {
      bodies[i].x += Math.max(-limit, Math.min(limit, moveX[i]));
      bodies[i].y += Math.max(-limit, Math.min(limit, moveY[i]));
      bodies[i].x += (bodies[i].houseX - bodies[i].x) * ANCHOR[bodies[i].kind];
      bodies[i].y += (bodies[i].houseY - bodies[i].y) * ANCHOR[bodies[i].kind];
      bodies[i].x += -bodies[i].x * GRAVITY;
      bodies[i].y += -bodies[i].y * GRAVITY;
    }
  }

  let hadOverlap = true;
  for (let pass = 0; hadOverlap && pass < 64; pass += 1) {
    hadOverlap = false;
    for (let i = 0; i < bodies.length; i += 1) {
      for (let j = i + 1; j < bodies.length; j += 1) {
        if (resolveOverlap(bodies[i], bodies[j])) hadOverlap = true;
      }
    }
  }

  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const body of bodies) {
    minX = Math.min(minX, body.x);
    minY = Math.min(minY, body.y);
    maxX = Math.max(maxX, body.x + body.width);
    maxY = Math.max(maxY, body.y + body.height);
  }
  const contentWidth = maxX - minX + BOUNDS_PADDING * 2;
  const contentHeight = maxY - minY + BOUNDS_PADDING * 2;
  const width = Math.max(contentWidth, MIN_CANVAS_WIDTH);
  const height = Math.max(contentHeight, MIN_CANVAS_HEIGHT);
  const offsetX = BOUNDS_PADDING + (width - contentWidth) / 2 - minX;
  const offsetY = BOUNDS_PADDING + (height - contentHeight) / 2 - minY;

  const positioned: NetworkNodePos[] = bodies.map((body) => ({
    id: body.id,
    kind: body.kind,
    x: body.x + offsetX,
    y: body.y + offsetY,
    cx: body.x + body.width / 2 + offsetX,
    cy: body.y + body.height / 2 + offsetY,
    width: body.width,
    height: body.height,
  }));

  const positions = new Map(positioned.map((pos) => [pos.id, pos]));
  const edgePositions: NetworkEdgePos[] = [];
  for (const edge of edges) {
    const source = positions.get(edge.source);
    const target = positions.get(edge.target);
    if (!source || !target) continue;
    const [x1, y1, x2, y2] = edgeEndpoints(source, target);
    const mx = (x1 + x2) / 2;
    const my = (y1 + y2) / 2;
    const len = Math.hypot(x2 - x1, y2 - y1) || 1;
    const offset = Math.min(72, len * 0.24);
    const sign = bendSign(edge.source, edge.target);
    const ctrlX = mx + (-(y2 - y1) / len) * offset * sign;
    const ctrlY = my + ((x2 - x1) / len) * offset * sign;
    edgePositions.push({
      source: edge.source,
      target: edge.target,
      kind: edge.kind,
      path: `M ${x1} ${y1} Q ${ctrlX} ${ctrlY} ${x2} ${y2}`,
    });
  }

  return { nodes: positioned, edges: edgePositions, width, height };
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
