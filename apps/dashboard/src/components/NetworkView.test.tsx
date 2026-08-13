import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  fetchAgentSessions,
  fetchConfig,
  fetchGraph,
  fetchGraphWorkspace,
  saveGraphWorkspace,
} from "../api";
import type { GraphData, GraphWorkspace } from "../types";
import { NetworkView } from "./NetworkView";

vi.mock("../api", () => ({
  agentSessionStreamUrl: (id: number) => `/api/sessions/${id}/stream`,
  fetchAgentSessions: vi.fn(),
  fetchConfig: vi.fn(),
  fetchGraph: vi.fn(),
  fetchGraphWorkspace: vi.fn(),
  saveConfig: vi.fn(),
  saveGraphWorkspace: vi.fn(),
}));

vi.mock("./AgentGraph", async () => {
  const React = await import("react");
  type MockGraphProps = {
    nodesById: Map<string, { id: string }>;
    edges: Array<{ id: string; kind: string }>;
    onNodeSelect: (id: string) => void;
    onEdgeSelect: (id: string) => void;
    onNodeDragStop: (id: string, position: { x: number; y: number }) => void;
  };
  const AgentGraph = React.forwardRef<unknown, MockGraphProps>((props, ref) => {
    React.useImperativeHandle(ref, () => ({
      fit: vi.fn(),
      center: vi.fn(),
      centerOn: vi.fn(),
      zoomIn: vi.fn(),
      zoomOut: vi.fn(),
      viewportCenter: () => ({ x: 300, y: 220 }),
    }));
    return (
      <div data-testid="agent-graph">
        {[...props.nodesById.values()].map((node) => (
          <button key={node.id} onClick={() => props.onNodeSelect(node.id)}>
            select {node.id}
          </button>
        ))}
        <button onClick={() => props.onNodeDragStop("agent:codex", { x: 515, y: 315 })}>
          drag codex
        </button>
        {props.edges
          .filter((edge) => edge.kind === "custom")
          .map((edge) => (
            <button key={edge.id} onClick={() => props.onEdgeSelect(edge.id)}>
              select {edge.id}
            </button>
          ))}
      </div>
    );
  });
  AgentGraph.displayName = "MockAgentGraph";
  return { AgentGraph };
});

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
  ],
  edges: [],
  metadata: { runs: 0, tasks: 0, agents: 2, missingAgents: 0, roles: 0 },
};

const workspace: GraphWorkspace = {
  version: 1,
  nodes: {
    "agent:codex": { x: 418, y: 216 },
    "agent:opencode": { x: 620, y: 260 },
  },
  customNodes: [],
  edges: [
    {
      id: "edge:custom:one",
      source: "agent:codex",
      target: "agent:opencode",
      kind: "custom",
    },
  ],
};

beforeEach(() => {
  vi.mocked(fetchGraph).mockReset().mockResolvedValue(graph);
  vi.mocked(fetchGraphWorkspace).mockReset().mockResolvedValue(workspace);
  vi.mocked(fetchConfig)
    .mockReset()
    .mockResolvedValue({
      agents: {
        codex: { command: "codex", args: ["exec"], env: {} },
        opencode: { command: "opencode", args: ["run"], env: {} },
      },
      roles: {},
    });
  vi.mocked(fetchAgentSessions).mockReset().mockResolvedValue([]);
  vi.mocked(saveGraphWorkspace).mockReset().mockResolvedValue();
});

afterEach(cleanup);

describe("Agent Graph interactions", () => {
  it("opens the Agent Console when an agent is selected", async () => {
    render(<NetworkView />);
    fireEvent.click(await screen.findByRole("button", { name: "select agent:codex" }));

    expect(await screen.findByLabelText("codex Agent Console")).toBeTruthy();
    expect(screen.getByText("No active session.")).toBeTruthy();
  });

  it("opens the compact node creation menu from the toolbar", async () => {
    render(<NetworkView />);
    fireEvent.click(await screen.findByRole("button", { name: "Add graph node" }));

    expect(screen.getByRole("dialog", { name: "Add graph node" })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Agent/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Role/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Group/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Note/ })).toBeTruthy();
  });

  it("persists a node position after drag end", async () => {
    render(<NetworkView />);
    fireEvent.click(await screen.findByRole("button", { name: "drag codex" }));

    await waitFor(() =>
      expect(saveGraphWorkspace).toHaveBeenCalledWith(
        expect.objectContaining({
          nodes: expect.objectContaining({
            "agent:codex": { x: 515, y: 315 },
          }),
        })
      )
    );
  });

  it("deletes and persists a selected custom edge", async () => {
    render(<NetworkView />);
    fireEvent.click(await screen.findByRole("button", { name: "select edge:custom:one" }));
    fireEvent.keyDown(window, { key: "Delete" });

    await waitFor(() =>
      expect(saveGraphWorkspace).toHaveBeenCalledWith(expect.objectContaining({ edges: [] }))
    );
  });
});
