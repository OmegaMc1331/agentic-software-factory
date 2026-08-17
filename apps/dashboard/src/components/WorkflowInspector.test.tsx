import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fetchRoles, fetchRun, updateWorkflowTeam } from "../api";
import type { GraphNode, RoleInfo, RunDetail, RunMeta } from "../types";
import { WorkflowInspector } from "./WorkflowInspector";

vi.mock("../api", () => ({
  fetchRoles: vi.fn(),
  fetchRun: vi.fn(),
  updateWorkflowTeam: vi.fn(),
}));

const team = {
  planner: "codex",
  workers: ["opencode", "qwen"],
  reviewers: ["claude"],
  additional: { security_auditor: ["claude"] },
};

const meta: RunMeta = {
  runId: 12,
  objective: "Implement email authentication",
  status: "planned",
  plannerAgent: "codex",
  team,
  createdAt: "2026-08-13T18:00:00Z",
  counts: { pending: 0, ready: 1, running: 0, blocked: 0, failed: 0, completed: 0, total: 1 },
};

const node: GraphNode = {
  id: "run:12",
  kind: "run",
  label: "Authentication workflow",
  meta,
};

const detail: RunDetail = {
  run: {
    id: 12,
    objective: "Implement email authentication",
    status: "planned",
    plannerAgent: "codex",
    team,
    createdAt: "2026-08-13T18:00:00Z",
    updatedAt: "2026-08-13T18:01:00Z",
  },
  tasks: [
    {
      id: 41,
      runId: 12,
      title: "Login API",
      objective: "Add login endpoint",
      acceptanceCriteria: ["Invalid credentials are rejected"],
      state: "ready",
      position: 0,
      dependencies: [],
      worktreePath: null,
      role: null,
      operation: "implement",
      createdAt: "2026-08-13T18:01:00Z",
      updatedAt: "2026-08-13T18:01:00Z",
    },
  ],
  attempts: [],
  sessions: [],
  stages: [
    {
      key: "implementation",
      label: "Implementation",
      total: 1,
      completed: 0,
      state: "active",
    },
  ],
  artifacts: [],
};

const roles: RoleInfo[] = [
  {
    id: "planner",
    name: "Planner",
    kind: "core",
    description: "",
    instructions: "",
    executionClass: "planning",
    assignments: [
      { agent: "codex", preferred: true },
      { agent: "gemini", preferred: false },
    ],
    available: true,
  },
  {
    id: "worker",
    name: "Worker",
    kind: "core",
    description: "",
    instructions: "",
    executionClass: "execution",
    assignments: [
      { agent: "opencode", preferred: true },
      { agent: "qwen", preferred: false },
    ],
    available: true,
  },
  {
    id: "reviewer",
    name: "Reviewer",
    kind: "core",
    description: "",
    instructions: "",
    executionClass: "review",
    assignments: [{ agent: "claude", preferred: true }],
    available: true,
  },
];

