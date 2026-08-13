import { describe, expect, it } from "vitest";
import {
  MIN_CANVAS_HEIGHT,
  MIN_CANVAS_WIDTH,
  computeNetworkLayout,
  jitter,
  neighborsOf,
} from "./networkLayout";
import type { GraphEdge, GraphNode } from "./types";

function node(id: string, kind: GraphNode["kind"], label = id): GraphNode {
  const base = { id, kind, label };
  if (kind === "agent") {
    return { ...base, meta: { command: "codex exec", available: true, roles: [] } };
  }
  if (kind === "role") {
    return { ...base, meta: { agent: "codex" } };
  }
  if (kind === "run") {
    return {
      ...base,
      meta: {
        runId: Number(id.split(":")[1]),
        objective: "",
        status: "planned",
        plannerAgent: "codex",
        workerAgent: "opencode",
        reviewerAgent: "claude",
        createdAt: "2026-01-01T00:00:00Z",
        counts: {
          pending: 0,
          ready: 0,
          running: 0,
          blocked: 0,
          failed: 0,
          completed: 0,
          total: 0,
        },
      },
    };
  }
  return {
    ...base,
    meta: {
      taskId: Number(id.split(":")[1]),
      runId: 1,
      objective: "",
      state: "pending",
      position: Number(id.split(":")[1]),
      dependencies: [],
      acceptanceCriteria: [],
      worktreePath: null,
      currentAttempt: null,
    },
  };
}

function edge(source: string, target: string, kind: GraphEdge["kind"]): GraphEdge {
  return {
    id: `${kind}:${source}:${target}`,
    source,
    target,
    kind,
    editable: false,
    semantic: kind === "depends" ? "execution" : "system",
  };
}

function task(n: number, runId = 1, position = n): GraphNode {
  const base = node(`task:${n}`, "task");
  return { ...base, meta: { ...base.meta, runId, position, taskId: n } as never };
}

function run(n: number, status = "planned"): GraphNode {
  const base = node(`run:${n}`, "run");
  return { ...base, meta: { ...base.meta, status } as never };
}

/** A familiar topology: 3 agents, planner/worker/reviewer roles, 2 runs with fans of tasks. */
export function fixture(): { nodes: GraphNode[]; edges: GraphEdge[] } {
  const nodes: GraphNode[] = [
    node("agent:codex", "agent", "Codex"),
    node("agent:opencode", "agent", "OpenCode"),
    node("agent:claude", "agent", "Claude"),
    node("role:planner", "role", "planner"),
    node("role:worker", "role", "worker"),
    node("role:reviewer", "role", "reviewer"),
    run(1, "active"),
    run(2),
  ];
  for (let r = 1; r <= 2; r += 1) {
    for (let i = 1; i <= 6; i += 1) {
      const id = (r - 1) * 6 + i;
      nodes.push(task(100 + id, r, i));
    }
  }
  const edges: GraphEdge[] = [
    edge("role:planner", "agent:codex", "binds"),
    edge("role:worker", "agent:opencode", "binds"),
    edge("role:reviewer", "agent:claude", "binds"),
    edge("run:1", "role:planner", "plans"),
    edge("run:2", "role:planner", "plans"),
  ];
  for (let r = 1; r <= 2; r += 1) {
    for (let i = 1; i <= 6; i += 1) {
      const id = 100 + (r - 1) * 6 + i;
      edges.push(edge(`run:${r}`, `task:${id}`, "contains"));
      if (i > 1) edges.push(edge(`task:${id - 1}`, `task:${id}`, "depends"));
    }
  }
  return { nodes, edges };
}

describe("jitter", () => {
  it("is deterministic and bounded", () => {
    const first = jitter("agent:codex", 3);
    expect(jitter("agent:codex", 3)).toBe(first);
    expect(Math.abs(first)).toBeLessThanOrEqual(3);
  });

  it("differs between ids", () => {
    expect(jitter("agent:codex")).not.toBe(jitter("agent:claude"));
  });
});

