import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fetchRunArtifacts } from "../api";
import type { GraphEdge, GraphNode } from "../types";
import { NodeInspector } from "./NodeInspector";

vi.mock("../api", () => ({
  fetchRunArtifacts: vi.fn(),
  fetchTaskArtifacts: vi.fn(),
}));

const roleNode: GraphNode = {
  id: "role:security_auditor",
  kind: "role",
  label: "Security Auditor",
  meta: {
    id: "security_auditor",
    name: "Security Auditor",
    kind: "core",
    description: "",
    instructions: "",
    executionClass: "review",
    assignments: [{ agent: "claude", preferred: true }],
    available: true,
  },
};

const taskNode: GraphNode = {
  id: "task:9",
  kind: "task",
  label: "Audit authentication",
  meta: {
    taskId: 9,
    runId: 2,
    objective: "review the auth change",
    state: "running",
    position: 1,
    dependencies: [8],
    acceptanceCriteria: ["decision"],
    worktreePath: null,
    role: "security_auditor",
    operation: "review",
    currentAttempt: {
      id: 31,
      taskId: 9,
      attemptNumber: 2,
      agent: "claude",
      role: "security_auditor",
      operation: "review",
      status: "changes_requested",
      startedAt: "2026-08-13T18:00:00Z",
      finishedAt: "2026-08-13T18:02:00Z",
      worktreePath: "worktree",
      commitSha: null,
      exitCode: 0,
      error: null,
      evidence: {
        changedFiles: [],
        diffSummary: "",
        commitSha: null,
        commands: [],
        acceptanceCriteria: [],
        workerExitCode: 0,
        artifacts: [5],
      },
      review: {
        decision: "request_changes",
        reason: "token appears in query string",
        feedback: ["[high] token appears in query string"],
      },
    },
  },
};

beforeEach(() => {
  vi.mocked(fetchRunArtifacts).mockReset();
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("Node Inspector (role-aware task)", () => {
  it("shows role, operation, execution class, agent and review state", () => {
    vi.mocked(fetchRunArtifacts).mockResolvedValue([]);
    const nodesById = new Map<string, GraphNode>([
      [roleNode.id, roleNode],
      [taskNode.id, taskNode],
    ]);
    render(
      <NodeInspector
        node={taskNode}
        edge={null as GraphEdge | null}
        nodesById={nodesById}
        onClose={vi.fn()}
        onDelete={vi.fn()}
        onConnect={vi.fn()}
      />
    );

    expect(screen.getAllByText("review").length).toBeGreaterThan(0);
    expect(screen.getByText("Execution class")).toBeTruthy();
    expect(screen.getByText("security_auditor")).toBeTruthy();
    expect(screen.getAllByText("claude").length).toBeGreaterThan(0);
    expect(screen.getByText("#8")).toBeTruthy(); // dependency
    expect(screen.getByText(/token appears in query string/)).toBeTruthy();
    expect(screen.getByText("none (no repository changes required)")).toBeTruthy();
  });

  it("lists produced and consumed artifacts from the run artifact set", async () => {
    vi.mocked(fetchRunArtifacts).mockResolvedValue([
      {
        id: 5,
        runId: 2,
        taskId: 9,
        attemptId: 31,
        role: "security_auditor",
        operation: "review",
        kind: "review",
        content: '{"decision":"request_changes","findings":[]}',
        createdAt: "2026-08-13T18:02:00Z",
      },
      {
        id: 4,
        runId: 2,
        taskId: 8,
        attemptId: 30,
        role: "researcher",
        operation: "advisory",
        kind: "research",
        content: '{"summary":"uses JWT"}',
        createdAt: "2026-08-13T18:01:00Z",
      },
    ]);
    const nodesById = new Map<string, GraphNode>([[taskNode.id, taskNode]]);
    render(
      <NodeInspector
        node={taskNode}
        edge={null as GraphEdge | null}
        nodesById={nodesById}
        onClose={vi.fn()}
        onDelete={vi.fn()}
        onConnect={vi.fn()}
      />
    );

    expect(screen.getByText("Produced artifacts")).toBeTruthy();
    expect(await screen.findByText(/Specialized review/)).toBeTruthy();
    expect(screen.getAllByText("Consumed artifacts (dependencies)").length).toBe(1);
    expect(screen.getByText(/Research findings/)).toBeTruthy();
    fireEvent.click(screen.getAllByText("Inspect content")[0]);
    expect(screen.getByText(/uses JWT/)).toBeTruthy();
  });
});