beforeEach(() => {
  vi.mocked(fetchRun).mockReset().mockResolvedValue(detail);
  vi.mocked(fetchRoles).mockReset().mockResolvedValue(roles);
  vi.mocked(updateWorkflowTeam).mockReset().mockResolvedValue(team);
  vi.spyOn(window, "confirm").mockReturnValue(true);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("Workflow inspector", () => {
  it("shows derived stages and an artifact inspector from the run detail", async () => {
    vi.mocked(fetchRun).mockResolvedValue({
      ...detail,
      stages: [
        { key: "analysis", label: "Analysis", total: 1, completed: 0, state: "active" },
        {
          key: "implementation",
          label: "Implementation",
          total: 1,
          completed: 1,
          state: "completed",
        },
      ],
      artifacts: [
        {
          id: 3,
          runId: 12,
          taskId: 40,
          attemptId: 9,
          role: "researcher",
          operation: "advisory",
          kind: "research",
          content: '{"summary":"tokens in httpOnly cookies","findings":[]}',
          createdAt: "2026-08-13T18:00:00Z",
        },
      ],
    });
    render(
      <WorkflowInspector
        node={node}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );

    expect(await screen.findByText("Analysis")).toBeTruthy();
    expect(screen.getByText("Implementation")).toBeTruthy();
    expect(screen.getByText("✓")).toBeTruthy();
    expect(screen.getByText("1/1")).toBeTruthy();

    fireEvent.click(screen.getByText("Role artifacts (1)"));
    expect(screen.getByText(/Research findings/)).toBeTruthy();
    fireEvent.click(screen.getByText("Inspect content"));
    expect(screen.getByText(/tokens in httpOnly cookies/)).toBeTruthy();
  });

  it("stays simple: workflows without stages show no empty stage list", async () => {
    vi.mocked(fetchRun).mockResolvedValue({ ...detail, stages: [], artifacts: [] });
    render(
      <WorkflowInspector
        node={node}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );
    await screen.findByText("0 / 1 tasks");
    expect(screen.queryByText("Stages")).toBeNull();
  });

  it("labels every task with its role operation in the tasks tab", async () => {
    render(
      <WorkflowInspector
        node={node}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );
    fireEvent.click(await screen.findByRole("tab", { name: "tasks" }));
    expect(screen.getByText("implement")).toBeTruthy();
    expect(screen.getByText("worker")).toBeTruthy();
  });

  it("shows the persisted plan with its team and starts it after confirmation", async () => {
    const onStart = vi.fn().mockResolvedValue(undefined);
    render(
      <WorkflowInspector
        node={node}
        onClose={vi.fn()}
        onStart={onStart}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );

    expect(await screen.findByText("0 / 1 tasks")).toBeTruthy();
    expect(screen.getByText("codex")).toBeTruthy();
    expect(screen.getByText("opencode, qwen")).toBeTruthy();
    expect(screen.getByText("security_auditor")).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: "tasks" }));
    expect(screen.getByText("#41 Login API")).toBeTruthy();
    expect(screen.getByText("Invalid credentials are rejected")).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: "overview" }));
    fireEvent.click(screen.getByRole("button", { name: "Start" }));

    await waitFor(() => expect(onStart).toHaveBeenCalledWith(12));
    expect(window.confirm).toHaveBeenCalledWith(expect.stringContaining("Workers: opencode, qwen"));
    expect(window.confirm).toHaveBeenCalledWith(expect.stringContaining("isolated worktrees"));
  });

  it("edits the team while the workflow is still planned", async () => {
    const onTeamUpdated = vi.fn();
    render(
      <WorkflowInspector
        node={node}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
        onTeamUpdated={onTeamUpdated}
      />
    );

    fireEvent.click(await screen.findByRole("button", { name: "Edit team" }));
    await waitFor(() => expect(fetchRoles).toHaveBeenCalled());
    fireEvent.click(screen.getByLabelText("qwen"));
    fireEvent.click(screen.getByRole("button", { name: "Save team" }));

    await waitFor(() =>
      expect(updateWorkflowTeam).toHaveBeenCalledWith(12, {
        planner: "codex",
        workers: ["opencode"],
        reviewers: ["claude"],
        additional: { security_auditor: ["claude"] },
      })
    );
    await waitFor(() => expect(onTeamUpdated).toHaveBeenCalled());
  });

  it("locks the team once the workflow is active", async () => {
    vi.mocked(fetchRun).mockResolvedValue({
      ...detail,
      run: { ...detail.run, status: "active" },
    });
    const activeNode: GraphNode = {
      ...node,
      meta: { ...meta, status: "active" },
    };
    render(
      <WorkflowInspector
        node={activeNode}
        onClose={vi.fn()}
        onStart={vi.fn()}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );

    expect(await screen.findByText("0 / 1 tasks")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Edit team" })).toBeNull();
    expect(screen.getByText("The team is locked while the workflow is active.")).toBeTruthy();
  });

  it("shows operation errors inline", async () => {
    const onStart = vi.fn().mockRejectedValue(new Error("No Worker agent configured."));
    render(
      <WorkflowInspector
        node={node}
        onClose={vi.fn()}
        onStart={onStart}
        onCancel={vi.fn()}
        onRetry={vi.fn()}
      />
    );

    await screen.findByText("0 / 1 tasks");
    fireEvent.click(screen.getByRole("button", { name: "Start" }));
    expect((await screen.findByRole("alert")).textContent).toContain("No Worker agent configured.");
  });
});
