import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  addRoleAssignment,
  createWorkflow,
  deleteRole,
  fetchAgentSessions,
  fetchConfig,
  fetchGraph,
  fetchGraphWorkspace,
  fetchRoles,
  fetchRecipes,
  removeRoleAssignment,
  saveConfig,
  saveGraphWorkspace,
} from "../api";
import type { GraphData, GraphWorkspace, RoleInfo } from "../types";
import { NetworkView } from "./NetworkView";

vi.mock("../api", () => ({
  addRoleAssignment: vi.fn(),
  agentSessionStreamUrl: (id: number) => `/api/sessions/${id}/stream`,
  cancelWorkflow: vi.fn(),
  createRole: vi.fn(),
  createWorkflow: vi.fn(),
  deleteRole: vi.fn(),
  fetchAgentPerformance: vi.fn().mockRejectedValue(new Error("no history")),
  fetchAgentSessions: vi.fn(),
  fetchAgentTools: vi.fn().mockResolvedValue([]),
  fetchConfig: vi.fn(),
  fetchGraph: vi.fn(),
  fetchGraphWorkspace: vi.fn(),
  fetchRoles: vi.fn(),
  fetchRoleTools: vi.fn().mockResolvedValue([]),
  fetchRecipes: vi.fn(),
  fetchRunArtifacts: vi.fn().mockResolvedValue([]),
  fetchTaskArtifacts: vi.fn().mockResolvedValue([]),
  removeRoleAssignment: vi.fn(),
  retryTask: vi.fn(),
  saveConfig: vi.fn(),
  saveGraphWorkspace: vi.fn(),
  setPreferredAssignment: vi.fn(),
  startWorkflow: vi.fn(),
  updateWorkflowTeam: vi.fn(),
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
          .filter((edge) => edge.kind === "custom" || edge.kind === "binds")
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
      meta: { command: "codex exec", available: true, roles: ["planner", "worker"] },
    },
    {
      id: "agent:opencode",
      kind: "agent",
      label: "OpenCode",
      meta: { command: "opencode run", available: true, roles: ["worker"] },
    },
    {
      id: "role:worker",
      kind: "role",
      label: "Worker",
      meta: {
        id: "worker",
        name: "Worker",
        kind: "core",
        description: "Implements a planned task in an isolated worktree.",
        instructions: "",
        executionClass: "execution",
        assignments: [{ agent: "opencode", preferred: true }],
        available: true,
      },
    },
  ],
  edges: [
    {
      id: "assignment:worker:opencode",
      source: "role:worker",
      target: "agent:opencode",
      kind: "binds",
      editable: true,
      semantic: "configuration",
    },
  ],
  metadata: { runs: 0, tasks: 0, agents: 2, missingAgents: 0, roles: 1 },
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

const roles: RoleInfo[] = [
  {
    id: "planner",
    name: "Planner",
    kind: "core",
    description: "",
    instructions: "",
    executionClass: "planning",
    assignments: [{ agent: "codex", preferred: true }],
    available: true,
  },
  {
    id: "worker",
    name: "Worker",
    kind: "core",
    description: "Implements a planned task in an isolated worktree.",
    instructions: "",
    executionClass: "execution",
    assignments: [{ agent: "opencode", preferred: true }],
    available: true,
  },
  {
    id: "reviewer",
    name: "Reviewer",
    kind: "core",
    description: "",
    instructions: "",
    executionClass: "review",
    assignments: [{ agent: "codex", preferred: true }],
    available: true,
  },
];

const customRole: RoleInfo = {
  id: "database_engineer",
  name: "Database Engineer",
  kind: "custom",
  description: "Designs schema migrations.",
  instructions: "Keep the schema consistent.",
  executionClass: "advisory",
  assignments: [{ agent: "codex", preferred: true }],
  available: true,
};

const graphWithCustomRole: GraphData = {
  ...graph,
  nodes: [
    ...graph.nodes,
    {
      id: "role:database_engineer",
      kind: "role",
      label: "Database Engineer",
      meta: customRole,
    },
  ],
  metadata: { ...graph.metadata, roles: 2 },
};

beforeEach(() => {
  vi.mocked(fetchGraph).mockReset().mockResolvedValue(graph);
  vi.mocked(fetchGraphWorkspace).mockReset().mockResolvedValue(workspace);
  vi.mocked(fetchRoles).mockReset().mockResolvedValue(roles);
  vi.mocked(fetchRecipes).mockReset().mockResolvedValue([]);
  vi.mocked(fetchConfig)
    .mockReset()
    .mockResolvedValue({
      agents: {
        codex: {
          kind: "custom",
          command: "codex",
          args: ["run", "--model", "openai/gpt-5", "{mission}", "--label hello world"],
          env: { FACTORY_TEST: "1" },
          prompt_transport: "argument",
          interactive_args: ["--interactive"],
          capabilities: ["planning"],
          max_concurrency: 4,
        },
        opencode: { command: "opencode", args: ["run"], env: {} },
      },
      roles: {},
      role_assignments: [{ role: "planner", agent: "codex", preferred: true }],
    });
  vi.mocked(fetchAgentSessions).mockReset().mockResolvedValue([]);
  vi.mocked(saveConfig).mockReset().mockResolvedValue();
  vi.mocked(saveGraphWorkspace).mockReset().mockResolvedValue();
  vi.mocked(deleteRole).mockReset().mockResolvedValue();
  vi.mocked(addRoleAssignment).mockReset().mockResolvedValue(roles[1]);
  vi.mocked(removeRoleAssignment).mockReset().mockResolvedValue(roles[1]);
  vi.mocked(createWorkflow).mockReset().mockResolvedValue({
    id: 12,
    objective: "Implement authentication",
    status: "planning",
    plannerAgent: "codex",
    team: null,
    createdAt: "2026-08-13T18:00:00Z",
    updatedAt: "2026-08-13T18:00:00Z",
  });
  vi.spyOn(window, "confirm").mockReturnValue(true);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("Agent Graph interactions", () => {
  it("opens the Agent Console when an agent is selected", async () => {
    render(<NetworkView />);
    fireEvent.click(await screen.findByRole("button", { name: "select agent:codex" }));

    expect(await screen.findByLabelText("codex Agent Console")).toBeTruthy();
    expect(screen.getByText("No active interactive session.")).toBeTruthy();
  });

  it("opens the existing agent editor and saves without duplicating the agent", async () => {
    render(<NetworkView />);
    fireEvent.click(await screen.findByRole("button", { name: "select agent:codex" }));
    fireEvent.click(await screen.findByRole("button", { name: "Edit" }));

    expect(screen.getByRole("region", { name: "Agent configuration form" })).toBeTruthy();
    expect((screen.getByLabelText("Argument 4") as HTMLInputElement).value).toBe("{mission}");
    expect((screen.getByLabelText("Argument 5") as HTMLInputElement).value).toBe(
      "--label hello world"
    );
    expect(
      (screen.getByLabelText("Environment (KEY=VALUE, one per line)") as HTMLTextAreaElement).value
    ).toBe("FACTORY_TEST=1");
    expect((screen.getByLabelText("Max concurrency (optional)") as HTMLInputElement).value).toBe(
      "4"
    );
    expect(
      (screen.getByLabelText("Capabilities (one per line)") as HTMLTextAreaElement).value
    ).toBe("planning");

    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "renamed-codex" } });
    fireEvent.change(screen.getByLabelText("Capabilities (one per line)"), {
      target: { value: "planning\nreview" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(saveConfig).toHaveBeenCalledWith(
        expect.objectContaining({
          agents: expect.objectContaining({
            "renamed-codex": expect.objectContaining({
              args: ["run", "--model", "openai/gpt-5", "{mission}", "--label hello world"],
              capabilities: ["planning", "review"],
              max_concurrency: 4,
            }),
          }),
          role_assignments: [{ role: "planner", agent: "renamed-codex", preferred: true }],
        })
      )
    );
    expect(vi.mocked(saveConfig).mock.calls[0][0].agents).not.toHaveProperty("codex");
    expect(Object.keys(vi.mocked(saveConfig).mock.calls[0][0].agents)).toHaveLength(2);
    expect(saveGraphWorkspace).toHaveBeenCalledWith(
      expect.objectContaining({
        nodes: expect.objectContaining({ "agent:renamed-codex": { x: 418, y: 216 } }),
        edges: [expect.objectContaining({ source: "agent:renamed-codex" })],
      })
    );
    await waitFor(() => expect(vi.mocked(fetchGraph).mock.calls.length).toBeGreaterThan(1));
    expect(screen.queryByRole("region", { name: "Agent configuration form" })).toBeNull();
  });

  it("cancels an existing agent edit without saving", async () => {
    render(<NetworkView />);
    fireEvent.click(await screen.findByRole("button", { name: "select agent:codex" }));
    fireEvent.click(await screen.findByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "discarded-name" } });
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(saveConfig).not.toHaveBeenCalled();
    expect(screen.queryByRole("region", { name: "Agent configuration form" })).toBeNull();
    expect(await screen.findByLabelText("codex Agent Console")).toBeTruthy();
  });

  it("rejects a duplicate rename and leaves the editor open", async () => {
    render(<NetworkView />);
    fireEvent.click(await screen.findByRole("button", { name: "select agent:codex" }));
    fireEvent.click(await screen.findByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "opencode" } });
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(
        within(screen.getByRole("region", { name: "Agent configuration form" })).getByRole("alert")
          .textContent
      ).toContain("already exists")
    );
    expect(saveConfig).not.toHaveBeenCalled();
    expect(screen.getByRole("region", { name: "Agent configuration form" })).toBeTruthy();
  });

  it("opens the compact node creation menu from the toolbar", async () => {
    render(<NetworkView />);
    fireEvent.click(await screen.findByRole("button", { name: "Add graph node" }));

    expect(screen.getByRole("dialog", { name: "Add graph node" })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Workflow/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Agent/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Role/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Group/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Note/ })).toBeTruthy();
  });

  it("creates a workflow with the default team and persists its viewport-centered position", async () => {
    render(<NetworkView />);
    fireEvent.click(await screen.findByRole("button", { name: "Add graph node" }));
    fireEvent.click(screen.getByRole("button", { name: /Workflow/ }));
    fireEvent.change(screen.getByLabelText("What should the Factory build?"), {
      target: { value: "Implement authentication" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Plan" }));

    await waitFor(() =>
      expect(createWorkflow).toHaveBeenCalledWith("Implement authentication", {
        planner: "codex",
        workers: ["opencode"],
        reviewers: ["codex"],
        additional: {},
      })
    );
    expect(saveGraphWorkspace).toHaveBeenCalledWith(
      expect.objectContaining({
        nodes: expect.objectContaining({ "run:12": expect.any(Object) }),
      })
    );
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

  it("removes a binds edge through the role assignment API", async () => {
    render(<NetworkView />);
    fireEvent.click(
      await screen.findByRole("button", { name: "select assignment:worker:opencode" })
    );
    fireEvent.click(screen.getByRole("button", { name: "Delete connection" }));

    await waitFor(() => expect(removeRoleAssignment).toHaveBeenCalledWith("worker", "opencode"));
    await waitFor(() => expect(vi.mocked(fetchGraph).mock.calls.length).toBeGreaterThan(1));
    expect(saveConfig).not.toHaveBeenCalled();
  });

  it("opens the Role inspector for a selected role node", async () => {
    render(<NetworkView />);
    fireEvent.click(await screen.findByRole("button", { name: "select role:worker" }));

    expect(await screen.findByText("Core role")).toBeTruthy();
    expect(screen.getByText("Implements a planned task in an isolated worktree.")).toBeTruthy();
    expect(screen.getByText("opencode")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Edit" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Delete role" })).toBeNull();

    fireEvent.keyDown(window, { key: "Delete" });
    expect((await screen.findByRole("alert")).textContent).toContain(
      "Built-in core roles cannot be deleted; remove their assignments instead."
    );
    expect(deleteRole).not.toHaveBeenCalled();
  });

  it("deletes an unused custom role and refreshes graph, roles, and config", async () => {
    vi.mocked(fetchGraph)
      .mockReset()
      .mockResolvedValueOnce(graphWithCustomRole)
      .mockResolvedValue(graph);
    vi.mocked(fetchRoles)
      .mockReset()
      .mockResolvedValueOnce([...roles, customRole])
      .mockResolvedValue(roles);

    render(<NetworkView />);
    fireEvent.click(await screen.findByRole("button", { name: "select role:database_engineer" }));
    fireEvent.click(screen.getByRole("button", { name: "Delete role" }));

    await waitFor(() => expect(deleteRole).toHaveBeenCalledWith("database_engineer"));
    await waitFor(() => expect(fetchGraph).toHaveBeenCalledTimes(2));
    expect(fetchRoles).toHaveBeenCalledTimes(2);
    expect(fetchConfig).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole("button", { name: "select role:database_engineer" })).toBeNull();
  });

  it("cancels custom role deletion before calling the API", async () => {
    vi.mocked(fetchGraph).mockReset().mockResolvedValue(graphWithCustomRole);
    vi.mocked(fetchRoles)
      .mockReset()
      .mockResolvedValue([...roles, customRole]);
    vi.mocked(window.confirm).mockReturnValue(false);

    render(<NetworkView />);
    fireEvent.click(await screen.findByRole("button", { name: "select role:database_engineer" }));
    fireEvent.click(screen.getByRole("button", { name: "Delete role" }));

    expect(window.confirm).toHaveBeenCalledWith(
      "Delete the Database Engineer custom role and its assignments?"
    );
    expect(deleteRole).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Delete role" })).toBeTruthy();
  });

  it("shows the exact API blocking reason when custom role deletion is rejected", async () => {
    vi.mocked(fetchGraph).mockReset().mockResolvedValue(graphWithCustomRole);
    vi.mocked(fetchRoles)
      .mockReset()
      .mockResolvedValue([...roles, customRole]);
    vi.mocked(deleteRole).mockRejectedValue(
      new Error(
        "Factory API request failed (HTTP 409): role 'database_engineer' is used by an active workflow and cannot be deleted"
      )
    );

    render(<NetworkView />);
    fireEvent.click(await screen.findByRole("button", { name: "select role:database_engineer" }));
    fireEvent.click(screen.getByRole("button", { name: "Delete role" }));

    expect((await screen.findByRole("alert")).textContent).toContain(
      "role 'database_engineer' is used by an active workflow and cannot be deleted"
    );
    expect(screen.getByRole("button", { name: "Delete role" })).toBeTruthy();
  });

  it("hides role nodes and binds edges when the Roles toggle is off", async () => {
    render(<NetworkView />);
    fireEvent.click(await screen.findByRole("button", { name: "select role:worker" }));
    fireEvent.click(screen.getByLabelText("Roles"));

    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "select role:worker" })).toBeNull()
    );
    expect(screen.queryByRole("button", { name: "select assignment:worker:opencode" })).toBeNull();
  });
});
