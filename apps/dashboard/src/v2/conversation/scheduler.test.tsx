import { cleanup, render, screen, fireEvent } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { MessageRow, MissionProgress } from "./ConversationPanel";
import type { Mission } from "./types";
import type { WorkspaceView } from "../workspace/production/model";
import type { DockSession } from "../workspace/production/controller";

afterEach(cleanup);
const mission: Mission = {
  run_id: 1,
  objective: "Coordinate independent changes",
  status: "active",
  completed: 1,
  total: 4,
  terminal: false,
  tasks: [
    { id: 1, title: "Integrated prerequisite", state: "completed", dependencies: [] },
    { id: 2, title: "Validation", state: "running", dependencies: [1] },
    { id: 3, title: "Logging", state: "running", dependencies: [1] },
    { id: 4, title: "Combined tests", state: "pending", dependencies: [2, 3] },
  ],
  active_tasks: [
    { id: 2, title: "Validation", stage: "running" },
    { id: 3, title: "Logging", stage: "reviewing" },
  ],
  plan_revision: 2,
  plan_revisions: [
    {
      revision: 1,
      source: "planner",
      reason: null,
      plannerSessionId: 1,
      createdAt: "2026-10-30",
      tasks: [{ id: 1, title: "Integrated prerequisite", state: "ready", dependencies: [] }],
    },
    {
      revision: 2,
      source: "replan",
      reason: "Combined candidate failed a real check",
      plannerSessionId: 8,
      createdAt: "2026-10-30",
      tasks: [
        { id: 1, title: "Integrated prerequisite", state: "completed", dependencies: [] },
        { id: 5, title: "Repair integration", state: "ready", dependencies: [1] },
      ],
    },
  ],
  integrations: [
    {
      id: 1,
      taskId: 1,
      attemptId: 1,
      status: "published",
      beforeSha: "abc",
      sourceSha: "def",
      candidateSha: "def",
    },
    {
      id: 2,
      taskId: 2,
      attemptId: 2,
      status: "conflict",
      beforeSha: "def",
      sourceSha: "ghi",
      candidateSha: null,
    },
  ],
  sessions: [],
  attempts: [],
  changed_files: [],
  integration_sha: "def",
  artifacts: [],
};
const view = { projectId: "p_project_A" } as WorkspaceView;
it("presents a recorded suspension without relabelling the underlying run outcome", () => {
  const recorded: Mission = {
    ...mission,
    lifecycle: {
      schemaVersion: 1,
      projectId: "p_project_A",
      workspaceSessionId: "s_session",
      runId: 1,
      generation: 2,
      state: "suspended",
      reason: "last_browser_client_expired",
      checkpointId: 1,
      problem: null,
      updatedAt: "2026-11-02",
    },
  };
  render(
    <MessageRow
      message={{
        id: 1,
        conversation_id: "c",
        role: "orchestrator",
        content: "Owned execution stopped; partial work is preserved.",
        status: "finished",
        run_id: 1,
        parent_message_id: null,
        provenance: { source: "runtime", snapshot: recorded },
        created_at: "2026-11-02",
      }}
      view={view}
      dock={{ open: vi.fn() } as unknown as DockSession}
    />
  );
  expect(screen.getByText("suspended")).toBeTruthy();
  expect(screen.queryByText("active")).toBeNull();
  expect(recorded.status).toBe("active");
});
it("renders actual concurrent stages and a separate serialized publication history", () => {
  render(
    <MissionProgress
      mission={mission}
      view={view}
      dock={{ open: vi.fn() } as unknown as DockSession}
    />
  );
  expect(screen.getByText("1 tasks executing · 1 awaiting review outcome")).toBeTruthy();
  expect(screen.getByText("Plan revision 2")).toBeTruthy();
  expect(screen.getByLabelText("Integration history").textContent).toContain("Task #2 · conflict");
  expect(screen.getByLabelText("Integration history").textContent).toContain("1 published");
  expect(screen.getByLabelText("Plan revision history").textContent).toContain(
    "Combined candidate failed a real check"
  );
  expect(screen.getByLabelText("Plan revision history").textContent).toContain(
    "Repair integration"
  );
});
it("retains authoritative project ownership when navigating replan and workflow details", () => {
  const open = vi.fn();
  render(
    <MissionProgress mission={mission} view={view} dock={{ open } as unknown as DockSession} />
  );
  fireEvent.click(screen.getByRole("button", { name: "Planner session #8" }));
  expect(open).toHaveBeenCalledWith("console", { projectId: "p_project_A", resourceId: 8 });
  fireEvent.click(screen.getByRole("button", { name: "Open workflow and review" }));
  expect(open).toHaveBeenCalledWith("workflow", { projectId: "p_project_A", resourceId: 1 });
});
