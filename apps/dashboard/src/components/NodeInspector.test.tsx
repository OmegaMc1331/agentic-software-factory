import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  fetchRoles,
  fetchRoutingDecisions,
  fetchRoutingPreview,
  fetchRun,
  fetchRunArtifacts,
} from "../api";
import type { GraphEdge, GraphNode } from "../types";
import { NodeInspector } from "./NodeInspector";

vi.mock("../api", () => ({
  fetchRunArtifacts: vi.fn(),
  fetchTaskArtifacts: vi.fn(),
  fetchRun: vi.fn(),
  fetchRoutingPreview: vi.fn(),
  fetchRoutingDecisions: vi.fn(),
  fetchRoles: vi.fn(),
  setTaskRouting: vi.fn(),
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
  vi.mocked(fetchRoutingPreview)
    .mockReset()
    .mockRejectedValue(new Error("routing preview unavailable"));
  vi.mocked(fetchRoutingDecisions).mockReset().mockResolvedValue([]);
  vi.mocked(fetchRoles).mockReset().mockResolvedValue([]);
  vi.mocked(fetchRun)
    .mockReset()
    .mockResolvedValue({
      run: {
        id: 9,
        objective: "",
        status: "active",
        plannerAgent: null,
        team: null,
        createdAt: "",
        updatedAt: "",
      },
      tasks: [],
      attempts: [],
      sessions: [],
      stages: [],
      artifacts: [],
      integration: { branch: "factory/run-9", head: null, integratedTasks: [] },
    });
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

  it("shows the routing preview and manual agent override for ready tasks", async () => {
    vi.mocked(fetchRunArtifacts).mockResolvedValue([]);
    vi.mocked(fetchRoles).mockResolvedValue([
      {
        id: "security_auditor",
        name: "Security Auditor",
        kind: "core",
        description: "",
        instructions: "",
        executionClass: "review",
        assignments: [{ agent: "claude", preferred: true }],
        available: true,
      },
    ]);
    vi.mocked(fetchRoutingPreview).mockResolvedValue({
      mode: "performance",
      taskId: 9,
      role: "security_auditor",
      operation: "review",
      language: null,
      overrideAgent: null,
      likelyAgent: "claude",
      reason: "Highest reliable routing score with available capacity.",
      candidates: [
        { agent: "claude", score: 0.89, reliable: true, note: "role+operation slice, n=12" },
        { agent: "codex", score: null, reliable: false, note: "insufficient data (n=3 of 10)" },
      ],
    });
    // The override selector only appears while the task has not started.
    const readyTask: GraphNode = {
      ...taskNode,
      meta: { ...taskNode.meta, state: "ready" },
    };
    const nodesById = new Map<string, GraphNode>([[readyTask.id, readyTask]]);
    render(
      <NodeInspector
        node={readyTask}
        edge={null as GraphEdge | null}
        nodesById={nodesById}
        onClose={vi.fn()}
        onDelete={vi.fn()}
        onConnect={vi.fn()}
      />
    );

    expect(await screen.findByText("Routing")).toBeTruthy();
    expect(await screen.findByText("performance")).toBeTruthy();
    expect(await screen.findByText(/Highest reliable routing score/)).toBeTruthy();
    // The override selector offers Automatic plus the role's assigned agents.
    const override = await screen.findByLabelText("Manual agent override");
    expect(override).toBeTruthy();
    const options = Array.from(override.querySelectorAll("option")).map(
      (option) => option.textContent
    );
    expect(options).toEqual(["Automatic", "claude"]);
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

describe("Node Inspector (agent permissions)", () => {
  const agentNode: GraphNode = {
    id: "agent:claude",
    kind: "agent",
    label: "claude",
    meta: {
      command: "claude -p",
      available: true,
      roles: ["security_auditor"],
      permissions: {
        source: "agent:claude",
        permissive: false,
        filesystemMode: "read_only",
        readScopes: ["**"],
        writeScopes: [],
        denyWriteScopes: [".factory/**"],
        commandsMode: "restricted",
        commandsAllow: ["git"],
        commandsDeny: ["bash"],
        network: "deny",
        networkEnforcement: "advisory",
        environmentMode: "filtered",
        environmentAllowed: ["PATH"],
        environmentDenied: ["GITHUB_TOKEN"],
        gitAllowed: ["read"],
        gitDenied: ["push", "force_push", "delete_branch", "reset_branch", "modify_remotes"],
      },
    },
  };

  it("shows the agent's effective permissions including advisory network", () => {
    render(
      <NodeInspector
        node={agentNode}
        edge={null as GraphEdge | null}
        nodesById={new Map([[agentNode.id, agentNode]])}
        onClose={vi.fn()}
        onDelete={vi.fn()}
        onConnect={vi.fn()}
      />
    );

    expect(screen.getByText("Permissions")).toBeTruthy();
    expect(screen.getByText("Read-only")).toBeTruthy();
    expect(screen.getByText("Denied")).toBeTruthy();
    expect(screen.getByText(/advisory — not process-enforced/)).toBeTruthy();
    expect(screen.getByText("Policy source: agent:claude")).toBeTruthy();
  });

  it("omits nothing when the agent carries no policy information", () => {
    const bare = { ...agentNode, meta: { command: "claude -p", available: true, roles: [] } };
    render(
      <NodeInspector
        node={bare}
        edge={null as GraphEdge | null}
        nodesById={new Map([[bare.id, bare]])}
        onClose={vi.fn()}
        onDelete={vi.fn()}
        onConnect={vi.fn()}
      />
    );

    expect(screen.getByText("No policy information available.")).toBeTruthy();
  });
});
