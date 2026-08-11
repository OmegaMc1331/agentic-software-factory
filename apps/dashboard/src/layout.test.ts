import { describe, expect, it } from "vitest";
import { NODE_WIDTH, LEVEL_GAP, computeLayout, truncate } from "./layout";
import type { Task } from "./types";

function task(id: number, dependencies: number[]): Task {
  return {
    id,
    runId: 1,
    title: `Task ${id}`,
    objective: "",
    acceptanceCriteria: [],
    state: "pending",
    position: id,
    dependencies,
    worktreePath: null,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
  };
}

describe("computeLayout", () => {
  it("layers a dependency chain left to right", () => {
    const tasks = [task(1, []), task(2, [1]), task(3, [2])];
    const layout = computeLayout(tasks);
    const byId = new Map(layout.nodes.map((n) => [n.id, n]));

    expect(byId.get(1)?.level).toBe(0);
    expect(byId.get(2)?.level).toBe(1);
    expect(byId.get(3)?.level).toBe(2);
    expect(byId.get(2)?.x).toBe(NODE_WIDTH + LEVEL_GAP);
    expect(layout.edges).toHaveLength(2);
  });

  it("places independent tasks on the same level", () => {
    const tasks = [task(1, []), task(2, []), task(3, [1, 2])];
    const layout = computeLayout(tasks);
    const byId = new Map(layout.nodes.map((n) => [n.id, n]));

    expect(byId.get(1)?.level).toBe(0);
    expect(byId.get(2)?.level).toBe(0);
    expect(byId.get(3)?.level).toBe(1);
    expect(byId.get(1)?.y).not.toBe(byId.get(2)?.y);
  });

  it("keeps diamond dependencies acyclic and level-consistent", () => {
    const tasks = [task(1, []), task(2, [1]), task(3, [1]), task(4, [2, 3])];
    const layout = computeLayout(tasks);
    const byId = new Map(layout.nodes.map((n) => [n.id, n]));

    expect(byId.get(4)?.level).toBe(2);
    expect(layout.edges).toHaveLength(4);
    expect(layout.width).toBeGreaterThan(NODE_WIDTH);
  });

  it("handles an empty task list", () => {
    const layout = computeLayout([]);
    expect(layout.nodes).toHaveLength(0);
    expect(layout.edges).toHaveLength(0);
    expect(layout.width).toBeGreaterThan(0);
    expect(layout.height).toBeGreaterThan(0);
  });
});

describe("truncate", () => {
  it("keeps short strings unchanged", () => {
    expect(truncate("hello")).toBe("hello");
  });

  it("ellipsizes long strings", () => {
    const value = "x".repeat(40);
    const result = truncate(value);
    expect(result.endsWith("…")).toBe(true);
    expect(result.length).toBe(26);
  });
});