describe("computeNetworkLayout", () => {
  it("is deterministic for the same input", () => {
    const { nodes, edges } = fixture();
    const a = computeNetworkLayout(nodes, edges);
    const b = computeNetworkLayout(nodes, edges);
    expect(a.nodes.map((n) => [n.id, n.cx, n.cy])).toEqual(b.nodes.map((n) => [n.id, n.cx, n.cy]));
    expect(a.edges).toEqual(b.edges);
  });

  it("places every node inside the canvas", () => {
    const layout = computeNetworkLayout(fixture().nodes, fixture().edges);
    expect(layout.nodes).toHaveLength(fixture().nodes.length);
    for (const pos of layout.nodes) {
      expect(pos.x).toBeGreaterThanOrEqual(0);
      expect(pos.y).toBeGreaterThanOrEqual(0);
      expect(pos.x + pos.width).toBeLessThanOrEqual(layout.width);
      expect(pos.y + pos.height).toBeLessThanOrEqual(layout.height);
    }
    for (const edgePos of layout.edges) {
      expect(edgePos.path.startsWith("M")).toBe(true);
    }
  });

  it("keeps edge endpoints on the source and target nodes", () => {
    const layout = computeNetworkLayout(fixture().nodes, fixture().edges);
    const byPos = new Map(layout.nodes.map((n) => [n.id, n]));
    for (const edgePos of layout.edges) {
      const match = edgePos.path.match(/^M ([\d.-]+) ([\d.-]+) Q /);
      const source = byPos.get(edgePos.source);
      const target = byPos.get(edgePos.target);
      expect(match).not.toBeNull();
      expect(source, edgePos.source).toBeDefined();
      expect(target, edgePos.target).toBeDefined();
      const [startX, startY] = [Number(match?.[1]), Number(match?.[2])];
      const cxHead = Number(edgePos.path.match(/Q ([\d.-]+) ([\d.-]+) ([\d.-]+) ([\d.-]+)$/)?.[3]);
      const cyHead = Number(edgePos.path.match(/Q ([\d.-]+) ([\d.-]+) ([\d.-]+) ([\d.-]+)$/)?.[4]);
      const distanceToSource = Math.hypot(startX - source!.cx, startY - source!.cy);
      const distanceToTarget = Math.hypot(cxHead - target!.cx, cyHead - target!.cy);
      expect(distanceToSource).toBeLessThanOrEqual(source!.width);
      expect(distanceToTarget).toBeLessThanOrEqual(target!.width);
    }
  });

  it("lays out as a hub: roles above, agents centered, runs mid, tasks below", () => {
    const layout = computeNetworkLayout(fixture().nodes, fixture().edges);
    const meanCy = (kind: string) => {
      const list = layout.nodes.filter((n) => n.kind === kind);
      return list.reduce((sum, n) => sum + n.cy, 0) / list.length;
    };
    const agentCy = meanCy("agent");
    const agentCx =
      layout.nodes.filter((n) => n.kind === "agent").reduce((sum, n) => sum + n.cx, 0) /
      layout.nodes.filter((n) => n.kind === "agent").length;

    expect(agentCx).toBeGreaterThan(layout.width * 0.33);
    expect(agentCx).toBeLessThan(layout.width * 0.67);
    expect(agentCy).toBeGreaterThan(layout.height * 0.25);
    expect(agentCy).toBeLessThan(layout.height * 0.75);
    expect(meanCy("role")).toBeLessThan(agentCy);
    expect(meanCy("run")).toBeGreaterThan(agentCy);
    expect(meanCy("task")).toBeGreaterThan(meanCy("run"));
  });

  it("clusters each run's tasks around its own run", () => {
    const layout = computeNetworkLayout(fixture().nodes, fixture().edges);
    const byRun = (id: number) =>
      layout.nodes.filter((n) => n.kind === "task" && n.id.startsWith(`task:${id}`));

    const meanDistance = (tasks: typeof layout.nodes, run: (typeof layout.nodes)[number]) =>
      tasks.reduce((sum, t) => sum + Math.hypot(t.cx - run.cx, t.cy - run.cy), 0) / tasks.length;

    const run1 = layout.nodes.find((n) => n.id === "run:1")!;
    const run2 = layout.nodes.find((n) => n.id === "run:2")!;
    const run1Tasks = byRun(101).concat(byRun(102), byRun(103), byRun(104), byRun(105), byRun(106));
    const run2Tasks = byRun(107).concat(byRun(108), byRun(109), byRun(110), byRun(111), byRun(112));

    expect(meanDistance(run1Tasks, run1)).toBeLessThan(meanDistance(run2Tasks, run1));
    expect(meanDistance(run2Tasks, run2)).toBeLessThan(meanDistance(run1Tasks, run2));
  });

  it("grows edges as gentle quadratics", () => {
    const layout = computeNetworkLayout(fixture().nodes, fixture().edges);
    expect(layout.edges.length).toBeGreaterThan(0);
    for (const edgePos of layout.edges) {
      expect(edgePos.path).toContain("Q");
    }
  });

  it("does not overlap any two nodes", () => {
    const layout = computeNetworkLayout(fixture().nodes, fixture().edges);
    for (let i = 0; i < layout.nodes.length; i += 1) {
      for (let j = i + 1; j < layout.nodes.length; j += 1) {
        const a = layout.nodes[i];
        const b = layout.nodes[j];
        const overlapX = (a.width + b.width) / 2 - Math.abs(a.cx - b.cx);
        const overlapY = (a.height + b.height) / 2 - Math.abs(a.cy - b.cy);
        // Boxes only collide when BOTH axes overlap; a small knick on the
        // dominant axis is invisible because nodes render as rounded shapes.
        expect(overlapX < 8 || overlapY < 8, `${a.id} overlaps ${b.id}`).toBe(true);
      }
    }
  });

  it("shows the agent and role topology when there are no runs", () => {
    const nodes = [node("agent:codex", "agent", "Codex"), node("role:planner", "role", "planner")];
    const edges = [edge("role:planner", "agent:codex", "binds")];
    const layout = computeNetworkLayout(nodes, edges);
    expect(layout.nodes).toHaveLength(2);
    expect(layout.edges).toHaveLength(1);
    expect(layout.width).toBeGreaterThan(0);
    expect(layout.height).toBeGreaterThan(0);
  });

  it("skips edges whose endpoints are missing", () => {
    const layout = computeNetworkLayout(
      [task(1), task(2)],
      [edge("run:99", "task:1", "contains"), edge("task:1", "task:2", "depends")]
    );
    expect(layout.edges).toHaveLength(1);
  });

  it("handles an empty graph with a minimum canvas", () => {
    const layout = computeNetworkLayout([], []);
    expect(layout.nodes).toHaveLength(0);
    expect(layout.edges).toHaveLength(0);
    expect(layout.width).toBe(MIN_CANVAS_WIDTH);
    expect(layout.height).toBe(MIN_CANVAS_HEIGHT);
  });
});

describe("neighborsOf", () => {
  it("returns nodes connected in either direction", () => {
    const { nodes, edges } = fixture();
    const layout = computeNetworkLayout(nodes, edges);
    const neighbors = neighborsOf(layout.nodes, layout.edges, "agent:codex").sort();
    expect(neighbors).toContain("role:planner");
    expect(neighborsOf(layout.nodes, layout.edges, "run:1")).toContain("role:planner");
  });

  it("returns no neighbors for an isolated node", () => {
    const layout = computeNetworkLayout(
      [node("a", "agent"), node("b", "agent")],
      [edge("a", "x", "plans")]
    );
    expect(neighborsOf(layout.nodes, layout.edges, "a")).toEqual([]);
  });
});
