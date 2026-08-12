import { describe, expect, it } from "vitest";
import {
  LANE_ORDER,
  NET_LANE_GAP,
  NET_MARGIN,
  NET_NODE_HEIGHT,
  NODE_WIDTH,
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
        objective: "",
        status: "planned",
        plannerAgent: "codex",
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
      worktreePath: null,
    },
  };
}

function edge(source: string, target: string, kind: GraphEdge["kind"]): GraphEdge {
  return { source, target, kind };
}

const task = (n: number, runId = 1, position = n) => ({
  ...node(`task:${n}`, "task"),
  meta: { ...node(`task:${n}`, "task").meta, runId, position, taskId: n },
});

describe("computeNetworkLayout", () => {
  it("places lanes left to right in agent, role, run, task order", () => {
    const layout = computeNetworkLayout(
      [task(1), node("run:2", "run"), node("role:planner", "role"), node("agent:a", "agent")],
      []
    );
    const axis = new Map(layout.lanes.map((lane) => [lane.kind, lane.x]));
    const kinds = layout.lanes.map((lane) => lane.kind);
    expect(kinds).toEqual(LANE_ORDER);
    expect(axis.get("agent") ?? 0).toBeLessThan(axis.get("role") ?? 0);
    expect(axis.get("role") ?? 0).toBeLessThan(axis.get("run") ?? 0);
    expect(axis.get("run") ?? 0).toBeLessThan(axis.get("task") ?? 0);
  });

  it("orders runs by id and tasks by run then position", () => {
    const layout = computeNetworkLayout(
      [task(9, 2, 1), task(5, 1, 2), task(3, 1, 1), node("run:2", "run"), node("run:1", "run")],
      []
    );
    const runs = layout.nodes.filter((n) => n.kind === "run").map((n) => n.id);
    const tasks = layout.nodes.filter((n) => n.kind === "task").map((n) => n.id);
    expect(runs).toEqual(["run:1", "run:2"]);
    expect(tasks).toEqual(["task:3", "task:5", "task:9"]);
  });

  it("keeps lanes vertically centered", () => {
    const layout = computeNetworkLayout([node("run:1", "run"), task(1), task(2)], []);
    const run = layout.nodes.find((n) => n.id === "run:1");
    const runLaneHeight = NET_NODE_HEIGHT;
    const expectedTop = Math.max(0, (layout.height - runLaneHeight) / 2);
    expect(run?.y).toBeCloseTo(expectedTop + jitter("run:1", 2), 0);
  });

  it("builds curved paths for cross-lane edges and bows for dependencies", () => {
    const layout = computeNetworkLayout(
      [node("role:planner", "role"), node("run:1", "run"), task(1), task(2)],
      [
        edge("run:1", "role:planner", "uses"),
        edge("run:1", "task:1", "contains"),
        edge("task:1", "task:2", "depends"),
      ]
    );
    const usesPath = layout.edges.find((e) => e.kind === "uses")?.path ?? "";
    const dependsPath = layout.edges.find((e) => e.kind === "depends")?.path ?? "";
    expect(usesPath).toContain("C");
    expect(dependsPath).toContain("Q");
  });

  it("is deterministic for the same input", () => {
    const nodes = [node("run:1", "run"), task(1), task(2), node("role:planner", "role")];
    const edges = [edge("run:1", "task:1", "contains")];
    const a = computeNetworkLayout(nodes, edges);
    const b = computeNetworkLayout(nodes, edges);
    expect(a.nodes.map((n) => [n.id, n.cx, n.cy])).toEqual(b.nodes.map((n) => [n.id, n.cx, n.cy]));
    expect(a.edges).toEqual(b.edges);
  });

  it("skips edges whose endpoints are missing and sizes the canvas", () => {
    const layout = computeNetworkLayout([task(1)], [edge("run:99", "task:1", "contains")]);
    expect(layout.edges).toHaveLength(0);
    expect(layout.height).toBeGreaterThan(0);
    const expectedWidth =
      NET_MARGIN * 2 +
      LANE_ORDER.reduce((sum, kind) => sum + NODE_WIDTH[kind], 0) +
      NET_LANE_GAP * (LANE_ORDER.length - 1);
    expect(layout.width).toBe(expectedWidth);
  });

  it("handles an empty graph", () => {
    const layout = computeNetworkLayout([], []);
    expect(layout.nodes).toHaveLength(0);
    expect(layout.edges).toHaveLength(0);
    expect(layout.width).toBeGreaterThan(0);
    expect(layout.height).toBeGreaterThan(0);
  });
});

describe("neighborsOf", () => {
  it("returns nodes connected in either direction", () => {
    const nodes = [node("a", "agent"), node("b", "agent"), node("c", "agent")];
    const layout = computeNetworkLayout(nodes, [edge("a", "b", "uses"), edge("c", "b", "uses")]);
    const neighbors = neighborsOf(layout.nodes, layout.edges, "b").sort();
    expect(neighbors).toEqual(["a", "c"]);
    expect(neighborsOf(layout.nodes, layout.edges, "a")).toEqual(["b"]);
  });

  it("returns no neighbors for an isolated node", () => {
    const layout = computeNetworkLayout([node("a", "agent"), node("b", "agent")], []);
    expect(neighborsOf(layout.nodes, layout.edges, "a")).toEqual([]);
  });
});
