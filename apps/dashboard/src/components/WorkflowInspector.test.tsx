import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fetchRun } from "../api";
import type { GraphNode, RunDetail } from "../types";
import { WorkflowInspector } from "./WorkflowInspector";

vi.mock("../api", () => ({ fetchRun: vi.fn() }));

const node: GraphNode = {
  id: "run:12",
  kind: "run",
  label: "Authentication workflow",
  meta: {
    runId: 12,
    objective: "Implement email authentication",
    status: "planned",
    plannerAgent: "codex",
    workerAgent: "opencode",
    reviewerAgent: "claude",
    createdAt: "2026-08-13T18:00:00Z",
    counts: { pending: 0, ready: 1, running: 0, blocked: 0, failed: 0, completed: 0, total: 1 },
  },
};

const detail: RunDetail = {
  run: {
    id: 12,
    objective: "Implement email authentication",
    status: "planned",
    plannerAgent: "codex",
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
      createdAt: "2026-08-13T18:01:00Z",
      updatedAt: "2026-08-13T18:01:00Z",
    },
  ],
  attempts: [],
  sessions: [],
};

beforeEach(() => {
  vi.mocked(fetchRun).mockReset().mockResolvedValue(detail);
  vi.spyOn(window, "confirm").mockReturnValue(true);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("Workflow inspector", () => {
  it("shows the persisted plan and starts it after confirmation", async () => {
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
    fireEvent.click(screen.getByRole("tab", { name: "tasks" }));
    expect(screen.getByText("#41 Login API")).toBeTruthy();
    expect(screen.getByText("Invalid credentials are rejected")).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: "overview" }));
    fireEvent.click(screen.getByRole("button", { name: "Start" }));

    await waitFor(() => expect(onStart).toHaveBeenCalledWith(12));
    expect(window.confirm).toHaveBeenCalledWith(expect.stringContaining("isolated worktrees"));
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
