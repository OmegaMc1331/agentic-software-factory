import { describe, expect, it } from "vitest";
import {
  findFreePosition,
  mergeGraphWorkspace,
  validateConnection,
  workspacePosition,
} from "./graphWorkspace";
import type { GraphData, GraphWorkspace } from "./types";

const workspace: GraphWorkspace = {
  version: 1,
  nodes: { "agent:codex": { x: 418, y: 216 } },
  customNodes: [
    { id: "group:backend", kind: "group", label: "Backend" },
    { id: "note:review", kind: "note", label: "Review", text: "Tests first" },
  ],
  edges: [
    {
      id: "edge:custom:one",
      source: "agent:codex",
      target: "agent:opencode",
      kind: "custom",
    },
  ],
};

const graph: GraphData = {
  nodes: [
    {
      id: "agent:codex",
      kind: "agent",
      label: "Codex",
      meta: { command: "codex exec", available: true, roles: ["planner"] },
    },
    {
      id: "agent:opencode",
      kind: "agent",
      label: "OpenCode",
      meta: { command: "opencode run", available: true, roles: ["worker"] },
    },
    {
      id: "role:planner",
      kind: "role",
      label: "planner",
      meta: {
        id: "planner",
        name: "Planner",
        kind: "core",
        description: "Transforms the objective into tasks.",
        instructions: "",
        executionClass: "planning",
        assignments: [{ agent: "codex", preferred: true }],
        available: true,
      },
    },
  ],
  edges: [
    {
      id: "assignment:planner",
      source: "role:planner",
      target: "agent:codex",
      kind: "binds",
      editable: true,
      semantic: "configuration",
    },
  ],
  metadata: { runs: 0, tasks: 0, agents: 2, missingAgents: 0, roles: 1 },
};

describe("graph workspace state", () => {
  it("uses persisted manual positions over automatic positions", () => {
    expect(workspacePosition("agent:codex", { x: 10, y: 20 }, workspace)).toEqual({
      x: 418,
      y: 216,
    });
    expect(workspacePosition("agent:opencode", { x: 10, y: 20 }, workspace)).toEqual({
      x: 10,
      y: 20,
    });
  });

  it("returns to automatic positions after the manual layout is reset", () => {
    const reset = { ...workspace, nodes: {} };
    expect(workspacePosition("agent:codex", { x: 10, y: 20 }, reset)).toEqual({
      x: 10,
      y: 20,
    });
  });

  it("merges only visual nodes and custom edges into Factory graph state", () => {
    const merged = mergeGraphWorkspace(graph, workspace);
    expect(merged.nodes.map((node) => node.id)).toContain("group:backend");
    expect(merged.nodes.map((node) => node.id)).toContain("note:review");
    expect(merged.edges.at(-1)).toMatchObject({
      kind: "custom",
      editable: true,
      semantic: "visual",
    });
  });

  it("accepts agent links and rejects duplicates and invalid endpoints", () => {
    const merged = mergeGraphWorkspace(graph, workspace);
    const nodes = new Map(merged.nodes.map((node) => [node.id, node]));
    expect(
      validateConnection(
        { source: "agent:opencode", target: "agent:codex", sourceHandle: null, targetHandle: null },
        nodes,
        merged.edges
      )
    ).toBeNull();
    expect(
      validateConnection(
        { source: "agent:codex", target: "agent:opencode", sourceHandle: null, targetHandle: null },
        nodes,
        merged.edges
      )
    ).toBe("This connection already exists.");
    expect(
      validateConnection(
        { source: "role:planner", target: "group:backend", sourceHandle: null, targetHandle: null },
        nodes,
        merged.edges
      )
    ).toBeNull();
    expect(
      validateConnection(
        { source: "role:planner", target: "note:review", sourceHandle: null, targetHandle: null },
        nodes,
        merged.edges
      )
    ).toBe("These node types do not support a configurable connection.");
  });

  it("places new nodes away from occupied bounds", () => {
    const result = findFreePosition({ x: 100, y: 100 }, [{ x: 20, y: 50, width: 160, height: 90 }]);
    expect(result).not.toEqual({ x: 30, y: 65 });
  });
});
